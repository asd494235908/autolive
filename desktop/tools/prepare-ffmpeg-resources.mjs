import { chmodSync, copyFileSync, existsSync, mkdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  assertExactRegularFileTree,
  commitStagedPaths,
  fileDescriptor,
  loadValidatedMpvRuntimeRelease,
  temporarySiblingPath,
  validatePreparedMpvRuntimeFiles,
  WINDOWS_MPV_SOURCE_TO_PREPARED,
  WINDOWS_TARGET,
} from './mpv-runtime-release.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  WINDOWS_TARGET,
]);

function detectTargetTriple() {
  if (process.platform === 'darwin') {
    return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  }
  if (process.platform === 'win32' && process.arch === 'x64') return WINDOWS_TARGET;
  return null;
}

function resolveTarget(target) {
  const value = target ?? process.env.AUTOLIVE_TARGET_TRIPLE ?? detectTargetTriple();
  if (!value || !supportedTargets.has(value)) {
    throw new Error(`当前构建平台不受支持：${value ?? `${process.platform}/${process.arch}`}`);
  }
  return value;
}

function descriptorMatches(source, destination, label) {
  const expected = fileDescriptor(source, `${label} 源文件`);
  const actual = fileDescriptor(destination, `${label} 暂存文件`);
  if (actual.size_bytes !== expected.size_bytes || actual.sha256 !== expected.sha256) {
    throw new Error(`${label} 暂存文件大小或 SHA-256 不匹配`);
  }
}

function assertFfmpegInputs(ffmpegSourceRoot, target, names) {
  const missing = names.filter((name) => {
    const path = join(ffmpegSourceRoot, target, name);
    try {
      return !statSync(path).isFile() || statSync(path).size === 0;
    } catch {
      return true;
    }
  });
  if (missing.length > 0) {
    throw new Error(
      `缺少当前目标的 FFmpeg 资源（${target}）：\n${missing
        .map((name) => join(ffmpegSourceRoot, target, name))
        .join('\n')}`,
    );
  }
}

export async function prepareMediaRuntimeResources({
  target: requestedTarget,
  ffmpegSourceRoot = resolve(
    process.env.AUTOLIVE_FFMPEG_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'ffmpeg'),
  ),
  mpvSourceRoot = resolve(
    process.env.AUTOLIVE_MPV_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'mpv'),
  ),
  outputRoot = resolve(
    process.env.AUTOLIVE_FFMPEG_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'binaries'),
  ),
  supplyGate,
} = {}) {
  const target = resolveTarget(requestedTarget);
  const extension = target === WINDOWS_TARGET ? '.exe' : '';
  const ffmpegNames = [`ffmpeg${extension}`, `ffprobe${extension}`];
  assertFfmpegInputs(ffmpegSourceRoot, target, ffmpegNames);

  let mpvRelease = null;
  if (target === WINDOWS_TARGET) {
    mpvRelease = await loadValidatedMpvRuntimeRelease({
      mpvSourceRoot,
      target,
      mode: 'release',
      supplyGate,
    });
  }

  const staged = temporarySiblingPath(outputRoot);
  try {
    mkdirSync(staged, { recursive: true });
    for (const name of ffmpegNames) {
      const source = join(ffmpegSourceRoot, target, name);
      const destination = join(staged, name);
      copyFileSync(source, destination);
      if (target !== WINDOWS_TARGET) chmodSync(destination, 0o755);
      descriptorMatches(source, destination, name);
    }
    if (target === WINDOWS_TARGET) {
      for (const [sourceName, outputName] of WINDOWS_MPV_SOURCE_TO_PREPARED) {
        const source = join(mpvRelease.targetRoot, ...sourceName.split('/'));
        const destination = join(staged, ...outputName.split('/'));
        mkdirSync(dirname(destination), { recursive: true });
        copyFileSync(source, destination);
        descriptorMatches(source, destination, outputName);
      }
      validatePreparedMpvRuntimeFiles({ binariesRoot: staged, release: mpvRelease, mode: 'release' });
    } else {
      assertExactRegularFileTree(staged, ffmpegNames, 'FFmpeg 暂存运行资源树');
    }
    commitStagedPaths([{ target: outputRoot, staged }]);
  } finally {
    if (existsSync(staged)) rmSync(staged, { recursive: true, force: true });
  }
  return { target, outputRoot, manifest: mpvRelease?.manifest ?? null };
}

async function main() {
  const result = await prepareMediaRuntimeResources();
  console.log(
    `已准备 FFmpeg/FFprobe${result.target === WINDOWS_TARGET ? '/mpv' : ''} 资源：${result.target}`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
