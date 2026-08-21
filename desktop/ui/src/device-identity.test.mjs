import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadModule() {
  const source = await readFile(new URL('./deviceIdentity.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

function createStorage() {
  const values = new Map();
  return {
    getItem(key) { return values.get(key) ?? null; },
    setItem(key, value) { values.set(key, String(value)); },
  };
}

test('设备标识跨启动稳定且与钥匙串命名约束兼容', async () => {
  globalThis.__APP_VERSION__ = '0.1.0';
  const { getOrCreateDeviceId, buildDeviceRegistration } = await loadModule();
  const storage = createStorage();
  const first = getOrCreateDeviceId(storage);
  const second = getOrCreateDeviceId(storage);

  assert.equal(first, second);
  assert.match(first, /^desktop-[A-Za-z0-9-]+$/);
  assert.equal(buildDeviceRegistration(first).device_id, first);
  assert.equal(buildDeviceRegistration(first).device_name, 'autoLive Desktop');
});

test('超出钥匙串命名上限的旧标识会被替换', async () => {
  const { getOrCreateDeviceId } = await loadModule();
  const storage = createStorage();
  storage.setItem('autolive.desktop.device-id.v1', `desktop-${'x'.repeat(64)}`);

  const deviceId = getOrCreateDeviceId(storage);

  assert.notEqual(deviceId, `desktop-${'x'.repeat(64)}`);
  assert.ok(deviceId.length <= 64);
});
