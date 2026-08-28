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
  const interludeSource = await readFile(path.join(currentDir, 'interlude-player.ts'), 'utf8');
  const interludeTranspiled = typescript.transpileModule(interludeSource, {
    compilerOptions: { module: typescript.ModuleKind.ESNext, target: typescript.ScriptTarget.ES2022 },
  }).outputText;
  const interludeUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(interludeTranspiled)}`;
  const source = await readFile(path.join(currentDir, 'interlude-config-storage.ts'), 'utf8');
  const transpiled = typescript.transpileModule(source, {
    compilerOptions: { module: typescript.ModuleKind.ESNext, target: typescript.ScriptTarget.ES2022 },
  }).outputText.replaceAll("'./interlude-player'", JSON.stringify(interludeUrl));
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
  enabled: true,
  directory: 'C:\\media\\interludes',
  audio_selection_mode: 'random',
  audio_fixed_preset_id: 'p01',
  audio_preset_ids: ['p01', 'p02'],
  audio_mix_enabled: true,
  audio_mix_pick_min: 1,
  audio_mix_pick_max: 2,
  audio_variation_mode: 'periodic',
  audio_variation_period_min_ms: 10_000,
  audio_variation_period_max_ms: 20_000,
  interval_min_ms: 8_000,
  interval_max_ms: 13_000,
  volume_db: -6,
  ducking_depth_db: -60,
  ducking_attack_ms: 50,
  ducking_release_ms: 250,
};

test('随机插话配置保存后可跨重新挂载恢复全部输入', async () => {
  const { loadInterludeConfig, saveInterludeConfig } = await loadStorageModule();
  const storage = createStorage();

  saveInterludeConfig(validConfig, storage);

  assert.deepEqual(loadInterludeConfig(storage), validConfig);
});

test('随机插话配置拒绝损坏数据和越界输入', async () => {
  const { INTERLUDE_CONFIG_STORAGE_KEY, loadInterludeConfig } = await loadStorageModule();
  const storage = createStorage();
  storage.setItem(INTERLUDE_CONFIG_STORAGE_KEY, JSON.stringify({
    ...validConfig,
    interval_min_ms: -1,
  }));

  assert.equal(loadInterludeConfig(storage), null);
});

test('桌面端保存成功后落盘，并在首次快照到达时自动恢复', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );

  assert.match(appSource, /loadInterludeConfig\(\)/);
  assert.match(appSource, /invokePlaybackSnapshot\('set_interlude_config',[\s\S]*savedInterludeConfig/);
  assert.match(saveHandler, /saveInterludeConfigToStorage\(request\)/);
  assert.match(appSource, /persistInterludeVolume[\s\S]*saveInterludeConfigToStorage/);
});
