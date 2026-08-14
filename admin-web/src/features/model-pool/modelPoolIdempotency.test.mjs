import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./modelPoolIdempotency.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 }
}).outputText;
const { createModelPoolIdempotencyKeyManager } = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`
);

const formValues = {
  provider: 'openai-compatible',
  model: 'rewrite-model',
  api_key: 'secret',
  priority: 10,
  daily_limit: 1000,
  concurrency_limit: 2
};

test('same form fingerprint reuses one idempotency key', () => {
  let calls = 0;
  const manager = createModelPoolIdempotencyKeyManager(() => `key-${++calls}`);

  assert.equal(manager.getKey(formValues), 'key-1');
  assert.equal(manager.getKey({ ...formValues }), 'key-1');
  assert.equal(calls, 1);
});

test('changed form fingerprint gets a new idempotency key', () => {
  let calls = 0;
  const manager = createModelPoolIdempotencyKeyManager(() => `key-${++calls}`);

  assert.equal(manager.getKey(formValues), 'key-1');
  assert.equal(manager.getKey({ ...formValues, model: 'another-model' }), 'key-2');
  assert.equal(calls, 2);
});

test('successful submission clears the idempotency key', () => {
  let calls = 0;
  const manager = createModelPoolIdempotencyKeyManager(() => `key-${++calls}`);

  assert.equal(manager.getKey(formValues), 'key-1');
  manager.clear();

  assert.equal(manager.getKey(formValues), 'key-2');
});

test('cancelled submission clears the idempotency key', () => {
  let calls = 0;
  const manager = createModelPoolIdempotencyKeyManager(() => `key-${++calls}`);

  assert.equal(manager.getKey(formValues), 'key-1');
  manager.clear();

  assert.equal(manager.getKey(formValues), 'key-2');
});
