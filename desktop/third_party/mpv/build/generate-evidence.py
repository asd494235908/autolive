#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_INPUTS = Path('/build-inputs')
DEFAULT_WORK = Path('/work')
DEFAULT_MESON_ROOT = Path('/tmp/autolive-meson-build')
DEFAULT_RECIPE = Path('/recipe')
DEFAULT_OUT = Path('/out')
EPOCH = 1_787_875_200
CREATED = datetime.fromtimestamp(EPOCH, timezone.utc).isoformat().replace('+00:00', 'Z')
RECIPE_PREFIX = 'desktop/third_party/mpv/build/'
INTROSPECTION_KINDS = ('buildoptions', 'dependencies', 'targets', 'projectinfo', 'machines')
ARTIFACT_CONTRACT = (
    ('build-log', 'build-log.txt', 'text'),
    ('build-parameters', 'build-parameters.json', 'json'),
    ('corresponding-source-archive', 'corresponding-source.tar.zst', 'tar-zst'),
    ('copyright-inventory', 'copyright-inventory.txt', 'text'),
    ('cyclonedx-sbom', 'sbom.cdx.json', 'cyclonedx-json'),
    ('dependency-lock', 'dependency-lock.json', 'json'),
    ('docker-host-evidence', 'docker-host-evidence.json', 'json'),
    ('license-inventory', 'license-inventory.txt', 'text'),
    ('meson-introspection', 'meson-introspection.json', 'json'),
    ('mpv-executable', 'mpv.exe', 'pe'),
    ('patch-bundle', 'patch-bundle.tar.zst', 'tar-zst'),
    ('pe-imports', 'pe-imports.json', 'json'),
    ('spdx-sbom', 'sbom.spdx.json', 'spdx-json'),
    ('spirv-cross-runtime', 'spirv-cross-c-shared.dll', 'pe'),
    ('vulkan-loader-runtime', 'vulkan-1.dll', 'pe'),
)
RUNTIME_FILES = {
    'mpv-executable': 'mpv.exe',
    'spirv-cross-runtime': 'spirv-cross-c-shared.dll',
    'vulkan-loader-runtime': 'vulkan-1.dll',
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + '\n', encoding='utf-8')


