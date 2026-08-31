import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import * as ts from 'typescript';

const source = fs.readFileSync(
  new URL('../src/processing-switch-single-flight.ts', import.meta.url),
  'utf8',
);
const javascript = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2021,
  },
}).outputText;
const singleFlight = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(javascript)}`
);

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((next, fail) => {
    resolve = next;
    reject = fail;
  });
  return { promise, resolve, reject };
}

test('处理开关在首个请求完成前只执行一次并在成功后解锁', async () => {
  const gate = singleFlight.createProcessingSwitchSingleFlight();
  const pending = deferred();
  const busy = [];
  let calls = 0;
  const first = singleFlight.runProcessingSwitchSingleFlight(gate, (value) => busy.push(value), async () => {
    calls += 1;
    return pending.promise;
  });
  const second = await singleFlight.runProcessingSwitchSingleFlight(gate, () => {}, async () => {
    calls += 1;
  });

  assert.equal(second, undefined);
  assert.equal(calls, 1);
  assert.deepEqual(busy, [true]);
  pending.resolve('ok');
  assert.equal(await first, 'ok');
  assert.equal(gate.busy, false);
  assert.deepEqual(busy, [true, false]);
});

test('处理开关请求失败后也会解锁', async () => {
  const gate = singleFlight.createProcessingSwitchSingleFlight();
  const pending = deferred();
  const busy = [];
  const result = singleFlight.runProcessingSwitchSingleFlight(
    gate,
    (value) => busy.push(value),
    () => pending.promise,
  );
  pending.reject(new Error('expected'));

  await assert.rejects(result, /expected/);
  assert.equal(gate.busy, false);
  assert.deepEqual(busy, [true, false]);
});
