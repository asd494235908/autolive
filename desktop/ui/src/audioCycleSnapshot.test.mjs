import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./audioCycleSnapshot.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const snapshotModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUDIO_CYCLE_SNAPSHOT_STORAGE_KEY,
  appendAudioCycleSnapshot,
  loadAudioCycleSnapshots,
} = snapshotModule;

test('周期快照可写入并读回 localStorage', () => {
  const store = new Map();
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
  };
  appendAudioCycleSnapshot(
    {
      seed: 42,
      presetIds: ['p01', 'p02'],
      weights: [0.5, 0.5],
      values: { input_gain_db: 0.1 },
      at: '2026-08-18T00:00:00.000Z',
    },
    storage,
  );
  assert.ok(store.has(AUDIO_CYCLE_SNAPSHOT_STORAGE_KEY));
  const loaded = loadAudioCycleSnapshots(storage);
  assert.equal(loaded.length, 1);
  assert.equal(loaded[0].seed, 42);
  assert.deepEqual(loaded[0].presetIds, ['p01', 'p02']);
  assert.equal(loaded[0].values.input_gain_db, 0.1);
});
