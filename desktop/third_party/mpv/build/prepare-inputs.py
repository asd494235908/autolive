#!/usr/bin/env python3
from __future__ import annotations

import shutil
import subprocess
import tarfile
import os
import re
import hashlib
import json
from copy import copy
from pathlib import Path, PurePosixPath

INPUTS = Path('/build-inputs')
WORK = Path('/work')
RECIPE = Path('/recipe')
MAX_MEMBERS = 500_000
MAX_EXPANDED_BYTES = 24 * 1024 * 1024 * 1024

RECIPE_PREFIX = 'desktop/third_party/mpv/build/'


def extracted_members(archive: tarfile.TarFile, strip_top: bool):
    members = archive.getmembers()
    if len(members) > MAX_MEMBERS:
        raise RuntimeError(f'{archive.name}: archive member limit exceeded')
    if sum(member.size for member in members if member.isfile()) > MAX_EXPANDED_BYTES:
        raise RuntimeError(f'{archive.name}: expanded byte limit exceeded')
    top_levels = {
        PurePosixPath(member.name).parts[0]
        for member in members
        if PurePosixPath(member.name).parts and PurePosixPath(member.name).parts[0] != '.'
    }
    if strip_top and len(top_levels) != 1:
        raise RuntimeError(f'{archive.name}: expected one archive root, got {sorted(top_levels)}')
    for original in members:
        parts = list(PurePosixPath(original.name).parts)
        while parts and parts[0] == '.':
            parts.pop(0)
        if strip_top and parts:
            parts.pop(0)
        if not parts:
            continue
        if any(part in ('', '.', '..') for part in parts):
            raise RuntimeError(f'{archive.name}: unsafe path {original.name}')
        member = copy(original)
        member.name = PurePosixPath(*parts).as_posix()
        yield member


def extract(relative: str, destination: Path, *, strip_top: bool = True) -> None:
    archive_path = INPUTS / relative
    if not archive_path.is_file() or archive_path.is_symlink():
        raise RuntimeError(f'missing regular build input: {relative}')
    if destination.exists():
        raise RuntimeError(f'refusing to reuse extraction destination: {destination}')
    destination.mkdir(parents=True)
    with tarfile.open(archive_path, mode='r:*') as archive:
        archive.extractall(destination, members=extracted_members(archive, strip_top), filter='data')


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def verify_file(path: Path, expected_size: int, expected_sha256: str, label: str) -> None:
    if not path.is_file() or path.is_symlink():
        raise RuntimeError(f'missing regular file: {label}')
    if path.stat().st_size != expected_size or sha256(path) != expected_sha256:
        raise RuntimeError(f'locked bytes mismatch: {label}')


def copy_tree(source: Path, destination: Path) -> None:
    if not source.is_dir() or source.is_symlink():
        raise RuntimeError(f'invalid source tree copy: {source} -> {destination}')
    if destination.exists():
        if not destination.is_dir() or destination.is_symlink() or any(destination.iterdir()):
            raise RuntimeError(f'refusing to replace non-empty dependency directory: {destination}')
        destination.rmdir()
    shutil.copytree(source, destination, symlinks=False)


def add_lowercase_file_aliases(root: Path) -> None:
    for source in [path for path in root.rglob('*') if path.is_file() and not path.is_symlink()]:
        lowercase = source.with_name(source.name.lower())
        if lowercase == source:
            continue
        if os.path.lexists(lowercase):
            if not lowercase.samefile(source):
                raise RuntimeError(f'case-insensitive sysroot collision: {source} -> {lowercase}')
            continue
        lowercase.symlink_to(source.name)


def add_requested_header_aliases(scan_roots: tuple[Path, ...], include_roots: tuple[Path, ...]) -> None:
    headers: dict[str, list[Path]] = {}
    for root in include_roots:
        for path in root.rglob('*'):
            if path.is_file() and not path.is_symlink():
                headers.setdefault(path.name.casefold(), []).append(path)
    requested: set[str] = set()
    include_pattern = re.compile(rb'^\s*#\s*include\s*[<"]([^>"]+)[>"]', re.MULTILINE)
    text_suffixes = {'.c', '.cc', '.cpp', '.h', '.hh', '.hpp', '.inc', '.in', '.m', '.mm', '.rc'}
    for root in scan_roots:
        for path in root.rglob('*'):
            if path.is_file() and path.suffix.lower() in text_suffixes and path.stat().st_size <= 4 * 1024 * 1024:
                for match in include_pattern.finditer(path.read_bytes()):
                    token = PurePosixPath(match.group(1).decode('utf-8', errors='ignore'))
                    if token.parts and '..' not in token.parts:
                        requested.add(token.name)
    for name in sorted(requested):
        for source in headers.get(name.casefold(), ()):
            alias = source.with_name(name)
            if alias == source:
                continue
            if os.path.lexists(alias):
                if not alias.samefile(source):
                    raise RuntimeError(f'case-insensitive header collision: {source} -> {alias}')
                continue
            alias.symlink_to(source.name)


