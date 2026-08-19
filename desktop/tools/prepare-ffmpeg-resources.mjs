import { chmodSync, copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const sourceRoot = resolve(
  process.env.AUTOLIVE_FFMPEG_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'ffmpeg'),
);
const outputRoot = resolve(
  process.env.AUTOLIVE_FFMPEG_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'binaries'),
);
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  'x86_64-pc-windows-msvc',
]);

function targetTriple() {
  const value =
    process.env.AUTOLIVE_TARGET_TRIPLE ??
    (process.platform === 'darwin' && process.arch === 'x64'
      ? 'x86_64-apple-darwin'
      : process.platform === 'darwin' && process.arch === 'arm64'
        ? 'aarch64-apple-darwin'
        : process.platform === 'win32' && process.arch === 'x64'
          ? 'x86_64-pc-windows-msvc'
          : null);
  if (!value || !supportedTargets.has(value)) {
    throw new Error(`当前构建平台不受支持：${value ?? `${process.platform}/${process.arch}`}`);
  }
  return value;
}

const target = targetTriple();
const extension = target === 'x86_64-pc-windows-msvc' ? '.exe' : '';
const inputRoot = join(sourceRoot, target);
const inputs = [
  { name: `ffmpeg${extension}`, output: `ffmpeg${extension}` },
  { name: `ffprobe${extension}`, output: `ffprobe${extension}` },
];

const missing = inputs.filter(({ name }) => {
  const path = join(inputRoot, name);
  try {
    return !statSync(path).isFile() || statSync(path).size === 0;
  } catch {
    return true;
  }
});

if (missing.length > 0) {
  const names = missing.map(({ name }) => join(inputRoot, name)).join('\n');
  throw new Error(`缺少当前目标的 FFmpeg 资源（${target}）：\n${names}`);
}

mkdirSync(outputRoot, { recursive: true });
for (const { name, output } of inputs) {
  const source = join(inputRoot, name);
  const destination = join(outputRoot, output);
  copyFileSync(source, destination);
  if (target !== 'x86_64-pc-windows-msvc') {
    chmodSync(destination, 0o755);
  }
}

console.log(`已准备 FFmpeg/FFprobe 资源：${target}`);
