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
    runtimeResourceEnsureDecision,
  } = await loadRuntimeResources();

  assert.equal(canResumeRuntimeAction({ state: 'ready' }), true);
  assert.equal(canResumeRuntimeAction({ state: 'downloading' }), false);
  assert.equal(canResumeRuntimeAction({ state: 'failed' }), false);
  assert.equal(isCurrentRuntimeResourceAction(3, 3), true);
  assert.equal(isCurrentRuntimeResourceAction(4, 3), false);
  assert.equal(isCurrentRuntimeResourceAction(4, undefined), true);

  const pending = { component: 'media', token: 4 };
  const ready = { state: 'ready', component: 'media' };
  const resolved = resolvePendingRuntimeAction(pending, ready, 4, 4);
  assert.equal(resolved.shouldResume, true);
  assert.equal(resolved.pending, null);
  assert.equal(resolvePendingRuntimeAction(resolved.pending, ready, 4, 4).shouldResume, false);
  assert.equal(resolvePendingRuntimeAction(pending, ready, 5, 4).shouldResume, false);
  assert.equal(runtimeResourceEnsureDecision('media', ready), 'resume');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'checking', component: null }), 'conflict');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'checking', component: 'media' }), 'wait');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'downloading', component: 'media' }), 'wait');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'failed', component: 'media' }), 'install');
});

test('运行资源轮询固定回到 media 组件', async () => {
  const { runtimeResourcePollComponent } = await loadRuntimeResources();
  for (const state of ['checking', 'downloading', 'verifying']) {
    assert.equal(runtimeResourcePollComponent({ state, component: null }), 'media');
  }
  assert.equal(runtimeResourcePollComponent({ state: 'downloading', component: 'media' }), 'media');
  for (const state of ['failed', 'cancelled']) {
    assert.equal(runtimeResourcePollComponent({ state, component: null }), null);
  }
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