def add_tree(archive: tarfile.TarFile, source: Path, arcname: str) -> None:
    def canonical(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = 'root'
        info.mtime = EPOCH
        return info

    archive.add(source, arcname=arcname, recursive=True, filter=canonical)


def tar_zstd(output: Path, trees: tuple[tuple[Path, str], ...], work: Path) -> None:
    with tempfile.TemporaryDirectory(dir=work) as temporary:
        tar_path = Path(temporary) / 'payload.tar'
        with tarfile.open(tar_path, mode='w', format=tarfile.PAX_FORMAT, dereference=True) as archive:
            for source, arcname in trees:
                add_tree(archive, source, arcname)
        subprocess.run(
            [str(work / 'native/bin/zstd'), '-19', '--threads=1', '--no-progress', '-f', str(tar_path), '-o', str(output)],
            check=True,
            env={**os.environ, 'TZ': 'UTC'},
        )


def source_version(source: dict) -> str:
    return source.get('commit') or source['sha256']


def source_bom_ref(source: dict) -> str:
    return f"source:{source['name']}@{source_version(source)}"


def tool_bom_ref(name: str, tool: dict) -> str:
    return f"tool:{name}@{tool['version']}"


def build_component_rows(lock: dict) -> list[dict]:
    rows = []
    for source in lock['sources']:
        rows.append({
            'kind': 'source', 'name': source['name'], 'version': source_version(source),
            'usage': source['usage'], 'license': source['license_expression'],
            'cachePath': source['cache_path'], 'sha256': source['sha256'],
            'runtimeArtifacts': source['runtime_artifacts'], 'bomRef': source_bom_ref(source),
        })
    for name, tool in sorted(lock['toolchain']['tools'].items()):
        rows.append({
            'kind': 'tool', 'name': name, 'version': tool['version'], 'usage': 'build',
            'license': tool['license_expression'], 'cachePath': tool['cache_path'], 'sha256': tool['sha256'],
            'runtimeArtifacts': [], 'bomRef': tool_bom_ref(name, tool),
        })
    return rows


def generate_content(inputs: Path, work: Path, meson_root: Path, recipe: Path, out: Path) -> None:
    artifacts = out / 'build-evidence'
    artifacts.mkdir(parents=True)
    lock_path = recipe / 'reproducible-build-lock.json'
    lock_bytes = lock_path.read_bytes()
    lock = json.loads(lock_bytes)

    runtime_paths = {
        'mpv-executable': work / 'mpv.exe',
        'spirv-cross-runtime': work / 'spirv-cross-c-shared.dll',
        'vulkan-loader-runtime': work / 'vulkan-1.dll',
    }
    for role, source in runtime_paths.items():
        if not source.is_file():
            raise RuntimeError(f'{role} missing after successful build')
        shutil.copy2(source, artifacts / RUNTIME_FILES[role])
    shutil.copy2(work / 'build.log', artifacts / 'build-log.txt')

    input_rows = []
    for relative in lock['cache_inventory']:
        path = inputs / relative
        input_rows.append({'path': relative, 'sizeBytes': path.stat().st_size, 'sha256': sha256(path)})
    write_json(artifacts / 'dependency-lock.json', {
        'schemaVersion': 2, 'target': lock['target'],
        'lockSha256': hashlib.sha256(lock_bytes).hexdigest(), 'inputs': input_rows,
        'patches': lock['patches'], 'recipeInventory': lock['recipe_inventory'],
    })
    write_json(artifacts / 'build-parameters.json', {
        'schemaVersion': 2, 'target': lock['target'], 'network': 'none',
        'sourceDateEpoch': EPOCH, 'compiler': 'clang-cl 22.1.4',
        'linker': 'lld-link 22.1.4', 'runtime': 'static MultiThreaded',
        'mesonArguments': lock['build_recipe']['meson_arguments'],
        'spirvCrossCmakeArguments': lock['build_recipe']['spirv_cross_cmake_arguments'],
        'ffmpegArguments': lock['build_recipe']['ffmpeg_arguments'],
    })

    projects = {}
    for name in ('freetype', 'fribidi', 'harfbuzz', 'libass', 'lcms2', 'libplacebo', 'mpv'):
        info_root = meson_root / f'build-{name}/meson-info'
        project = {}
        for kind in INTROSPECTION_KINDS:
            info = info_root / f'intro-{kind}.json'
            if not info.is_file():
                raise RuntimeError(f'Meson introspection missing: {name}/{kind}')
            project[kind] = json.loads(info.read_text(encoding='utf-8'))
        projects[name] = project
    write_json(artifacts / 'meson-introspection.json', {'schemaVersion': 2, 'projects': projects})

    pe_imports = {}
    for pe in runtime_paths.values():
        pe_text = subprocess.run(
            [str(work / 'native/bin/llvm-readobj'), '--coff-imports', str(pe)],
            check=True, capture_output=True, text=True,
        ).stdout
        pe_imports[pe.name] = sorted({
            match.group(1).upper()
            for match in re.finditer(r'^\s*Name:\s*([^\s]+\.dll)\s*$', pe_text, re.IGNORECASE | re.MULTILINE)
        })
        if not pe_imports[pe.name]:
            raise RuntimeError(f'llvm-readobj did not report PE imports for {pe.name}')
    imports = sorted({name for names in pe_imports.values() for name in names})
    unknown = sorted(set(imports) - set(lock['output_policy']['dynamic_dependencies_allowlist']))
    if unknown:
        raise RuntimeError(f'PE imports exceed locked allowlist: {unknown!r}')
    for dll in lock['output_policy']['bundled_dynamic_dependencies']:
        if dll not in pe_imports['mpv.exe']:
            raise RuntimeError(f'mpv.exe does not hard-import bundled runtime: {dll}')
    write_json(artifacts / 'pe-imports.json', {
        'schemaVersion': 2, 'artifacts': pe_imports, 'imports': imports,
        'bundled': lock['output_policy']['bundled_dynamic_dependencies'],
    })

    rows = build_component_rows(lock)
    artifact_components = []
    for role, filename in RUNTIME_FILES.items():
        artifact_components.append({
            'type': 'file', 'name': filename, 'bom-ref': f'artifact:{role}',
            'hashes': [{'alg': 'SHA-256', 'content': sha256(artifacts / filename)}],
            'properties': [
                {'name': 'autolive:role', 'value': role},
                {'name': 'autolive:usage', 'value': 'runtime'},
            ],
        })
    source_components = []
    for row in rows:
        component = {
            'type': 'library' if row['kind'] == 'source' else 'framework',
            'name': row['name'], 'version': row['version'], 'bom-ref': row['bomRef'],
            'licenses': [{'expression': row['license']}],
            'properties': [
                {'name': 'autolive:kind', 'value': row['kind']},
                {'name': 'autolive:usage', 'value': row['usage']},
                {'name': 'autolive:cache-path', 'value': row['cachePath'] or 'builder-image'},
            ],
        }
        if row['sha256']:
            component['hashes'] = [{'alg': 'SHA-256', 'content': row['sha256']}]
        source_components.append(component)
    dependencies = [{
        'ref': f'artifact:{role}',
        'dependsOn': sorted(row['bomRef'] for row in rows if role in row['runtimeArtifacts']),
    } for role in RUNTIME_FILES]
    write_json(artifacts / 'sbom.cdx.json', {
        'bomFormat': 'CycloneDX', 'specVersion': '1.6',
        'serialNumber': 'urn:uuid:8e8eb0b7-16b4-5c20-8d7e-a7d5d758b9bf', 'version': 1,
        'metadata': {'timestamp': CREATED, 'component': {'type': 'application', 'name': 'mpv-phase7a', 'version': lock['sources'][0]['commit']}},
        'components': artifact_components + source_components, 'dependencies': dependencies,
    })

    packages = []
    for row in rows:
        packages.append({
            'name': row['name'], 'SPDXID': 'SPDXRef-' + re.sub(r'[^A-Za-z0-9.-]', '-', row['bomRef']),
            'versionInfo': row['version'], 'downloadLocation': 'NOASSERTION', 'filesAnalyzed': False,
            'licenseConcluded': row['license'], 'licenseDeclared': row['license'],
            'copyrightText': 'NOASSERTION',
            'externalRefs': [{'referenceCategory': 'OTHER', 'referenceType': 'autolive-usage', 'referenceLocator': row['usage']}],
        })
    artifact_spdx = []
    relationships = []
    for role, filename in RUNTIME_FILES.items():
        artifact_id = 'SPDXRef-Artifact-' + role
        artifact_spdx.append({
            'name': filename, 'SPDXID': artifact_id, 'versionInfo': lock['sources'][0]['commit'],
            'downloadLocation': 'NOASSERTION', 'filesAnalyzed': False,
            'checksums': [{'algorithm': 'SHA256', 'checksumValue': sha256(artifacts / filename)}],
            'licenseConcluded': 'NOASSERTION', 'licenseDeclared': 'NOASSERTION',
            'copyrightText': 'NOASSERTION',
        })
        for row in rows:
            if role in row['runtimeArtifacts']:
                relationships.append({
                    'spdxElementId': artifact_id, 'relationshipType': 'GENERATED_FROM',
                    'relatedSpdxElement': 'SPDXRef-' + re.sub(r'[^A-Za-z0-9.-]', '-', row['bomRef']),
                })
    write_json(artifacts / 'sbom.spdx.json', {
        'spdxVersion': 'SPDX-2.3', 'dataLicense': 'CC0-1.0', 'SPDXID': 'SPDXRef-DOCUMENT',
        'name': 'mpv-phase7a', 'documentNamespace': 'urn:uuid:8e8eb0b7-16b4-5c20-8d7e-a7d5d758b9bf',
        'creationInfo': {'created': CREATED, 'creators': ['Tool: mpv-phase7a-generate-evidence']},
        'packages': artifact_spdx + packages, 'relationships': relationships,
    })

    (artifacts / 'license-inventory.txt').write_text(
        'kind\tname\tversion\tusage\tlicense\tsha256\tcache_path\n'
        + ''.join(
            f"{row['kind']}\t{row['name']}\t{row['version']}\t{row['usage']}\t{row['license']}\t{row['sha256'] or '-'}\t{row['cachePath'] or 'builder-image'}\n"
            for row in rows
        ), encoding='utf-8',
    )

    copyright_rows = []
    for source in lock['sources']:
        root = work / 'sources' / source['name']
        for candidate in sorted(path for path in root.rglob('*') if path.is_file() and not path.is_symlink()):
            if candidate.name.lower().startswith(('license', 'copying', 'copyright')):
                copyright_rows.append((source['name'], candidate.relative_to(root).as_posix(), sha256(candidate)))
    (artifacts / 'copyright-inventory.txt').write_text(
        'component\tpath\tsha256\n'
        + ''.join(f'{component}\t{path}\t{digest}\n' for component, path, digest in copyright_rows),
        encoding='utf-8',
    )

    with tempfile.TemporaryDirectory(dir=work) as temporary:
        stage = Path(temporary)
        patch_stage = stage / 'patches'
        recipe_stage = stage / 'recipe'
        patch_stage.mkdir()
        recipe_stage.mkdir()
        for patch in lock['patches']:
            relative = patch['path'][len(RECIPE_PREFIX):]
            shutil.copy2(recipe / relative, patch_stage / Path(relative).name)
        for item in lock['recipe_inventory']:
            relative = item['path'][len(RECIPE_PREFIX):]
            destination = recipe_stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(recipe / relative, destination)
        tar_zstd(artifacts / 'patch-bundle.tar.zst', ((patch_stage, 'patches'),), work)
        tar_zstd(
            artifacts / 'corresponding-source.tar.zst',
            ((work / 'sources', 'sources'), (recipe_stage, 'recipe'), (lock_path, 'build-lock/reproducible-build-lock.json')),
            work,
        )


def generate_report(lock_path: Path, artifacts: Path, out: Path) -> None:
    lock_bytes = lock_path.read_bytes()
    lock = json.loads(lock_bytes)
    report_artifacts = []
    for role, relative, format_name in ARTIFACT_CONTRACT:
        artifact = artifacts / relative
        if not artifact.is_file():
            raise RuntimeError(f'final evidence missing: {relative}')
        report_artifacts.append({
            'role': role, 'path': relative, 'size': artifact.stat().st_size,
            'sha256': sha256(artifact), 'format': format_name,
        })
    pe_imports = json.loads((artifacts / 'pe-imports.json').read_text(encoding='utf-8'))
    write_json(out / 'reproducible-build-report.json', {
        'schema_version': 2, 'scope': 'phase7a_supply_candidate',
        'claim': 'one_locked_cold_build_candidate',
        'lock_sha256': hashlib.sha256(lock_bytes).hexdigest(),
        'configured_inputs': {
            'meson_arguments': lock['build_recipe']['meson_arguments'],
            'spirv_cross_cmake_arguments': lock['build_recipe']['spirv_cross_cmake_arguments'],
            'ffmpeg_arguments': lock['build_recipe']['ffmpeg_arguments'],
            'cache_inventory': lock['cache_inventory'],
        },
        'artifacts': report_artifacts, 'dynamic_dependencies': pe_imports['imports'],
        'build_environment': {
            'target': lock['target'], 'builder_image': lock['toolchain']['builder_image'],
            'network': lock['fetch_policy']['build_network'], 'source_mode': lock['fetch_policy']['source_mode'],
            'source_date_epoch': lock['build_recipe']['environment']['source_date_epoch'],
            'locale': lock['build_recipe']['environment']['locale'],
            'timezone': lock['build_recipe']['environment']['timezone'],
            'path_prefix_map': lock['build_recipe']['environment']['path_prefix_map'],
        },
    })


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('--report-only', action='store_true')
    parser.add_argument('--inputs', type=Path, default=DEFAULT_INPUTS)
    parser.add_argument('--work', type=Path, default=DEFAULT_WORK)
    parser.add_argument('--meson-root', type=Path, default=DEFAULT_MESON_ROOT)
    parser.add_argument('--recipe', type=Path, default=DEFAULT_RECIPE)
    parser.add_argument('--output-root', type=Path, default=DEFAULT_OUT)
    parser.add_argument('--artifact-root', type=Path)
    parser.add_argument('--lock', type=Path)
    args = parser.parse_args()
    if args.report_only:
        if args.artifact_root is None or args.lock is None:
            parser.error('--report-only requires --artifact-root and --lock')
        generate_report(args.lock, args.artifact_root, args.output_root)
    else:
        generate_content(args.inputs, args.work, args.meson_root, args.recipe, args.output_root)


if __name__ == '__main__':
    main()
