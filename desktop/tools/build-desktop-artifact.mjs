import { spawnSync } from 'node:child_process';
import { rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { archiveDesktopArtifacts, detectTargetTriple } from './archive-desktop-artifact.mjs';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const TEST_CONTROL_PLANE_BASE_URL = 'http://101.96.208.132:9090';

function configuredTargetRoot() {
  return resolve(process.env.CARGO_TARGET_DIR?.trim() || resolve(desktopRoot, 'src-tauri', 'target'));
}

function configuredBuildDirectory(profile = process.env.AUTOLIVE_BUILD_PROFILE?.trim() || '') {
  return profile === 'test' ? 'debug' : 'release';
}

export function applyBuildProfile(profile = process.argv.includes('--profile=test') ? 'test' : '') {
  if (profile !== 'test') return;
  const configuredBaseUrl = process.env.VITE_CONTROL_PLANE_BASE_URL?.trim();
  if (configuredBaseUrl && configuredBaseUrl !== TEST_CONTROL_PLANE_BASE_URL) {
    throw new Error(`测试包只能使用 ${TEST_CONTROL_PLANE_BASE_URL}`);
  }
  process.env.VITE_CONTROL_PLANE_BASE_URL = TEST_CONTROL_PLANE_BASE_URL;
  process.env.VITE_CONTROL_PLANE_ENV = 'test';
  process.env.AUTOLIVE_TAURI_CONFIG = 'src-tauri/tauri.test.conf.json';
  process.env.AUTOLIVE_BUILD_PROFILE = 'test';
  process.env.CARGO_TARGET_DIR ||= resolve(desktopRoot, 'src-tauri', 'target-test-package');
  const debugRoot = resolve(configuredTargetRoot(), 'debug');
  process.env.AUTOLIVE_BUNDLE_SOURCE_DIR ||= resolve(debugRoot, 'bundle');
  process.env.AUTOLIVE_RELEASE_EXECUTABLE ||= resolve(debugRoot, 'autolive-desktop-core.exe');
  process.env.AUTOLIVE_PACKAGE_ROOT ||= resolve(desktopRoot, 'package-test');
}

export function tauriBuildArguments(
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
) {
  const config = process.env.AUTOLIVE_TAURI_CONFIG?.trim() || 'src-tauri/tauri.conf.json';
  const argumentsList = ['build', '--config', config];
  if (process.env.AUTOLIVE_BUILD_PROFILE === 'test') {
    argumentsList.push('--debug', '--no-sign');
  }
  if (targetTriple === 'x86_64-pc-windows-msvc') {
    argumentsList.push('--bundles', 'nsis');
    return argumentsList;
  }
  const bundles = process.env.AUTOLIVE_BUNDLES?.trim();
  if (bundles) argumentsList.push('--bundles', bundles);
  return argumentsList;
}

export function cleanBundleOutputForTarget(
  targetTriple,
  root = resolve(configuredTargetRoot(), configuredBuildDirectory(), 'bundle'),
) {
  if (targetTriple !== 'x86_64-pc-windows-msvc') return null;
  const resolvedRoot = resolve(root);
  const outputDirectory = resolve(resolvedRoot, 'nsis');
  if (dirname(outputDirectory) !== resolvedRoot) {
    throw new Error(`拒绝清理 bundle 根目录之外的路径：${outputDirectory}`);
  }
  rmSync(outputDirectory, { force: true, recursive: true });
  return outputDirectory;
}

export function buildDesktopArtifacts(
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
) {
  cleanBundleOutputForTarget(targetTriple);
  const result = spawnSync('tauri', tauriBuildArguments(targetTriple), {
    cwd: desktopRoot,
    shell: process.platform === 'win32',
    stdio: 'inherit',
  });
  if (result.error) {
    throw new Error(`Tauri 构建启动失败：${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`Tauri 构建失败，退出码：${result.status ?? 'unknown'}`);
  }
  return archiveDesktopArtifacts({ targetTriple });
}

function main() {
  applyBuildProfile();
  const destination = buildDesktopArtifacts();
  console.log(`已构建并归档桌面产物：${destination}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
