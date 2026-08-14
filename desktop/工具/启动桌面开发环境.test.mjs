import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { resolveDevEnvironment } from './启动桌面开发环境.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));

test('开发启动自动注入本地 Worker 和 FFmpeg 资源', () => {
  const environment = resolveDevEnvironment({
    root: desktopRoot,
    env: {
      PATH: '',
    },
  });

  assert.equal(
    environment.AUTOLIVE_VOICE_CLONE_WORKER,
    `${desktopRoot}/worker/voice_clone_adapter.py`,
  );
  assert.equal(environment.AUTOLIVE_FFMPEG_PATH, `${desktopRoot}/src-tauri/binaries/ffmpeg`);
  assert.equal(environment.AUTOLIVE_FFPROBE_PATH, `${desktopRoot}/src-tauri/binaries/ffprobe`);
  assert.ok(environment.AUTOLIVE_VOICE_PYTHON);
  assert.match(environment.PATH, /autolive-voice/);
});

test('开发启动自动绑定已经准备完成的本地模型缓存', () => {
  const modelRoot = mkdtempSync(`${tmpdir()}/autolive-model-cache-`);
  const files = [
    'huggingface/hub/models--adefossez--HTDemucs/refs/main',
    'huggingface/hub/models--adefossez--HTDemucs/snapshots/test/htdemucs.yaml',
    'huggingface/hub/models--adefossez--HTDemucs/snapshots/test/model.safetensors',
    'huggingface/hub/models--Systran--faster-whisper-small/refs/main',
    'huggingface/hub/models--Systran--faster-whisper-small/snapshots/test/config.json',
    'huggingface/hub/models--Systran--faster-whisper-small/snapshots/test/model.bin',
    'huggingface/hub/models--Systran--faster-whisper-small/snapshots/test/tokenizer.json',
    'huggingface/hub/models--Systran--faster-whisper-small/snapshots/test/vocabulary.txt',
    'tts/tts/tts_models--multilingual--multi-dataset--xtts_v2/config.json',
    'tts/tts/tts_models--multilingual--multi-dataset--xtts_v2/model.pth',
    'tts/tts/tts_models--multilingual--multi-dataset--xtts_v2/speakers_xtts.pth',
    'tts/tts/tts_models--multilingual--multi-dataset--xtts_v2/vocab.json',
  ];
  for (const relativePath of files) {
    const path = `${modelRoot}/${relativePath}`;
    mkdirSync(path.slice(0, path.lastIndexOf('/')), { recursive: true });
    writeFileSync(path, 'ready');
  }

  const environment = resolveDevEnvironment({
    root: desktopRoot,
    env: {
      PATH: '',
      AUTOLIVE_VOICE_MODEL_CACHE_DIR: modelRoot,
    },
  });

  assert.equal(environment.AUTOLIVE_VOICE_CLONE_MODEL_ROOT, modelRoot);
});

test('tauri:dev 通过统一启动脚本运行当前开发版本', () => {
  const packageJson = JSON.parse(
    readFileSync(fileURLToPath(new URL('../ui/package.json', import.meta.url)), 'utf8'),
  );

  assert.match(packageJson.scripts['tauri:dev'], /启动桌面开发环境\.mjs/);
});
