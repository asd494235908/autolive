import test from 'node:test';
import assert from 'node:assert/strict';
import { dirname, join, resolve } from 'node:path';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { resolvePython, resolveTargetTriple, modelResourceLayout, assertModelCacheComplete } from './准备语音模型资源.mjs';

test('resolveTargetTriple rejects unsupported targets', () => {
  assert.throws(
    () => resolveTargetTriple('unsupported-target'),
    /当前构建平台不受支持/,
  );
});

test('resolvePython rejects a missing explicit interpreter', () => {
  assert.throws(
    () => resolvePython(resolve('autolive-python-that-does-not-exist')),
    /找不到可用的 Python/,
  );
});

test('modelResourceLayout lists all packaged model roots', () => {
  const root = resolve('autolive-models');
  assert.deepEqual(modelResourceLayout(root), {
    demucs: join(root, 'huggingface', 'hub', 'models--adefossez--HTDemucs'),
    whisper: join(root, 'huggingface', 'hub', 'models--Systran--faster-whisper-small'),
    xtts: join(root, 'tts', 'tts', 'tts_models--multilingual--multi-dataset--xtts_v2'),
  });
});

test('assertModelCacheComplete rejects an incomplete cache', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-model-cache-'));
  assert.throws(() => assertModelCacheComplete(root), /模型缓存不完整/);
});

test('assertModelCacheComplete rejects non-model placeholders', () => {
  const root = mkdtempSync(join(tmpdir(), 'autolive-model-cache-'));
  for (const relativePath of [
    ['huggingface', 'hub', 'models--adefossez--HTDemucs', 'README.md'],
    ['huggingface', 'hub', 'models--Systran--faster-whisper-small', 'README.md'],
    ['tts', 'tts', 'tts_models--multilingual--multi-dataset--xtts_v2', 'README.md'],
  ]) {
    const path = join(root, ...relativePath);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, 'placeholder');
  }
  assert.throws(() => assertModelCacheComplete(root), /模型缓存不完整/);
});

test('正式打包命令会在 Tauri 之前准备并校验 Worker 与语音模型资源', () => {
  const packageJsonPath = fileURLToPath(new URL('../ui/package.json', import.meta.url));
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
  const buildScript = packageJson.scripts['tauri:build'];

  assert.match(buildScript, /准备语音Worker资源\.mjs/);
  assert.match(buildScript, /准备语音模型资源\.mjs/);
  assert.match(buildScript, /tauri build/);

  const tauriConfig = JSON.parse(
    readFileSync(fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url)), 'utf8'),
  );
  assert.ok(tauriConfig.bundle.resources.includes('voice-models'));
  assert.ok(tauriConfig.bundle.resources.includes('voice-worker'));
});

test('CI 打包环境会安装 Worker 构建依赖并显式传入许可确认', () => {
  const workflow = readFileSync(
    fileURLToPath(new URL('../../.github/workflows/desktop-package.yml', import.meta.url)),
    'utf8',
  );

  assert.match(workflow, /requirements-voice-clone-build\.txt/);
  assert.match(workflow, /AUTOLIVE_COQUI_TOS_AGREE/);
});
