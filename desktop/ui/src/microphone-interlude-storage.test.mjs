import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const typescript = require('../node_modules/typescript');

async function loadStorageModule() {
  const source = await readFile(path.join(currentDir, 'microphone-interlude-storage.ts'), 'utf8');
  const transpiled = typescript.transpileModule(source, {
    compilerOptions: { module: typescript.ModuleKind.ESNext, target: typescript.ScriptTarget.ES2022 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(transpiled)}#${Date.now()}`);
}

function createStorage() {
  const values = new Map();
  return {
    getItem(key) { return values.get(key) ?? null; },
    setItem(key, value) { values.set(key, value); },
    dump() { return new Map(values); },
  };
}

const validConfig = {
  version: 1,
  device_id: 'pa-input-mme-0123456789abcdef',
  sensitivity: 'high',
  aec_enabled: true,
  noise_suppression_enabled: false,
  agc_enabled: true,
};

test('麦克风配置使用独立版本化键并只保存设备与清理档位', async () => {
  const { MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY, saveMicrophoneInterludeConfig } = await loadStorageModule();
  const storage = createStorage();

  saveMicrophoneInterludeConfig(validConfig, storage);

  assert.equal(MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY, 'autolive.microphone-interlude-config.v1');
  const saved = JSON.parse(storage.dump().get(MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY));
  assert.deepEqual(saved, validConfig);
  assert.equal('listening' in saved, false);
  assert.equal('armed' in saved, false);
  assert.equal('speaking' in saved, false);
});

test('麦克风配置保存时剥离运行时状态字段', async () => {
  const { MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY, saveMicrophoneInterludeConfig } = await loadStorageModule();
  const storage = createStorage();

  saveMicrophoneInterludeConfig({ ...validConfig, armed: true, listening: true, speaking: true }, storage);

  const saved = JSON.parse(storage.dump().get(MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY));
  assert.deepEqual(saved, validConfig);
});

test('麦克风配置可恢复且缺少保存值时返回默认未监听配置', async () => {
  const { DEFAULT_MICROPHONE_INTERLUDE_CONFIG, loadMicrophoneInterludeConfig } = await loadStorageModule();
  const loaded = loadMicrophoneInterludeConfig(createStorage());

  assert.deepEqual(loaded.config, DEFAULT_MICROPHONE_INTERLUDE_CONFIG);
  assert.equal(loaded.error, null);
  assert.equal(loaded.recovered, false);
  assert.equal('listening' in loaded.config, false);
});

test('麦克风配置存储不可用时返回默认并提示可恢复错误', async () => {
  const { DEFAULT_MICROPHONE_INTERLUDE_CONFIG, loadMicrophoneInterludeConfig } = await loadStorageModule();
  const loaded = loadMicrophoneInterludeConfig(null);

  assert.deepEqual(loaded.config, DEFAULT_MICROPHONE_INTERLUDE_CONFIG);
  assert.equal(loaded.recovered, true);
  assert.match(loaded.error, /本地存储不可用/);
});

test('麦克风配置损坏或版本未知时恢复默认并返回可恢复错误', async () => {
  const { DEFAULT_MICROPHONE_INTERLUDE_CONFIG, MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY, loadMicrophoneInterludeConfig } = await loadStorageModule();
  for (const raw of ['{bad json', JSON.stringify({ ...validConfig, version: 2 })]) {
    const storage = createStorage();
    storage.setItem(MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY, raw);

    const loaded = loadMicrophoneInterludeConfig(storage);
    assert.deepEqual(loaded.config, DEFAULT_MICROPHONE_INTERLUDE_CONFIG);
    assert.equal(loaded.recovered, true);
    assert.match(loaded.error, /恢复默认设置/);
  }
});

test('麦克风配置拒绝非法设备 ID、灵敏度和清理开关', async () => {
  const { saveMicrophoneInterludeConfig } = await loadStorageModule();
  const storage = createStorage();

  assert.throws(() => saveMicrophoneInterludeConfig({ ...validConfig, device_id: 'x'.repeat(257) }, storage));
  assert.throws(() => saveMicrophoneInterludeConfig({ ...validConfig, sensitivity: 'unknown' }, storage));
  assert.throws(() => saveMicrophoneInterludeConfig({ ...validConfig, agc_enabled: 'yes' }, storage));
});

test('App 集成麦克风配置存储且不保存监听状态', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /loadMicrophoneInterludeConfig/);
  assert.match(appSource, /saveMicrophoneInterludeConfig/);
  assert.match(appSource, /microphoneAecEnabled/);
  assert.match(appSource, /microphoneNoiseSuppressionEnabled/);
  assert.match(appSource, /microphoneAgcEnabled/);
  const saveCall = appSource.match(/saveMicrophoneInterludeConfig\(\{[\s\S]*?\}\)/)?.[0];
  assert.ok(saveCall, 'App 应通过明确的配置对象调用麦克风存储');
  assert.doesNotMatch(saveCall, /listening|speaking|armed/);
});
