import { cpSync, mkdirSync, readdirSync, rmSync, statSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { readDesktopVersion } from './桌面版本.mjs';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function archiveDesktopArtifacts({
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
  bundleSourceDir = resolveDesktopPath(
    process.env.AUTOLIVE_BUNDLE_SOURCE_DIR || 'src-tauri/target/release/bundle',
  ),
  packageRoot = resolveDesktopPath(process.env.AUTOLIVE_PACKAGE_ROOT || 'package'),
} = {}) {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(targetTriple)) {
    throw new Error(`目标三元组无效：${targetTriple}`);
  }

  const destination = join(packageRoot, readDesktopVersion().release, targetTriple);
  archiveNativeBundle({ destination, bundleSourceDir, targetTriple });
  return destination;
}

function archiveNativeBundle({ destination, bundleSourceDir, targetTriple }) {
  if (!isDirectory(bundleSourceDir)) {
    throw new Error(`找不到 Tauri bundle 目录：${bundleSourceDir}`);
  }
  const installers = findInstallerFiles(bundleSourceDir, targetTriple);
  if (installers.length === 0) {
    throw new Error(`找不到 ${targetTriple} 的用户安装文件`);
  }
  rmSync(destination, { force: true, recursive: true });
  mkdirSync(destination, { recursive: true });
  for (const { directory, source } of installers) {
    const targetDirectory = join(destination, directory);
    mkdirSync(targetDirectory, { recursive: true });
    cpSync(source, join(targetDirectory, basename(source)));
  }
}

function findInstallerFiles(bundleSourceDir, targetTriple) {
  const rules = targetTriple.endsWith('-apple-darwin')
    ? [{ directory: 'dmg', extension: '.dmg' }]
    : targetTriple === 'x86_64-pc-windows-msvc'
      ? [
          { directory: 'msi', extension: '.msi' },
          { directory: 'nsis', extension: '.exe' },
        ]
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

function resolveDesktopPath(value) {
  return isAbsolute(value) ? value : resolve(desktopRoot, value);
}

function detectTargetTriple() {
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
