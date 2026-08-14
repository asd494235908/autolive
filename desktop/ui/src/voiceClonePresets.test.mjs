import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

class MemoryStorage {
  #store = new Map();

  get length() {
    return this.#store.size;
  }

  clear() {
    this.#store.clear();
  }

  getItem(key) {
    return this.#store.has(key) ? this.#store.get(key) : null;
  }

  key(index) {
    return Array.from(this.#store.keys())[index] ?? null;
  }

  removeItem(key) {
    this.#store.delete(key);
  }

  setItem(key, value) {
    this.#store.set(key, String(value));
  }
}

async function loadTypeScriptModule(fileName, exports) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

test('loads invalid storage as an empty preset list', async () => {
  const storage = new MemoryStorage();
  storage.setItem('autolive.voice-clone-presets.v1', '{invalid');
  const { loadVoiceClonePresets } = await loadTypeScriptModule('voiceClonePresets.ts', [
    'loadVoiceClonePresets',
  ]);

  assert.deepEqual(loadVoiceClonePresets(storage), []);
});

test('adds and updates at most ten presets', async () => {
  const storage = new MemoryStorage();
  const { addVoiceClonePreset, loadVoiceClonePresets, updateVoiceClonePreset } = await loadTypeScriptModule(
    'voiceClonePresets.ts',
    ['addVoiceClonePreset', 'loadVoiceClonePresets', 'updateVoiceClonePreset'],
  );

  let presets = [];
  for (let index = 0; index < 10; index += 1) {
    presets = addVoiceClonePreset(
      storage,
      presets,
      { title: `预制 ${index + 1}`, text: `第 ${index + 1} 条固定话术` },
      `2026-08-14T12:00:${String(index).padStart(2, '0')}.000Z`,
    );
  }

  assert.equal(presets.length, 10);
  assert.equal(loadVoiceClonePresets(storage).length, 10);
  assert.throws(
    () =>
      addVoiceClonePreset(storage, presets, {
        title: '第 11 条',
        text: '这条不应该被保存',
      }),
    /最多保存 10 条/i,
  );

  const updated = updateVoiceClonePreset(
    storage,
    presets,
    {
      id: presets[0].id,
      title: '更新后的标题',
      text: '更新后的固定话术',
    },
    '2026-08-14T12:10:00.000Z',
  );

  assert.equal(updated.length, 10);
  assert.equal(updated[0].title, '更新后的标题');
  assert.equal(updated[0].text, '更新后的固定话术');
  assert.equal(updated[0].createdAt, presets[0].createdAt);
  assert.equal(updated[0].updatedAt, '2026-08-14T12:10:00.000Z');
});

test('rejects blank titles and text over 500 unicode characters', async () => {
  const storage = new MemoryStorage();
  const { addVoiceClonePreset } = await loadTypeScriptModule('voiceClonePresets.ts', ['addVoiceClonePreset']);
  const tooLongText = '你'.repeat(501);

  assert.throws(
    () => addVoiceClonePreset(storage, [], { title: '   ', text: '有效文本' }),
    /标题不能为空/i,
  );
  assert.throws(
    () => addVoiceClonePreset(storage, [], { title: '有效标题', text: '   ' }),
    /文本不能为空/i,
  );
  assert.throws(
    () => addVoiceClonePreset(storage, [], { title: '有效标题', text: tooLongText }),
    /文本最多 500 个字符/i,
  );
});

test('removes a preset without mutating the original list', async () => {
  const storage = new MemoryStorage();
  const { addVoiceClonePreset, removeVoiceClonePreset } = await loadTypeScriptModule('voiceClonePresets.ts', [
    'addVoiceClonePreset',
    'removeVoiceClonePreset',
  ]);

  const original = addVoiceClonePreset(
    storage,
    [],
    { title: '保留项', text: '第一条' },
    '2026-08-14T12:00:00.000Z',
  );
  const withSecond = addVoiceClonePreset(
    storage,
    original,
    { title: '删除项', text: '第二条' },
    '2026-08-14T12:01:00.000Z',
  );
  const removed = removeVoiceClonePreset(storage, withSecond, withSecond[1].id);

  assert.equal(withSecond.length, 2);
  assert.equal(removed.length, 1);
  assert.equal(removed[0].id, withSecond[0].id);
  assert.notEqual(removed, withSecond);
});
