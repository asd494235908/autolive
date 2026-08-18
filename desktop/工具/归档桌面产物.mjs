import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  closeSync,
  cpSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { readDesktopVersion } from './桌面版本.mjs';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function archiveDesktopArtifacts({
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
  bundleSourceDir = resolveDesktopPath(
    process.env.AUTOLIVE_BUNDLE_SOURCE_DIR || 'src-tauri/target/release/bundle',
  ),
  executableSourcePath = resolveDesktopPath(
    process.env.AUTOLIVE_RELEASE_EXECUTABLE
      || 'src-tauri/target/release/autolive-desktop-core.exe',
  ),
  manifestPath = resolveDesktopPath(
    process.env.AUTOLIVE_RUNTIME_RESOURCE_MANIFEST || 'src-tauri/runtime-resources.json',
  ),
  embeddedResourceDir = resolveDesktopPath(
    process.env.AUTOLIVE_EMBEDDED_RESOURCE_DIR || 'src-tauri/embedded-runtime-resources',
  ),
  packageRoot = resolveDesktopPath(process.env.AUTOLIVE_PACKAGE_ROOT || 'package'),
  createPortableZip = process.env.AUTOLIVE_SKIP_PORTABLE_ZIP !== '1',
} = {}) {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(targetTriple)) {
    throw new Error(`目标三元组无效：${targetTriple}`);
  }

  const destination = join(packageRoot, readDesktopVersion().release, targetTriple);
  if (targetTriple === 'x86_64-pc-windows-msvc') {
    archiveWindowsArtifacts({
      bundleSourceDir,
      createPortableZip,
      destination,
      embeddedResourceDir,
      executableSourcePath,
      manifestPath,
      targetTriple,
    });
  } else {
    archiveNativeBundle({ destination, bundleSourceDir, targetTriple });
  }
  return destination;
}

function archiveWindowsArtifacts({
  bundleSourceDir,
  createPortableZip,
  destination,
  embeddedResourceDir,
  executableSourcePath,
  manifestPath,
  targetTriple,
}) {
  const installers = requireInstallerFiles(bundleSourceDir, targetTriple);
  rmSync(destination, { force: true, recursive: true });
  mkdirSync(destination, { recursive: true });
  archiveWindowsPortable({
    createPortableZip,
    destination,
    embeddedResourceDir,
    executableSourcePath,
    manifestPath,
    targetTriple,
  });
  copyInstallerFiles(destination, installers);
}

function archiveWindowsPortable({
  createPortableZip,
  destination,
  embeddedResourceDir,
  executableSourcePath,
  manifestPath,
  targetTriple,
}) {
  if (!isFile(executableSourcePath)) {
    throw new Error(`找不到正式 Tauri EXE：${executableSourcePath}`);
  }
  if (!isFile(manifestPath)) {
    throw new Error(`找不到运行资源清单：${manifestPath}`);
  }
  if (!isDirectory(embeddedResourceDir)) {
    throw new Error(`找不到内置运行资源目录：${embeddedResourceDir}`);
  }

  const manifest = readResourceManifest(manifestPath, targetTriple);
  validateEmbeddedResources(embeddedResourceDir, manifest.files, false);

  mkdirSync(destination, { recursive: true });
  const portableRoot = join(destination, 'portable');
  const stagingRoot = join(destination, `.portable-${process.pid}.partial`);
  rmSync(stagingRoot, { force: true, recursive: true });
  mkdirSync(stagingRoot, { recursive: true });
  try {
    cpSync(executableSourcePath, join(stagingRoot, 'autolive-desktop-core.exe'));
    cpSync(manifestPath, join(stagingRoot, 'runtime-resources.json'));
    const stagedResources = join(stagingRoot, 'embedded-runtime-resources');
    cpSync(embeddedResourceDir, stagedResources, { recursive: true, dereference: true });
    validateEmbeddedResources(stagedResources, manifest.files, true);
    rmSync(portableRoot, { force: true, recursive: true });
    renameSync(stagingRoot, portableRoot);
  } catch (error) {
    rmSync(stagingRoot, { force: true, recursive: true });
    throw error;
  }

  if (createPortableZip) createWindowsPortableZip(destination);
}

function readResourceManifest(manifestPath, targetTriple) {
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(
      `运行资源清单解析失败：${error instanceof Error ? error.message : String(error)}`,
    );
  }
  const expectedRelease = readDesktopVersion().release;
  if (manifest?.schema_version !== 1) {
    throw new Error('运行资源清单版本无效');
  }
  if (manifest.release !== expectedRelease) {
    throw new Error(`运行资源清单发布版本不匹配：${manifest.release ?? '缺失'}`);
  }
  if (manifest.target !== targetTriple) {
    throw new Error(`运行资源清单目标不匹配：${manifest.target ?? '缺失'}`);
  }
  if (!Array.isArray(manifest.files) || manifest.files.length === 0) {
    throw new Error('运行资源清单没有文件');
  }
  return manifest;
}

