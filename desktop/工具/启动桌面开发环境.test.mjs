import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { resolveDevEnvironment } from './启动桌面开发环境.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));

test('开发启动只自动注入 FFmpeg 资源', () => {
  const environment = resolveDevEnvironment({
    root: desktopRoot,
    env: {
      PATH: '',
    },
  });

  const executableSuffix = process.platform === 'win32' ? '.exe' : '';
  assert.equal(
    environment.AUTOLIVE_FFMPEG_PATH,
    resolve(desktopRoot, 'src-tauri', 'binaries', `ffmpeg${executableSuffix}`),
  );
  assert.equal(
    environment.AUTOLIVE_FFPROBE_PATH,
    resolve(desktopRoot, 'src-tauri', 'binaries', `ffprobe${executableSuffix}`),
  );
  assert.equal(environment.AUTOLIVE_VOICE_CLONE_WORKER, undefined);
  assert.equal(environment.AUTOLIVE_VOICE_CLONE_MODEL_ROOT, undefined);
  assert.equal(environment.AUTOLIVE_VOICE_PYTHON, undefined);
});

test('tauri:dev 通过统一启动脚本运行当前开发版本', () => {
  const packageJson = JSON.parse(
    readFileSync(fileURLToPath(new URL('../ui/package.json', import.meta.url)), 'utf8'),
  );

  assert.match(packageJson.scripts['tauri:dev'], /启动桌面开发环境\.mjs/);
});
