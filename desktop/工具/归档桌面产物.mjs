import { cpSync, mkdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const configPath = join(desktopRoot, 'src-tauri', 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8'));

export function archiveDesktopArtifacts({
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
  bundleSourceDir = resolveDesktopPath(
    process.env.AUTOLIVE_BUNDLE_SOURCE_DIR || 'src-tauri/target/release/bundle',
  ),
  releaseDir = resolveDesktopPath(
    process.env.AUTOLIVE_RELEASE_DIR || 'src-tauri/target/release',
  ),
  resourceRoot = resolveDesktopPath(
    process.env.AUTOLIVE_RESOURCE_ROOT || 'src-tauri',
  ),
  packageRoot = resolveDesktopPath(process.env.AUTOLIVE_PACKAGE_ROOT || 'package'),
} = {}) {
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(config.version)) {
    throw new Error(`Tauri 版本号无效：${config.version}`);
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(targetTriple)) {
    throw new Error(`目标三元组无效：${targetTriple}`);
  }

  const destination = join(packageRoot, `v${config.version}`, targetTriple);
  if (targetTriple === 'x86_64-pc-windows-msvc') {
    archiveWindowsPortable({ destination, releaseDir, resourceRoot });
  } else {
    archiveNativeBundle({ destination, bundleSourceDir });
  }
  return destination;
}

function archiveNativeBundle({ destination, bundleSourceDir }) {
  if (!isDirectory(bundleSourceDir)) {
    throw new Error(`找不到 Tauri bundle 目录：${bundleSourceDir}`);
  }
  rmSync(destination, { force: true, recursive: true });
  mkdirSync(destination, { recursive: true });
  cpSync(bundleSourceDir, destination, { recursive: true });
}

function archiveWindowsPortable({ destination, releaseDir, resourceRoot }) {
  const executable = join(releaseDir, `${config.productName}.exe`);
  const requiredFiles = [
    executable,
    join(resourceRoot, 'binaries', 'ffmpeg.exe'),
    join(resourceRoot, 'binaries', 'ffprobe.exe'),
    join(resourceRoot, 'voice-worker', 'autolive-voice-clone-worker.exe'),
  ];
  for (const path of requiredFiles) {
    if (!isFile(path)) throw new Error(`Windows 便携包缺少资源：${path}`);
  }
  const modelRoot = join(resourceRoot, 'voice-models');
  if (!isDirectory(modelRoot)) {
    throw new Error(`Windows 便携包缺少模型目录：${modelRoot}`);
  }

  const portableRoot = join(destination, 'portable');
  rmSync(destination, { force: true, recursive: true });
  mkdirSync(portableRoot, { recursive: true });
  cpSync(executable, join(portableRoot, `${config.productName}.exe`));
  for (const directory of ['binaries', 'voice-models', 'voice-worker']) {
    cpSync(join(resourceRoot, directory), join(portableRoot, directory), { recursive: true });
  }
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
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
