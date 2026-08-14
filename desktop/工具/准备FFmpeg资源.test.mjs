import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const script = fileURLToPath(new URL('./准备FFmpeg资源.mjs', import.meta.url));

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
  const tauriScripts = `${packageJson.scripts['tauri:dev']}\n${packageJson.scripts['tauri:build']}`;

  assert.match(tauriScripts, /\btauri (?:dev|build)\b/);
  assert.doesNotMatch(tauriScripts, /\.\/ui\/node_modules\/\.bin\/tauri/);
});
