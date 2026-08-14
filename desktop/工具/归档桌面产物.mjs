import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const configPath = join(desktopRoot, 'src-tauri', 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8'));
const version = config.version;

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`Tauri 版本号无效：${version}`);
}

const targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple();
if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(targetTriple)) {
  throw new Error(`目标三元组无效：${targetTriple}`);
}

const sourceDir = resolveDesktopPath(
  process.env.AUTOLIVE_BUNDLE_SOURCE_DIR || 'src-tauri/target/release/bundle',
);
const packageRoot = resolveDesktopPath(process.env.AUTOLIVE_PACKAGE_ROOT || 'package');
const destination = join(packageRoot, `v${version}`, targetTriple);

if (!existsSync(sourceDir)) {
  throw new Error(`找不到 Tauri bundle 目录：${sourceDir}`);
}

rmSync(destination, { force: true, recursive: true });
mkdirSync(destination, { recursive: true });
cpSync(sourceDir, destination, { recursive: true });

console.log(`已归档桌面产物：${destination}`);

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