def main() -> None:
    if any((WORK / name).exists() for name in ('sources', 'toolchain', 'sysroot', 'native', 'prefix')):
        raise RuntimeError('/work must be a fresh container workspace')
    sources_root = WORK / 'sources'
    tools_root = WORK / 'toolchain'
    lock_path = RECIPE / 'reproducible-build-lock.json'
    lock = json.loads(lock_path.read_text(encoding='utf-8'))
    for source in lock['sources']:
        verify_file(INPUTS / source['cache_path'], source['size_bytes'], source['sha256'], source['cache_path'])
        extract(source['cache_path'], sources_root / source['name'])
    tool_names = {'meson': 'meson', 'nasm': 'nasm', 'pkg-config': 'pkgconf', 'llvm-rc': 'llvm-project'}
    for lock_name, destination_name in tool_names.items():
        tool = lock['toolchain']['tools'][lock_name]
        verify_file(INPUTS / tool['cache_path'], tool['size_bytes'], tool['sha256'], tool['cache_path'])
        extract(tool['cache_path'], tools_root / destination_name)
    for tool in lock['toolchain']['tools'].values():
        if tool['provision'] == 'cache':
            verify_file(INPUTS / tool['cache_path'], tool['size_bytes'], tool['sha256'], tool['cache_path'])
    for patch in sorted(lock['patches'], key=lambda item: item['order']):
        if not patch['path'].startswith(RECIPE_PREFIX):
            raise RuntimeError(f"patch path outside recipe: {patch['path']}")
        recipe_patch = RECIPE / patch['path'][len(RECIPE_PREFIX):]
        verify_file(recipe_patch, patch['size_bytes'], patch['sha256'], patch['path'])
        subprocess.run(
            [
                '/usr/bin/patch', '--batch', '--forward', '--fuzz=0', '-p1',
                f'--input={recipe_patch}',
            ],
            cwd=sources_root / patch['component'],
            check=True,
        )
    subprocess.run(['/bin/sh', '-n', 'configure'], cwd=sources_root / 'ffmpeg', check=True)
    extract('toolchain/windows-sdk-10.0.26100.0.tar.gz', WORK / 'sysroot/windows-sdk', strip_top=False)
    extract('toolchain/ucrt-10.0.26100.0.tar.gz', WORK / 'sysroot/ucrt', strip_top=False)
    extract('toolchain/msvc-toolset-14.44.35207.tar.gz', WORK / 'sysroot/msvc', strip_top=False)
    for sysroot in (WORK / 'sysroot/windows-sdk', WORK / 'sysroot/ucrt', WORK / 'sysroot/msvc'):
        add_lowercase_file_aliases(sysroot)

    shaderc_third_party = sources_root / 'shaderc/third_party'
    for source, destination in (
        ('glslang', 'glslang'),
        ('spirv-tools', 'spirv-tools'),
        ('spirv-headers', 'spirv-headers'),
    ):
        copy_tree(sources_root / source, shaderc_third_party / destination)
    placebo_third_party = sources_root / 'libplacebo/3rdparty'
    for source, destination in (
        ('vulkan-headers', 'Vulkan-Headers'),
        ('fast-float', 'fast_float'),
        ('jinja', 'jinja'),
        ('markupsafe', 'markupsafe'),
    ):
        copy_tree(sources_root / source, placebo_third_party / destination)

    include_roots = (
        WORK / 'sysroot/msvc/include',
        WORK / 'sysroot/ucrt/include',
        WORK / 'sysroot/windows-sdk/include/shared',
        WORK / 'sysroot/windows-sdk/include/um',
        WORK / 'sysroot/windows-sdk/include/winrt',
    )
    add_requested_header_aliases((sources_root, *include_roots), include_roots)

    for marker in (
        sources_root / 'mpv/meson.build',
        sources_root / 'ffmpeg/configure',
        sources_root / 'libplacebo/meson.build',
        sources_root / 'shaderc/DEPS',
        tools_root / 'llvm-project/llvm/CMakeLists.txt',
        WORK / 'sysroot/windows-sdk/include/um/Windows.h',
        WORK / 'sysroot/ucrt/include/corecrt.h',
        WORK / 'sysroot/msvc/include/vector',
    ):
        if not marker.is_file():
            raise RuntimeError(f'extracted input marker missing: {marker}')


if __name__ == '__main__':
    main()
