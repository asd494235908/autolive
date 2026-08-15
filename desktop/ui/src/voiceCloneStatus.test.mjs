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

test('preparation does not block on a source MP4 hash', async () => {
  const { getVoiceClonePrepareDisabledReason, getVoiceCloneIdleNotice } = await loadTypeScriptModule(
    'voiceCloneStatus.ts',
    ['getVoiceClonePrepareDisabledReason', 'getVoiceCloneIdleNotice'],
  );

  assert.equal(
    getVoiceClonePrepareDisabledReason({
      hasSource: true,
      workerAvailable: true,
      workerReason: null,
      status: 'idle',
    }),
    null,
  );
  assert.equal(
    getVoiceCloneIdleNotice(),
    '人声模型和运行环境首次使用时会自动下载，随后直接复用。',
  );
  assert.equal(
    getVoiceCloneIdleNotice(),
    '人声模型和运行环境首次使用时会自动下载，随后直接复用。',
  );
  assert.equal(
    getVoiceClonePrepareDisabledReason({
      hasSource: true,
      workerAvailable: true,
      workerReason: null,
      status: 'idle',
    }),
    null,
  );
});

test('automatic preparation starts once per source generation without an MP4 hash', async () => {
  const {
    getVoiceCloneAutoPrepareKey,
    shouldAutoPrepareVoiceCloneSource,
  } = await loadTypeScriptModule('voiceCloneStatus.ts', [
    'getVoiceCloneAutoPrepareKey',
    'shouldAutoPrepareVoiceCloneSource',
  ]);
  const sourcePath = '/tmp/demo.mp4';
  const sourceKey = `${sourcePath}:g7`;

  assert.equal(getVoiceCloneAutoPrepareKey(sourcePath, 7), sourceKey);
  assert.equal(
    shouldAutoPrepareVoiceCloneSource({
      sourcePath,
      playbackGeneration: 7,
      status: 'idle',
      triggeredKey: null,
    }),
    true,
  );
  assert.equal(
    shouldAutoPrepareVoiceCloneSource({
      sourcePath,
      playbackGeneration: 7,
      status: 'preparing',
      triggeredKey: null,
    }),
    false,
  );
  assert.equal(
    shouldAutoPrepareVoiceCloneSource({
      sourcePath,
      playbackGeneration: 7,
      status: 'idle',
      triggeredKey: sourceKey,
    }),
    false,
  );
});

test('文件选择取消或报错时仅为未变化的旧视频恢复一次自动准备', async () => {
  const { shouldRestoreVoiceCloneAutoPrepareAfterPicker } = await loadTypeScriptModule(
    'voiceCloneStatus.ts',
    ['shouldRestoreVoiceCloneAutoPrepareAfterPicker'],
  );
  const pendingPicker = {
    interactivePicker: true,
    hadPendingAutoPrepare: true,
    selectedPath: null,
    playbackGenerationBefore: 7,
    playbackGenerationAfter: 7,
  };

  assert.equal(shouldRestoreVoiceCloneAutoPrepareAfterPicker(pendingPicker), true);
  assert.equal(shouldRestoreVoiceCloneAutoPrepareAfterPicker({ ...pendingPicker, selectedPath: '/tmp/new.mp4' }), false);
  assert.equal(shouldRestoreVoiceCloneAutoPrepareAfterPicker({ ...pendingPicker, playbackGenerationAfter: 8 }), false);
  assert.equal(shouldRestoreVoiceCloneAutoPrepareAfterPicker({ ...pendingPicker, hadPendingAutoPrepare: false }), false);
  assert.equal(shouldRestoreVoiceCloneAutoPrepareAfterPicker({ ...pendingPicker, interactivePicker: false }), false);
});

test('replacement is enabled after preparation is ready and the player is synchronized', async () => {
  const { getVoiceCloneReplaceDisabledReason } = await loadTypeScriptModule(
    'voiceCloneStatus.ts',
    ['getVoiceCloneReplaceDisabledReason'],
  );

  assert.equal(
    getVoiceCloneReplaceDisabledReason({
      hasSource: true,
      playbackState: 'playing',
      positionMs: 1_000,
      workerAvailable: true,
      workerReason: null,
      status: 'ready',
      audioProcessingBlocked: false,
      realtimeAudioBusy: false,
      textError: null,
    }),
    null,
  );
  assert.equal(
    getVoiceCloneReplaceDisabledReason({
      hasSource: true,
      playbackState: 'playing',
      positionMs: 1_000,
      workerAvailable: true,
      workerReason: null,
      status: 'idle',
      audioProcessingBlocked: false,
      realtimeAudioBusy: false,
      textError: null,
    }),
    '请先准备人声',
  );
  assert.equal(
    getVoiceCloneReplaceDisabledReason({
      hasSource: true,
      playbackState: 'playing',
      positionMs: 2_000,
      workerAvailable: true,
      workerReason: null,
      status: 'playing',
      audioProcessingBlocked: false,
      realtimeAudioBusy: false,
      textError: null,
    }),
    null,
  );
  assert.equal(
    getVoiceCloneReplaceDisabledReason({
      hasSource: true,
      playbackState: 'playing',
      positionMs: 3_000,
      workerAvailable: true,
      workerReason: null,
      status: 'generating',
      audioProcessingBlocked: false,
      realtimeAudioBusy: false,
      textError: null,
    }),
    '固定话术 Worker 正在执行',
  );
});

