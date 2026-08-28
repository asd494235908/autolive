import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const sourceRoot = resolve(
  process.env.AUTOLIVE_FFMPEG_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'ffmpeg'),
);
const mpvSourceRoot = resolve(
  process.env.AUTOLIVE_MPV_SOURCE_DIR ?? join(desktopRoot, 'third_party', 'mpv'),
);
const outputRoot = resolve(
  process.env.AUTOLIVE_FFMPEG_OUTPUT_DIR ?? join(desktopRoot, 'src-tauri', 'binaries'),
);
const supportedTargets = new Set([
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  'x86_64-pc-windows-msvc',
]);
const mpvLegalFiles = [
  'mpv-runtime-manifest.json',
  'Copyright.txt',
  'GPL-2.0.txt',
  'LGPL-2.1.txt',
  'SOURCE.md',
  'D3DCOMPILER_43-EULA.txt',
];
const mpvHashedFiles = [
  'mpv.exe',
  'd3dcompiler_43.dll',
  ...mpvLegalFiles.filter((name) => name !== 'mpv-runtime-manifest.json').map((name) => join('legal', name)),
];
const fixedMpvManifestValues = [
  ['schema_version', 1],
  ['artifact.build_repository', 'https://github.com/shinchiro/mpv-winbuild-cmake'],
  ['artifact.release', '20260814'],
  ['artifact.asset', 'mpv-x86_64-20260814-git-7b8915bc1d.7z'],
  ['artifact.download_url', 'https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260814/mpv-x86_64-20260814-git-7b8915bc1d.7z'],
  ['artifact.archive_sha256', '1bf3b029da2c98e605e00e85f21ee3142f22a1dcc4ceb5c827b5c51e36e390f9'],
  ['components.mpv.version', 'v0.41.0-923-g7b8915bc1'],
  ['components.mpv.built_at', 'Aug 14 2026 00:27:31'],
  ['components.mpv.source_repository', 'https://github.com/mpv-player/mpv'],
  ['components.mpv.source_ref', '7b8915bc1d'],
  ['components.mpv.license_expression', 'GPL-2.0-or-later'],
  ['components.mpv.copyright_file', 'Copyright.txt'],
  ['components.mpv.license_file', 'GPL-2.0.txt'],
  ['components.mpv.source_file', 'SOURCE.md'],
  ['components.libplacebo.version', 'v7.371.0'],
  ['components.libplacebo.build_revision', 'v7.360.0-111-g22ee762-dirty'],
  ['components.libplacebo.source_repository', 'https://github.com/haasn/libplacebo'],
  ['components.libplacebo.source_ref', '22ee762'],
  ['components.libplacebo.license_expression', 'LGPL-2.1-or-later'],
  ['components.libplacebo.license_file', 'LGPL-2.1.txt'],
  ['components.libplacebo.source_file', 'SOURCE.md'],
  ['components.d3dcompiler_43.license_expression', 'LicenseRef-Microsoft-DirectX-Redistributable-EULA'],
  ['components.d3dcompiler_43.license_file', 'D3DCOMPILER_43-EULA.txt'],
];

function nestedValue(value, path) {
  return path.split('.').reduce((current, key) => current?.[key], value);
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function validateMpvManifest(manifestPath, binaryRoot) {
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(`mpv 发布清单无法读取：${manifestPath}\n${error instanceof Error ? error.message : error}`);
  }
  for (const [path, expected] of fixedMpvManifestValues) {
    if (nestedValue(manifest, path) !== expected) {
      throw new Error(`mpv 发布清单字段不匹配：${path}`);
    }
  }
  for (const name of mpvHashedFiles) {
    const manifestName = name.replaceAll('\\', '/');
    const declaredHash = manifest.files?.[manifestName]?.sha256;
    if (!/^[0-9a-f]{64}$/.test(declaredHash ?? '') || declaredHash !== sha256(join(binaryRoot, ...manifestName.split('/')))) {
      throw new Error(`mpv 发布清单文件哈希不匹配：${manifestName}`);
    }
  }
}

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
const inputs = [
  { root: sourceRoot, name: `ffmpeg${extension}`, output: `ffmpeg${extension}`, product: 'FFmpeg' },
  { root: sourceRoot, name: `ffprobe${extension}`, output: `ffprobe${extension}`, product: 'FFmpeg' },
];
if (target === 'x86_64-pc-windows-msvc') {
  inputs.push(
    { root: mpvSourceRoot, name: 'mpv.exe', output: 'mpv.exe', product: 'mpv' },
    { root: mpvSourceRoot, name: 'd3dcompiler_43.dll', output: 'd3dcompiler_43.dll', product: 'mpv' },
    ...mpvLegalFiles.map((name) => ({
      root: mpvSourceRoot,
      name: join('legal', name),
      output: join('licenses', 'mpv', name),
      product: 'mpv 许可证',
    })),
  );
}

const missing = inputs.filter(({ root, name }) => {
  const path = join(root, target, name);
  try {
    return !statSync(path).isFile() || statSync(path).size === 0;
  } catch {
    return true;
  }
});

if (missing.length > 0) {
  const names = missing.map(({ root, name }) => join(root, target, name)).join('\n');
  const products = [...new Set(missing.map(({ product }) => product))].join('/');
  throw new Error(`缺少当前目标的 ${products} 资源（${target}）：\n${names}`);
}

if (target === 'x86_64-pc-windows-msvc') {
  validateMpvManifest(
    join(mpvSourceRoot, target, 'legal', 'mpv-runtime-manifest.json'),
    join(mpvSourceRoot, target),
  );
}

mkdirSync(outputRoot, { recursive: true });
for (const { root, name, output } of inputs) {
  const source = join(root, target, name);
  const destination = join(outputRoot, output);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
  if (target !== 'x86_64-pc-windows-msvc') {
    chmodSync(destination, 0o755);
  }
}

console.log(`已准备 FFmpeg/FFprobe${target === 'x86_64-pc-windows-msvc' ? '/mpv' : ''} 资源：${target}`);
