import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

class MemoryStorage {
  #store = new Map();

  getItem(key) {
    return this.#store.has(key) ? this.#store.get(key) : null;
  }

  removeItem(key) {
    this.#store.delete(key);
  }

  setItem(key, value) {
    this.#store.set(key, String(value));
  }
}

async function loadModule(exports) {
  const source = await readFile(new URL('./fixedSpeechPresets.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

test('旧 voice clone 预制文本迁移到 fixed speech key', async () => {
  const storage = new MemoryStorage();
  const legacyPreset = [{
    id: 'legacy-1',
    title: '欢迎语',
    text: '欢迎来到直播间',
    createdAt: '2026-08-14T12:00:00.000Z',
    updatedAt: '2026-08-14T12:00:00.000Z',
  }];
  storage.setItem('autolive.voice-clone-presets.v1', JSON.stringify(legacyPreset));
  const { loadFixedSpeechPresets } = await loadModule(['loadFixedSpeechPresets']);

  assert.deepEqual(loadFixedSpeechPresets(storage), legacyPreset);
  assert.equal(storage.getItem('autolive.voice-clone-presets.v1'), null);
  assert.deepEqual(JSON.parse(storage.getItem('autolive.fixed-speech-presets.v1')), legacyPreset);
});

test('新 key 优先且无效数据安全返回空列表', async () => {
  const storage = new MemoryStorage();
  storage.setItem('autolive.fixed-speech-presets.v1', '{invalid');
  storage.setItem('autolive.voice-clone-presets.v1', JSON.stringify([{
    id: 'legacy-1',
    title: '旧数据',
    text: '不应回退',
    createdAt: '2026-08-14T12:00:00.000Z',
    updatedAt: '2026-08-14T12:00:00.000Z',
  }]));
  const { loadFixedSpeechPresets } = await loadModule(['loadFixedSpeechPresets']);

  assert.deepEqual(loadFixedSpeechPresets(storage), []);
});

test('预制文本支持增改删且最多十条', async () => {
  const storage = new MemoryStorage();
  const {
    addFixedSpeechPreset,
    removeFixedSpeechPreset,
    updateFixedSpeechPreset,
  } = await loadModule([
    'addFixedSpeechPreset',
    'removeFixedSpeechPreset',
    'updateFixedSpeechPreset',
  ]);

  let presets = [];
  for (let index = 0; index < 10; index += 1) {
    presets = addFixedSpeechPreset(
      storage,
      presets,
      { title: `预制 ${index + 1}`, text: `第 ${index + 1} 条话术` },
      `2026-08-14T12:00:${String(index).padStart(2, '0')}.000Z`,
    );
  }
  assert.throws(
    () => addFixedSpeechPreset(storage, presets, { title: '第 11 条', text: '超限' }),
    /最多保存 10 条/,
  );

  const updated = updateFixedSpeechPreset(storage, presets, {
    id: presets[0].id,
    title: '更新标题',
    text: '更新话术',
  });
  const removed = removeFixedSpeechPreset(storage, updated, updated[1].id);
  assert.equal(updated[0].text, '更新话术');
  assert.equal(removed.length, 9);
  assert.equal(updated.length, 10);
});

test('预制文本拒绝空标题、空文本和 Unicode 超限', async () => {
  const storage = new MemoryStorage();
  const { addFixedSpeechPreset } = await loadModule(['addFixedSpeechPreset']);

  assert.throws(() => addFixedSpeechPreset(storage, [], { title: ' ', text: '有效' }), /标题不能为空/);
  assert.throws(() => addFixedSpeechPreset(storage, [], { title: '有效', text: ' ' }), /文本不能为空/);
  assert.throws(
    () => addFixedSpeechPreset(storage, [], { title: '有效', text: '😀'.repeat(501) }),
    /文本最多 500 个字符/,
  );
});
