import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { archiveDesktopArtifacts, detectTargetTriple } from './archive-desktop-artifact.mjs';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function tauriBuildArguments(
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
) {
  const argumentsList = ['build', '--config', 'src-tauri/tauri.conf.json'];
  if (targetTriple === 'x86_64-pc-windows-msvc') {
    argumentsList.push('--bundles', 'nsis');
    return argumentsList;
  }
  const bundles = process.env.AUTOLIVE_BUNDLES?.trim();
  if (bundles) argumentsList.push('--bundles', bundles);
  return argumentsList;
}

export function buildDesktopArtifacts(
  targetTriple = process.env.AUTOLIVE_TARGET_TRIPLE?.trim() || detectTargetTriple(),
) {
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