test('current text playback keeps original audio while preparing, mutes while playing, and never auto-replays on a loop', async () => {
  const {
    shouldMuteOriginalAudioForVoiceClonePlayback,
    shouldAutoReplayVoiceClonePlaybackOnLoop,
  } = await loadTypeScriptModule('voiceCloneStatus.ts', [
    'shouldMuteOriginalAudioForVoiceClonePlayback',
    'shouldAutoReplayVoiceClonePlaybackOnLoop',
  ]);

  assert.equal(shouldMuteOriginalAudioForVoiceClonePlayback('preparing'), false);
  assert.equal(shouldMuteOriginalAudioForVoiceClonePlayback('playing'), true);
  assert.equal(shouldMuteOriginalAudioForVoiceClonePlayback('ready'), false);
  assert.equal(shouldMuteOriginalAudioForVoiceClonePlayback('failed'), false);
  assert.equal(shouldAutoReplayVoiceClonePlaybackOnLoop(), false);
});

test('model loading is shown only while preparation is loading the local XTTS model', async () => {
  const { isVoiceCloneModelLoading } = await loadTypeScriptModule('voiceCloneStatus.ts', [
    'isVoiceCloneModelLoading',
  ]);

  assert.equal(isVoiceCloneModelLoading('preparing', 'loading-local-model'), true);
  assert.equal(isVoiceCloneModelLoading('preparing', 'separating-voice'), false);
  assert.equal(isVoiceCloneModelLoading('ready', 'loading-local-model'), false);
});

test('pre-generation starts once for a ready source and a new text revision', async () => {
  const { getVoiceClonePreGenerationTriggerKey, shouldStartVoiceClonePreGeneration } =
    await loadTypeScriptModule('voiceCloneStatus.ts', [
      'getVoiceClonePreGenerationTriggerKey',
      'shouldStartVoiceClonePreGeneration',
    ]);
  const key = getVoiceClonePreGenerationTriggerKey(7, 2);
  assert.equal(key, '7:r2');
  assert.equal(shouldStartVoiceClonePreGeneration({
    sourceReady: true,
    sourceGeneration: 7,
    presetCount: 3,
    presetTextRevision: 2,
    batchStatus: 'idle',
    replacementStatus: 'ready',
    playbackStatus: 'ready',
    lastStartedKey: null,
  }), true);
  assert.equal(shouldStartVoiceClonePreGeneration({
    sourceReady: true,
    sourceGeneration: 7,
    presetCount: 3,
    presetTextRevision: 2,
    batchStatus: 'generating',
    replacementStatus: 'ready',
    playbackStatus: 'ready',
    lastStartedKey: null,
  }), false);
  assert.equal(shouldStartVoiceClonePreGeneration({
    sourceReady: true,
    sourceGeneration: 7,
    presetCount: 3,
    presetTextRevision: 2,
    batchStatus: 'ready',
    replacementStatus: 'ready',
    playbackStatus: 'ready',
    lastStartedKey: key,
  }), false);
});

test('批量预生成只在 voice 资源就绪且没有安装或清理时启动', async () => {
  const { canStartVoiceClonePreGenerationForRuntime } = await loadTypeScriptModule(
    'voiceCloneStatus.ts',
    ['canStartVoiceClonePreGenerationForRuntime'],
  );

  assert.equal(canStartVoiceClonePreGenerationForRuntime(true, false, false), true);
  assert.equal(canStartVoiceClonePreGenerationForRuntime(false, false, false), false);
  assert.equal(canStartVoiceClonePreGenerationForRuntime(true, true, false), false);
  assert.equal(canStartVoiceClonePreGenerationForRuntime(true, false, true), false);
});

test('a submitted pre-generation key stays occupied until text revision or source generation changes', async () => {
  const { getVoiceClonePreGenerationTriggerKey, shouldStartVoiceClonePreGeneration } =
    await loadTypeScriptModule('voiceCloneStatus.ts', [
      'getVoiceClonePreGenerationTriggerKey',
      'shouldStartVoiceClonePreGeneration',
    ]);
  const sameRevision = {
    sourceReady: true,
    sourceGeneration: 7,
    presetCount: 3,
    presetTextRevision: 2,
    batchStatus: 'ready',
    replacementStatus: 'ready',
    playbackStatus: 'ready',
    lastStartedKey: getVoiceClonePreGenerationTriggerKey(7, 2),
  };
  const afterTitleChange = { ...sameRevision };
  const afterDelete = { ...sameRevision, presetCount: 2 };

  assert.equal(shouldStartVoiceClonePreGeneration(afterTitleChange), false);
  assert.equal(shouldStartVoiceClonePreGeneration(afterDelete), false);
  assert.equal(shouldStartVoiceClonePreGeneration({
    ...sameRevision,
    sourceGeneration: 8,
    lastStartedKey: null,
  }), true);
});

