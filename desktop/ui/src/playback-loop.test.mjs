import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadTypeScriptModule(fileName, exports) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

const { shouldRestartPlayback } = await loadTypeScriptModule('playback-loop.ts', [
  'shouldRestartPlayback',
]);

test('同一个结束事件 token 只允许重启一次', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-1',
      lastRestartToken: null,
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-1',
      lastRestartToken: 'loop-1',
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
    }),
    true,
  );
});

test('接近 duration 但未结束时只在 token 更新后重启', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: 10,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: 10,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-2',
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.9,
      duration: 10,
      restartToken: 'loop-3',
      lastRestartToken: 'loop-2',
    }),
    false,
  );
});

test('旧的 generation 别名仍然兼容', () => {
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 1,
      ended: true,
      lastRestartGeneration: null,
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 1,
      ended: true,
      lastRestartGeneration: 1,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 2,
      ended: false,
      currentTime: 9.95,
      duration: 10,
      lastRestartGeneration: 1,
    }),
    true,
  );
});

test('非法输入不会触发重启', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: null,
      lastRestartToken: null,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: Number.NaN,
      duration: 10,
      restartToken: 'loop-4',
      lastRestartToken: null,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: Number.POSITIVE_INFINITY,
      restartToken: 'loop-4',
      lastRestartToken: null,
    }),
    false,
  );
});
