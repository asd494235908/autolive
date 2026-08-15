import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadRuntimeResources() {
  const source = await readFile(new URL('./runtimeResources.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('只有 ready 才允许恢复挂起动作', async () => {
  const {
    canResumeRuntimeAction,
    isCurrentRuntimeResourceAction,
    resolvePendingRuntimeAction,
  } = await loadRuntimeResources();

  assert.equal(canResumeRuntimeAction({ state: 'ready' }), true);
  assert.equal(canResumeRuntimeAction({ state: 'downloading' }), false);
  assert.equal(canResumeRuntimeAction({ state: 'failed' }), false);
  assert.equal(isCurrentRuntimeResourceAction(3, 3), true);
  assert.equal(isCurrentRuntimeResourceAction(4, 3), false);
  assert.equal(isCurrentRuntimeResourceAction(4, undefined), true);

  const pending = { component: 'voice', token: 4 };
  const ready = { state: 'ready', component: 'voice' };
  const resolved = resolvePendingRuntimeAction(pending, ready, 4, 4);
  assert.equal(resolved.shouldResume, true);
  assert.equal(resolved.pending, null);
  assert.equal(resolvePendingRuntimeAction(resolved.pending, ready, 4, 4).shouldResume, false);
  assert.equal(resolvePendingRuntimeAction(pending, ready, 5, 4).shouldResume, false);
  assert.equal(resolvePendingRuntimeAction(pending, { ...ready, component: 'media' }, 4, 4).shouldResume, false);
});

test('只有检查、下载和校验状态需要轮询并占用资源操作', async () => {
  const { isRuntimeResourceBusy, shouldPollRuntimeResources } = await loadRuntimeResources();

  for (const state of ['checking', 'downloading', 'verifying']) {
    assert.equal(isRuntimeResourceBusy({ state }), true);
    assert.equal(shouldPollRuntimeResources({ state }), true);
  }
  for (const state of ['not-installed', 'ready', 'failed', 'cancelled']) {
    assert.equal(isRuntimeResourceBusy({ state }), false);
    assert.equal(shouldPollRuntimeResources({ state }), false);
  }
});

test('资源进度处理空值、越界和已完成状态', async () => {
  const { runtimeResourcePercent } = await loadRuntimeResources();

  assert.equal(runtimeResourcePercent({ state: 'not-installed', downloaded_bytes: 0, total_bytes: 0 }), 0);
  assert.equal(runtimeResourcePercent({ state: 'downloading', downloaded_bytes: 25, total_bytes: 100 }), 25);
  assert.equal(runtimeResourcePercent({ state: 'downloading', downloaded_bytes: 120, total_bytes: 100 }), 100);
  assert.equal(runtimeResourcePercent({ state: 'ready', downloaded_bytes: 0, total_bytes: 0 }), 100);
});

test('资源消息包含组件、失败原因和目录信息', async () => {
  const {
    isRuntimeResourceConflict,
    runtimeResourceMessage,
    runtimeResourceProgressDetails,
  } = await loadRuntimeResources();

  assert.match(runtimeResourceMessage({ state: 'downloading', component: 'media', current_file: 'ffmpeg' }), /媒体资源/);
  assert.match(runtimeResourceMessage({ state: 'failed', component: 'voice', error: '校验失败' }), /校验失败/);
  assert.match(runtimeResourceMessage({ state: 'ready', component: 'voice', resource_root: '/data/runtime', installed_bytes: 1024 }), /\/data\/runtime/);
  assert.equal(isRuntimeResourceConflict('media', { state: 'downloading', component: 'voice' }), true);
  assert.equal(isRuntimeResourceConflict('voice', { state: 'downloading', component: 'voice' }), false);
  assert.match(runtimeResourceProgressDetails({ downloaded_bytes: 25, total_bytes: 100, bytes_per_second: 5 }), /25 B \/ 100 B/);
  assert.match(runtimeResourceProgressDetails({ downloaded_bytes: 25, total_bytes: 100, bytes_per_second: 5 }), /剩余 75 B/);
});