test('pre-generation retries the first IPC failure but stops after the retry fails', async () => {
  const { getVoiceClonePreGenerationTriggerKey, shouldRetryVoiceClonePreGeneration } =
    await loadTypeScriptModule('voiceCloneStatus.ts', [
      'getVoiceClonePreGenerationTriggerKey',
      'shouldRetryVoiceClonePreGeneration',
    ]);
  const key = getVoiceClonePreGenerationTriggerKey(7, 2);

  assert.equal(shouldRetryVoiceClonePreGeneration({
    failedKey: key,
    currentKey: key,
    lastRetriedKey: null,
  }), true);
  assert.equal(shouldRetryVoiceClonePreGeneration({
    failedKey: key,
    currentKey: key,
    lastRetriedKey: key,
  }), false);
});

test('pre-generation retry budget resets for a new source or text revision and ignores stale failures', async () => {
  const { getVoiceClonePreGenerationTriggerKey, shouldRetryVoiceClonePreGeneration } =
    await loadTypeScriptModule('voiceCloneStatus.ts', [
      'getVoiceClonePreGenerationTriggerKey',
      'shouldRetryVoiceClonePreGeneration',
    ]);
  const exhaustedKey = getVoiceClonePreGenerationTriggerKey(7, 2);
  const nextSourceKey = getVoiceClonePreGenerationTriggerKey(8, 2);
  const nextRevisionKey = getVoiceClonePreGenerationTriggerKey(7, 3);

  assert.equal(shouldRetryVoiceClonePreGeneration({
    failedKey: nextSourceKey,
    currentKey: nextSourceKey,
    lastRetriedKey: exhaustedKey,
  }), true);
  assert.equal(shouldRetryVoiceClonePreGeneration({
    failedKey: nextRevisionKey,
    currentKey: nextRevisionKey,
    lastRetriedKey: exhaustedKey,
  }), true);
  assert.equal(shouldRetryVoiceClonePreGeneration({
    failedKey: exhaustedKey,
    currentKey: nextSourceKey,
    lastRetriedKey: null,
  }), false);
});

test('pre-generation summary reports item progress and failures', async () => {
  const { getVoiceClonePreGenerationSummary } = await loadTypeScriptModule(
    'voiceCloneStatus.ts', ['getVoiceClonePreGenerationSummary']);
  assert.equal(getVoiceClonePreGenerationSummary({ status: 'generating', total: 10, completed: 3, failed: 0 }), '正在准备 3/10');
  assert.equal(getVoiceClonePreGenerationSummary({ status: 'failed', total: 10, completed: 10, failed: 2 }), '8 条完成，2 条失败');
  assert.equal(getVoiceClonePreGenerationSummary({ status: 'ready', total: 4, completed: 4, failed: 0 }), '4 条文案已准备');
});

test('pre-generation accepts results only for a mounted current matching source', async () => {
  const { shouldAcceptVoiceClonePreGenerationResult } = await loadTypeScriptModule(
    'voiceCloneStatus.ts', ['shouldAcceptVoiceClonePreGenerationResult']);
  const currentRequest = {
    cancelled: false,
    mounted: true,
    requestedGeneration: 7,
    currentGeneration: 7,
    resultGeneration: 7,
  };

  assert.equal(shouldAcceptVoiceClonePreGenerationResult(currentRequest), true);
  assert.equal(shouldAcceptVoiceClonePreGenerationResult({ ...currentRequest, cancelled: true }), false);
  assert.equal(shouldAcceptVoiceClonePreGenerationResult({ ...currentRequest, mounted: false }), false);
  assert.equal(shouldAcceptVoiceClonePreGenerationResult({ ...currentRequest, currentGeneration: 8 }), false);
  assert.equal(shouldAcceptVoiceClonePreGenerationResult({ ...currentRequest, resultGeneration: 8 }), false);
});

test('pre-generation blocks worker actions only while generating', async () => {
  const { getVoiceClonePreGenerationBusyReason } = await loadTypeScriptModule(
    'voiceCloneStatus.ts', ['getVoiceClonePreGenerationBusyReason']);

  assert.equal(getVoiceClonePreGenerationBusyReason('generating'), '正在批量准备文案人声');
  assert.equal(getVoiceClonePreGenerationBusyReason('ready'), null);
  assert.equal(getVoiceClonePreGenerationBusyReason('failed'), null);
  assert.equal(getVoiceClonePreGenerationBusyReason('cancelled'), null);
});

test('结构化 Tauri IPC 错误保留具体消息而不是退化成通用提示', async () => {
  const { getDisplayErrorMessage } = await loadTypeScriptModule(
    'errorDisplay.ts', ['getDisplayErrorMessage']);

  assert.equal(
    getDisplayErrorMessage({ code: 'media_probe_failed', message: '读取视频信息超时' }, '导入视频失败'),
    '读取视频信息超时',
  );
  assert.equal(getDisplayErrorMessage(new Error('本地错误'), '导入视频失败'), '本地错误');
  assert.equal(getDisplayErrorMessage({ message: '   ' }, '导入视频失败'), '导入视频失败');
});