function validateEmbeddedResources(root, files, verifyHashes) {
  const seenPaths = new Set();
  for (const file of files) {
    const relativePath = checkedManifestPath(file?.relative_path);
    if (seenPaths.has(relativePath)) {
      throw new Error(`运行资源清单包含重复路径：${relativePath}`);
    }
    seenPaths.add(relativePath);
    if (!Number.isSafeInteger(file?.size_bytes) || file.size_bytes < 0) {
      throw new Error(`运行资源大小无效：${relativePath}`);
    }
    const resourcePath = join(root, ...relativePath.split('/'));
    let resourceStat;
    try {
      resourceStat = statSync(resourcePath);
    } catch {
      throw new Error(`内置运行资源缺失：${relativePath}`);
    }
    if (!resourceStat.isFile() || resourceStat.size !== file.size_bytes) {
      throw new Error(`内置运行资源大小不匹配：${relativePath}`);
    }
    if (typeof file.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(file.sha256)) {
      throw new Error(`运行资源 SHA-256 无效：${relativePath}`);
    }
    if (verifyHashes && hashFile(resourcePath) !== file.sha256) {
      throw new Error(`内置运行资源 SHA-256 不匹配：${relativePath}`);
    }
  }
  if (countFiles(root) !== files.length) {
    throw new Error('内置运行资源文件数量与清单不匹配');
  }
}

function countFiles(root) {
  let count = 0;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) count += countFiles(path);
    else if (entry.isFile()) count += 1;
    else throw new Error(`内置运行资源包含不支持的文件类型：${path}`);
  }
  return count;
}

function hashFile(path) {
  const descriptor = openSync(path, 'r');
  const buffer = Buffer.allocUnsafe(64 * 1024);
  const hash = createHash('sha256');
  try {
    for (;;) {
      const bytesRead = readSync(descriptor, buffer, 0, buffer.length, null);
      if (bytesRead === 0) return hash.digest('hex');
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    closeSync(descriptor);
  }
}

function checkedManifestPath(value) {
  if (
    typeof value !== 'string'
    || value.length === 0
    || value.includes('\\')
    || value.includes('\0')
  ) {
    throw new Error('运行资源相对路径无效');
  }
  const segments = value.split('/');
  if (
    segments.some((segment) => (
      segment.length === 0 || segment === '.' || segment === '..' || segment.includes(':')
    ))
  ) {
    throw new Error(`运行资源相对路径无效：${value}`);
  }
  return value;
}

function createWindowsPortableZip(destination) {
  const { version } = readDesktopVersion();
  const archiveName = `autolive-desktop-core_${version}_x64-portable-with-resources.zip`;
  const archivePath = join(destination, archiveName);
  const temporaryArchive = join(destination, `.${archiveName}.${process.pid}.partial.zip`);
  rmSync(temporaryArchive, { force: true });
  const result = spawnSync('tar.exe', ['-a', '-c', '-f', temporaryArchive, 'portable'], {
    cwd: destination,
    encoding: 'utf8',
  });
  if (result.error) {
    rmSync(temporaryArchive, { force: true });
    throw new Error(`便携 ZIP 创建失败：${result.error.message}`);
  }
  if (result.status !== 0) {
    rmSync(temporaryArchive, { force: true });
    throw new Error(
      `便携 ZIP 创建失败，退出码：${result.status ?? 'unknown'} ${result.stderr?.trim() || ''}`.trim(),
    );
  }
  if (!hasZipHeader(temporaryArchive)) {
    rmSync(temporaryArchive, { force: true });
    throw new Error('便携 ZIP 创建失败：归档格式不是 ZIP');
  }
  rmSync(archivePath, { force: true });
  renameSync(temporaryArchive, archivePath);
}

function hasZipHeader(path) {
  const descriptor = openSync(path, 'r');
  const header = Buffer.alloc(2);
  try {
    return readSync(descriptor, header, 0, header.length, 0) === header.length
      && header[0] === 0x50
      && header[1] === 0x4b;
  } finally {
    closeSync(descriptor);
  }
}

function archiveNativeBundle({ destination, bundleSourceDir, targetTriple }) {
  const installers = requireInstallerFiles(bundleSourceDir, targetTriple);
  rmSync(destination, { force: true, recursive: true });
  mkdirSync(destination, { recursive: true });
  copyInstallerFiles(destination, installers);
}

function copyInstallerFiles(destination, installers) {
  for (const { directory, source } of installers) {
    const targetDirectory = join(destination, directory);
    mkdirSync(targetDirectory, { recursive: true });
    cpSync(source, join(targetDirectory, basename(source)));
  }
}

function requireInstallerFiles(bundleSourceDir, targetTriple) {
  if (!isDirectory(bundleSourceDir)) {
    throw new Error(`找不到 Tauri bundle 目录：${bundleSourceDir}`);
  }
  const installers = findInstallerFiles(bundleSourceDir, targetTriple);
  if (installers.length === 0) {
    throw new Error(`找不到 ${targetTriple} 的用户安装文件`);
  }
  return installers;
}

function findInstallerFiles(bundleSourceDir, targetTriple) {
  const rules = targetTriple.endsWith('-apple-darwin')
    ? [{ directory: 'dmg', extension: '.dmg' }]
    : targetTriple === 'x86_64-pc-windows-msvc'
      ? [{ directory: 'nsis', extension: '.exe' }]
      : [];
  return rules.flatMap(({ directory, extension }) => {
    const directoryPath = join(bundleSourceDir, directory);
    if (!isDirectory(directoryPath)) return [];
    return readdirSync(directoryPath, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(extension))
      .map((entry) => ({ directory, source: join(directoryPath, entry.name) }));
  });
}

function isDirectory(path) {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function resolveDesktopPath(value) {
  return isAbsolute(value) ? value : resolve(desktopRoot, value);
}

export function detectTargetTriple() {
  if (process.platform === 'darwin') {
    return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  }
  if (process.platform === 'win32' && process.arch === 'x64') {
    return 'x86_64-pc-windows-msvc';
  }
  throw new Error(`无法从当前环境推断目标三元组：${process.platform}/${process.arch}`);
}

function main() {
  const destination = archiveDesktopArtifacts();
  console.log(`已归档桌面产物：${destination}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
