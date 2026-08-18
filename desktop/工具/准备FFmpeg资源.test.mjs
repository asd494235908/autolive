import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

import { tauriBuildArguments } from './构建桌面产物.mjs';
import { readDesktopVersion } from './桌面版本.mjs';

const script = fileURLToPath(new URL('./准备FFmpeg资源.mjs', import.meta.url));
const archiveScript = fileURLToPath(new URL('./归档桌面产物.mjs', import.meta.url));
const buildScript = fileURLToPath(new URL('./构建桌面产物.mjs', import.meta.url));

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

test('缺少当前目标的 FFmpeg 文件时失败且不创建输出', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-ffmpeg-'));
  const source = join(root, 'source');
  const output = join(root, 'output');
  const target = 'aarch64-apple-darwin';
  const result = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: source,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: target,
    },
    encoding: 'utf8',
  });

  assert.notEqual(result.status, 0);
  assert.match(`${result.stdout}${result.stderr}`, /缺少当前目标的 FFmpeg 资源/);
  assert.throws(() => statSync(output));
});

test('按目标三元组复制当前包所需的标准资源名', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-ffmpeg-'));
  const source = join(root, 'source', 'aarch64-apple-darwin');
  const output = join(root, 'output');
  const sourceDir = join(root, 'source');
  mkdirSync(source, { recursive: true });
  writeFileSync(join(source, 'ffmpeg'), 'ffmpeg-test');
  writeFileSync(join(source, 'ffprobe'), 'ffprobe-test');

  const result = spawnSync(process.execPath, [script], {
    env: {
      ...process.env,
      AUTOLIVE_FFMPEG_SOURCE_DIR: sourceDir,
      AUTOLIVE_FFMPEG_OUTPUT_DIR: output,
      AUTOLIVE_TARGET_TRIPLE: 'aarch64-apple-darwin',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.equal(readFileSync(join(output, 'ffmpeg'), 'utf8'), 'ffmpeg-test');
  assert.equal(readFileSync(join(output, 'ffprobe'), 'utf8'), 'ffprobe-test');
});

test('Tauri scripts use the package binary lookup that works on Windows', () => {
  const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
  const buildSource = readFileSync(buildScript, 'utf8');

  assert.match(packageJson.scripts['tauri:build'], /构建桌面产物\.mjs/);
  assert.match(buildSource, /'tauri'/);
  assert.deepEqual(tauriBuildArguments('x86_64-pc-windows-msvc'), [
    'build',
    '--config',
    'src-tauri/tauri.conf.json',
    '--bundles',
    'nsis',
  ]);
  assert.doesNotMatch(`${packageJson.scripts['tauri:dev']}\n${buildSource}`, /\.\/ui\/node_modules\/\.bin\/tauri/);
});

test('归档脚本按版本和目标平台目录保存 bundle', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-package-'));
  const source = join(root, 'bundle');
  const output = join(root, 'package');
  const { release } = readDesktopVersion();
  mkdirSync(join(source, 'dmg'), { recursive: true });
  mkdirSync(join(source, 'macos', 'autolive.app'), { recursive: true });
  writeFileSync(join(source, 'dmg', 'autolive.dmg'), 'bundle-test');
  writeFileSync(join(source, 'macos', 'autolive.app', 'Contents.txt'), 'not-an-installer');

  const result = spawnSync(process.execPath, [archiveScript], {
    env: {
      ...process.env,
      AUTOLIVE_BUNDLE_SOURCE_DIR: source,
      AUTOLIVE_PACKAGE_ROOT: output,
      AUTOLIVE_TARGET_TRIPLE: 'aarch64-apple-darwin',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  assert.equal(
    readFileSync(join(output, release, 'aarch64-apple-darwin', 'dmg', 'autolive.dmg'), 'utf8'),
    'bundle-test',
  );
  assert.throws(() => statSync(join(output, release, 'aarch64-apple-darwin', 'macos')));
});

test('Windows 正式包归档 NSIS EXE 和 Tauri portable', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-package-'));
  const executable = join(root, 'target', 'release', 'autolive-desktop-core.exe');
  const manifest = join(root, 'runtime-resources.json');
  const embedded = join(root, 'embedded-runtime-resources');
  const bundleSource = join(root, 'target', 'release', 'bundle');
  const output = join(root, 'package');
  const { release } = readDesktopVersion();
  mkdirSync(join(root, 'target', 'release'), { recursive: true });
  mkdirSync(join(bundleSource, 'nsis'), { recursive: true });
  mkdirSync(join(embedded, 'x86_64-pc-windows-msvc', 'binaries'), { recursive: true });
  writeFileSync(executable, 'app-test');
  writeFileSync(join(bundleSource, 'nsis', 'autolive-setup.exe'), 'setup-test');
  writeFileSync(join(embedded, 'x86_64-pc-windows-msvc', 'binaries', 'ffmpeg.exe'), 'ffmpeg');
  writeFileSync(manifest, JSON.stringify({
    schema_version: 1,
    release,
    target: 'x86_64-pc-windows-msvc',
    files: [{
      relative_path: 'x86_64-pc-windows-msvc/binaries/ffmpeg.exe',
      size_bytes: 6,
      sha256: sha256('ffmpeg'),
      component: 'media',
      executable: true,
    }],
  }));

  const result = spawnSync(process.execPath, [archiveScript], {
    env: {
      ...process.env,
      AUTOLIVE_RELEASE_EXECUTABLE: executable,
      AUTOLIVE_RUNTIME_RESOURCE_MANIFEST: manifest,
      AUTOLIVE_EMBEDDED_RESOURCE_DIR: embedded,
      AUTOLIVE_BUNDLE_SOURCE_DIR: bundleSource,
      AUTOLIVE_PACKAGE_ROOT: output,
      AUTOLIVE_TARGET_TRIPLE: 'x86_64-pc-windows-msvc',
      AUTOLIVE_SKIP_PORTABLE_ZIP: '1',
    },
    encoding: 'utf8',
  });

  assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
  const bundle = join(output, release, 'x86_64-pc-windows-msvc');
  assert.equal(readFileSync(join(bundle, 'portable', 'autolive-desktop-core.exe'), 'utf8'), 'app-test');
  assert.equal(readFileSync(join(bundle, 'nsis', 'autolive-setup.exe'), 'utf8'), 'setup-test');
  assert.throws(() => statSync(join(bundle, 'msi')));
});

test('GitHub workflow uploads the versioned package directory', () => {
  const workflowPath = fileURLToPath(
    new URL('../../.github/workflows/desktop-package.yml', import.meta.url),
  );
  const workflow = readFileSync(workflowPath, 'utf8');

  assert.match(workflow, /id: desktop-version/);
  assert.match(
    workflow,
    /path: desktop\/package\/\$\{\{ steps\.desktop-version\.outputs\.version \}\}\/\$\{\{ matrix\.target_triple \}\}\/\*\*/,
  );
  assert.match(
    workflow,
    /name: desktop-bundle-\$\{\{ steps\.desktop-version\.outputs\.version \}\}-\$\{\{ matrix\.target_triple \}\}/,
  );
});
