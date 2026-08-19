import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadScheduler() {
  const source = await readFile(new URL('./startup-scheduler.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('空闲 API 可用时调度首屏后的任务并支持取消', async () => {
  const { scheduleAfterInitialPaint } = await loadScheduler();
  const previousRequest = globalThis.requestIdleCallback;
  const previousCancel = globalThis.cancelIdleCallback;
  let callback;
  let options;
  let cancelledHandle = null;
  globalThis.requestIdleCallback = (next, nextOptions) => {
    callback = next;
    options = nextOptions;
    return 42;
  };
  globalThis.cancelIdleCallback = (handle) => {
    cancelledHandle = handle;
  };

  try {
    let called = 0;
    const cancel = scheduleAfterInitialPaint(() => {
      called += 1;
    });

    assert.equal(options.timeout, 500);
    callback();
    assert.equal(called, 1);
    cancel();
    assert.equal(cancelledHandle, 42);
  } finally {
    if (previousRequest) globalThis.requestIdleCallback = previousRequest;
    else delete globalThis.requestIdleCallback;
    if (previousCancel) globalThis.cancelIdleCallback = previousCancel;
    else delete globalThis.cancelIdleCallback;
  }
});

test('没有空闲 API 时使用可取消的定时器回退', async () => {
  const { scheduleAfterInitialPaint } = await loadScheduler();
  const previousRequest = globalThis.requestIdleCallback;
  const previousCancel = globalThis.cancelIdleCallback;
  delete globalThis.requestIdleCallback;
  delete globalThis.cancelIdleCallback;

  try {
    let called = 0;
    const cancel = scheduleAfterInitialPaint(() => {
      called += 1;
    });
    cancel();
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(called, 0);
  } finally {
    if (previousRequest) globalThis.requestIdleCallback = previousRequest;
    if (previousCancel) globalThis.cancelIdleCallback = previousCancel;
  }
});

test('可取消延迟在时间到达后完成', async () => {
  const { waitForAbortableDelay } = await loadScheduler();
  await waitForAbortableDelay(1);
});

test('AbortSignal 取消可取消延迟并以 AbortError 拒绝', async () => {
  const { waitForAbortableDelay } = await loadScheduler();
  const controller = new AbortController();
  const pending = waitForAbortableDelay(50, controller.signal);

  controller.abort();

  await assert.rejects(pending, (error) => error?.name === 'AbortError');
});
