import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { resolveDevEnvironment } from './start-desktop-dev.mjs';

const desktopRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));

test('开发启动默认使用固定测试控制面并允许显式环境变量覆盖', () => {
  const defaults = resolveDevEnvironment({ root: desktopRoot, env: {} });
  assert.equal(defaults.VITE_CONTROL_PLANE_BASE_URL, 'http://101.96.208.132:9090');
  assert.equal(defaults.VITE_CONTROL_PLANE_ENV, 'test');

  const overrides = resolveDevEnvironment({
    root: desktopRoot,
    env: {
      VITE_CONTROL_PLANE_BASE_URL: 'https://control.example.com',
      VITE_CONTROL_PLANE_ENV: 'production',
    },
  });
  assert.equal(overrides.VITE_CONTROL_PLANE_BASE_URL, 'https://control.example.com');
  assert.equal(overrides.VITE_CONTROL_PLANE_ENV, 'production');
});

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

test('开发启动允许显式选择测试 Tauri 配置', () => {
  const environment = resolveDevEnvironment({
    root: desktopRoot,
    env: {
      AUTOLIVE_TAURI_CONFIG: 'src-tauri/tauri.test.conf.json',
    },
  });

  assert.equal(environment.AUTOLIVE_TAURI_CONFIG, 'src-tauri/tauri.test.conf.json');
});

test('开发启动默认使用测试 Tauri 配置', () => {
  const source = readFileSync(new URL('./start-desktop-dev.mjs', import.meta.url), 'utf8');
  assert.match(source, /AUTOLIVE_TAURI_CONFIG \|\| 'src-tauri\/tauri\.test\.conf\.json'/);
});

test('测试 Tauri capability 只放行明确的测试控制面 origin', () => {
  const defaultCapability = JSON.parse(
    readFileSync(resolve(desktopRoot, 'src-tauri', 'capabilities', 'default.json'), 'utf8'),
  );
  const testCapability = JSON.parse(
    readFileSync(resolve(desktopRoot, 'src-tauri', 'capabilities', 'test-control-plane.json'), 'utf8'),
  );
  const testConfig = JSON.parse(
    readFileSync(resolve(desktopRoot, 'src-tauri', 'tauri.test.conf.json'), 'utf8'),
  );
  const testHttpPermission = testCapability.permissions.find(
    (permission) => permission?.identifier === 'http:default',
  );

  assert.equal(testCapability.identifier, 'test-control-plane');
  assert.ok(testConfig.app.security.capabilities.includes('test-control-plane'));
  assert.deepEqual(testHttpPermission.allow, [
    { url: 'http://127.0.0.1:18090/**' },
    { url: 'http://101.96.208.132:9090/**' },
    { url: 'https://admin.example.com/**' },
  ]);
  assert.equal(
    defaultCapability.permissions.find((permission) => permission?.identifier === 'http:default')?.allow.some(
      (entry) => entry.url.includes('101.96.208.132'),
    ),
    false,
  );
  assert.equal(
    testHttpPermission.allow.some((entry) => entry.url === 'http://101.96.208.132:9090/**'),
    true,
  );
});

test('tauri:dev 通过统一启动脚本运行当前开发版本', () => {
  const packageJson = JSON.parse(
    readFileSync(fileURLToPath(new URL('../ui/package.json', import.meta.url)), 'utf8'),
  );

  assert.match(packageJson.scripts['tauri:dev'], /start-desktop-dev\.mjs/);
});
