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

  const pending = { component: 'voice', token: 4 };
  const ready = { state: 'ready', component: 'voice' };
  const resolved = resolvePendingRuntimeAction(pending, ready, 4, 4);
  assert.equal(resolved.shouldResume, true);
  assert.equal(resolved.pending, null);
  assert.equal(resolvePendingRuntimeAction(resolved.pending, ready, 4, 4).shouldResume, false);
  assert.equal(resolvePendingRuntimeAction(pending, ready, 5, 4).shouldResume, false);
  assert.equal(resolvePendingRuntimeAction(pending, { ...ready, component: 'media' }, 4, 4).shouldResume, false);
  assert.equal(runtimeResourceEnsureDecision('voice', ready), 'resume');
  assert.equal(runtimeResourceEnsureDecision('media', ready), 'conflict');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'checking', component: null }), 'conflict');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'checking', component: 'media' }), 'wait');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'downloading', component: 'voice' }), 'conflict');
  assert.equal(runtimeResourceEnsureDecision('media', { state: 'failed', component: 'voice' }), 'install');
});

test('全局清理用 media 查询直到清理终态，并只在成功清理后清空 capabilities', async () => {
  const {
    resolveRuntimeResourceClearLifecycle,
    runtimeResourcePollComponent,
  } = await loadRuntimeResources();

  for (const state of ['checking', 'downloading', 'verifying']) {
    assert.equal(runtimeResourcePollComponent({ state, component: null }, true), 'media');
    assert.equal(runtimeResourcePollComponent({ state, component: null }, false), 'media');
  }
  assert.equal(runtimeResourcePollComponent({ state: 'downloading', component: 'voice' }, false), 'voice');

  const checking = resolveRuntimeResourceClearLifecycle(true, { state: 'checking' });
  assert.deepEqual(checking, { inFlight: true, clearCapabilities: false, conflict: false });
  const completed = resolveRuntimeResourceClearLifecycle(true, { state: 'not-installed' });
  assert.deepEqual(completed, { inFlight: false, clearCapabilities: true, conflict: false });
  for (const state of ['failed', 'cancelled']) {
    assert.deepEqual(
      resolveRuntimeResourceClearLifecycle(true, { state }),
      { inFlight: false, clearCapabilities: false, conflict: false },
    );
    assert.equal(runtimeResourcePollComponent({ state, component: null }, true), null);
  }
  assert.deepEqual(
    resolveRuntimeResourceClearLifecycle(true, { state: 'ready' }),
    { inFlight: false, clearCapabilities: false, conflict: true },
  );
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
    isVoiceRuntimeResourceReady,
    isRuntimeResourceConflict,
    runtimeResourceComponentDescription,
    runtimeResourceComponentLabel,
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
  assert.equal(runtimeResourceComponentLabel(null), '全部运行资源');
  assert.match(runtimeResourceComponentDescription(null), /FFmpeg/);
  assert.match(runtimeResourceComponentDescription(null), /Worker/);
  assert.match(runtimeResourceComponentDescription(null), /模型/);
  assert.match(runtimeResourceMessage({ state: 'checking', component: null }), /全部运行资源/);
  assert.equal(isVoiceRuntimeResourceReady({ state: 'ready', component: 'voice' }, false, false), true);
  assert.equal(isVoiceRuntimeResourceReady({ state: 'ready', component: 'media' }, true, false), true);
  assert.equal(isVoiceRuntimeResourceReady({ state: 'ready', component: 'media' }, false, true), true);
  assert.equal(isVoiceRuntimeResourceReady({ state: 'ready', component: 'media' }, false, false), false);
});

test('清理资源时覆盖所有本地资源消费者', async () => {
  const { runtimeResourceConsumerBusyReason } = await loadRuntimeResources();
  const idle = {
    importVideoBusy: false,
    mediaProcessingBusy: false,
    researchRunning: false,
    researchActionBusy: false,
    voiceCloneActionBusy: false,
    voiceCloneModelLoading: false,
    preGenerationGenerating: false,
    voiceClonePreparing: false,
    voiceCloneGenerating: false,
    voiceClonePlaybackPreparing: false,
    realtimeAudioBusy: false,
  };

  assert.equal(runtimeResourceConsumerBusyReason(idle), null);
  for (const key of Object.keys(idle)) {
    assert.match(runtimeResourceConsumerBusyReason({ ...idle, [key]: true }), /正在使用运行资源/);
  }
});
