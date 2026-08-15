import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import {
  resolvePython,
  resolveWorkerBinaryPath,
  workerBinaryName,
  workerResourceLayout,
} from './准备语音Worker资源.mjs';

test('显式 Python 命令名从 PATH 解析', () => {
  const path = resolvePython('node');

  assert.notEqual(path, join(process.cwd(), 'node'));
});

test('Worker 资源使用当前平台的可执行文件名', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-voice-worker-'));
  const layout = workerResourceLayout(root, 'aarch64-apple-darwin');

  assert.equal(workerBinaryName('aarch64-apple-darwin'), 'autolive-voice-clone-worker');
  assert.equal(
    resolveWorkerBinaryPath(layout.outputRoot, 'aarch64-apple-darwin'),
    join(layout.outputRoot, 'autolive-voice-clone-worker'),
  );
});

test('正式打包脚本把 Worker 放入 Tauri 资源目录', () => {
  const packageJson = JSON.parse(
    readFileSync(fileURLToPath(new URL('../ui/package.json', import.meta.url)), 'utf8'),
  );

  assert.match(packageJson.scripts['tauri:build'], /准备语音Worker资源\.mjs/);
});
