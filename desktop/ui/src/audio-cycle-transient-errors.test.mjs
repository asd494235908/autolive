import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function loadRetryModule() {
  const source = await readFile(new URL('./audio-cycle-retry.ts', import.meta.url), 'utf8');
  const compiled = (await import('typescript')).transpileModule(source, {
    compilerOptions: {
      module: (await import('typescript')).ModuleKind.ES2022,
      target: (await import('typescript')).ScriptTarget.ES2021,
    },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('声音候选瞬时失败按有界退避自动重试，耗尽后才等待人工重试', async () => {
  const {
    clearAudioCycleRetry,
    createAudioCycleRetryState,
    isAudioCycleRetryReady,
    recordAudioCycleRetryFailure,
  } = await loadRetryModule();
  const identity = '3:8:13';
  let state = createAudioCycleRetryState();
  let nowMs = 10_000;

  for (const delayMs of [1_000, 2_000, 4_000, 8_000, 15_000]) {
    state = recordAudioCycleRetryFailure(state, identity, nowMs);
    assert.equal(state.exhausted, false);
    assert.equal(isAudioCycleRetryReady(state, identity, nowMs + delayMs - 1), false);
    assert.equal(isAudioCycleRetryReady(state, identity, nowMs + delayMs), true);
    nowMs += delayMs;
  }

  state = recordAudioCycleRetryFailure(state, identity, nowMs);
  assert.equal(state.exhausted, true);
  assert.equal(isAudioCycleRetryReady(state, identity, Number.MAX_SAFE_INTEGER), false);
  assert.deepEqual(clearAudioCycleRetry(), createAudioCycleRetryState());
});

test('成功、人工重试和时钟换代都可立即重建候选', async () => {
  const {
    clearAudioCycleRetry,
    createAudioCycleRetryState,
    isAudioCycleRetryReady,
    recordAudioCycleRetryFailure,
  } = await loadRetryModule();
  const initial = createAudioCycleRetryState();
  assert.equal(isAudioCycleRetryReady(initial, '1:1:1', 0), true);

  const failed = recordAudioCycleRetryFailure(initial, '1:1:1', 100);
  assert.equal(failed.failureCount, 1);
  assert.equal(isAudioCycleRetryReady(failed, '1:1:1', 1_099), false);
  assert.equal(isAudioCycleRetryReady(failed, '2:1:1', 100), true);

  assert.equal(isAudioCycleRetryReady(clearAudioCycleRetry(), '1:1:1', 100), true);
});

test('视频 prepare 可复用通用有界退避，连续第六次失败后耗尽', async () => {
  const {
    createCycleRetryState,
    isCycleRetryReady,
    recordCycleRetryFailure,
  } = await loadRetryModule();
  const identity = 'video:7:11:plan-1';
  let state = createCycleRetryState();
  let nowMs = 1_000;

  for (const delayMs of [1_000, 2_000, 4_000, 8_000, 15_000]) {
    state = recordCycleRetryFailure(state, identity, nowMs);
    assert.equal(state.exhausted, false);
    assert.equal(isCycleRetryReady(state, identity, nowMs + delayMs - 1), false);
    nowMs += delayMs;
  }

  state = recordCycleRetryFailure(state, identity, nowMs);
  assert.equal(state.exhausted, true);
  assert.equal(state.retryAtMs, null);
});

test('播放中修改声音预设不会让旧候选失去 owner 并锁死新周期', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const clearStart = app.indexOf('function clearAudioFutureMediaCyclePlans()');
  const clearBodyStart = app.indexOf('{', clearStart) + 1;
  const clearBodyEnd = app.indexOf('\n  }\n', clearBodyStart);
  assert.ok(clearStart >= 0 && clearBodyStart > clearStart && clearBodyEnd > clearBodyStart);

  const audioFuturePlansRef = { current: ['old-plan'] };
  const activeAudioRenderRef = { current: null };
  const audioCycleRetryRef = { current: { failureCount: 0, exhausted: false } };
  const clearAudioCycleRetry = () => ({ failureCount: 0, exhausted: false });
  const invalidateSettings = new Function(
    'audioFuturePlansRef',
    'activeAudioRenderRef',
    'audioCycleRetryRef',
    'clearAudioCycleRetry',
    app.slice(clearBodyStart, clearBodyEnd),
  ).bind(
    null,
    audioFuturePlansRef,
    activeAudioRenderRef,
    audioCycleRetryRef,
    clearAudioCycleRetry,
  );

  let resolveOldPrepare;
  const oldPrepareGate = new Promise((resolve) => {
    resolveOldPrepare = resolve;
  });
  let prepareInFlight = false;
  let rustPendingCandidate = null;
  let cycle = 26;
  const preparedPresetIds = [];

  async function prepare(candidate, gate = Promise.resolve()) {
    if (prepareInFlight || activeAudioRenderRef.current) return false;
    preparedPresetIds.push(candidate.presetIds);
    if (rustPendingCandidate) {
      audioCycleRetryRef.current.failureCount += 1;
      return false;
    }
    activeAudioRenderRef.current = candidate;
    prepareInFlight = true;
    await gate;
    rustPendingCandidate = candidate;
    prepareInFlight = false;
    return true;
  }

  function settlePreparedCandidate() {
    if (activeAudioRenderRef.current !== rustPendingCandidate) return false;
    activeAudioRenderRef.current = null;
    rustPendingCandidate = null;
    return true;
  }

  const oldCandidate = { presetIds: ['p01'] };
  const latestCandidate = { presetIds: ['p01', 'p22'] };
  const oldPrepare = prepare(oldCandidate, oldPrepareGate);
  invalidateSettings();
  await prepare(latestCandidate);
  assert.deepEqual(preparedPresetIds, [['p01']], '旧 prepare 未完成时不得并发准备新预设');

  resolveOldPrepare();
  await oldPrepare;
  assert.equal(settlePreparedCandidate(), true, '设置变更不得让旧 Rust pending 失去前端 owner');
  assert.equal(await prepare(latestCandidate), true);
  assert.equal(settlePreparedCandidate(), true);
  cycle += 1;

  assert.deepEqual(preparedPresetIds, [['p01'], ['p01', 'p22']]);
  assert.equal(audioCycleRetryRef.current.failureCount, 0);
  assert.equal(audioCycleRetryRef.current.exhausted, false);
  assert.equal(cycle, 27);
});

test('声音候选过期只推进声音时间轴，non-busy 终态失败进入有界自动重试', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const audioPrepare = app.slice(
    app.indexOf('async function prepareNextAudioMediaCandidate('),
    app.indexOf('function prepareNextVideoMediaCandidate('),
  );
  const audioCommit = app.slice(
    app.indexOf('function commitCompletedAudioRender('),
    app.indexOf('function commitCompletedVideoRender('),
  );
  assert.match(
    audioPrepare,
    /if \(isMediaWorkerBusyError\(cause\)\) return;[\s\S]*scheduleAudioCycleRetry/,
  );
  assert.doesNotMatch(audioPrepare, /else \{\s*audioCycleRetryRef\.current = clearAudioCycleRetry/);
  assert.match(audioCommit, /候选声音已超过有效媒体时间窗口[\s\S]*advanceIndependentAudioQueue/);
  assert.doesNotMatch(audioCommit, /疑似静音|audioSilentRetryCountRef/);
  assert.match(audioCommit, /isMediaProcessingPlaybackActive[\s\S]*scheduleAudioCycleRetry/);
  assert.match(audioCommit, /pending_audio_artifact_reference[\s\S]*clearAudioCycleRetry/);
  assert.doesNotMatch(audioCommit, /advanceIndependentVideoQueue/);
});

test('人工重试保留另一媒体域的队列，并且只在播放时钟有效时执行', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const audioRetry = app.slice(
    app.indexOf('function retryAudioProcessing('),
    app.indexOf('function retryVideoProcessing('),
  );
  const videoRetry = app.slice(
    app.indexOf('function retryVideoProcessing('),
    app.indexOf('useEffect(() => {', app.indexOf('function retryVideoProcessing(')),
  );

  assert.match(audioRetry, /if \(!audioPeriodActive\) return/);
  assert.match(audioRetry, /wakeMediaCycleScheduling\('retry'\)/);
  assert.match(videoRetry, /if \(!videoStreamActive\) return/);
  assert.match(videoRetry, /realtimeVideoCycleConfigurationKeyRef\.current = null/);
  assert.match(videoRetry, /setVideoCycleConfigureRevision/);
});

test('暂停与不健康 WebView 时钟只冻结音频候选，视频无前端重试队列', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const activeGate = app.slice(
    app.indexOf('function isMediaProcessingPlaybackActive('),
    app.indexOf('function initializeFutureAudioCyclePlans('),
  );
  const audioPrepare = app.slice(
    app.indexOf('async function prepareAudioMediaCandidate('),
    app.indexOf('function applyPlannedAudioCycle('),
  );
  const playbackAction = app.slice(
    app.indexOf('async function runPlaybackAction('),
    app.indexOf('async function startPlaybackFromHome('),
  );
  const videoConfigure = app.slice(
    app.indexOf('// WebView 时钟只约束声音周期；视频周期由 Rust/mpv 时钟独立推进。'),
    app.indexOf('if (!AUTO_PORTAUDIO_ENABLED', app.indexOf("invoke<unknown>('configure_realtime_video_cycle'")),
  );
  assert.match(playbackAction, /action === 'stop'[\s\S]*clearFutureMediaCyclePlans\(\)/);
  assert.match(activeGate, /playback_state\?\.toLowerCase\(\) === 'playing'/);
  assert.match(activeGate, /clock\.playback_generation === playbackGeneration[\s\S]*!clock\.paused[\s\S]*clock\.clock_health === 'healthy'/);
  assert.match(audioPrepare, /await invokePlaybackSnapshot\('prepare_audio_media_candidate'[\s\S]*if \(!isMediaProcessingPlaybackActive\(candidate\.playbackGeneration\)\) return/);
  assert.doesNotMatch(app, /videoCycleRetryRef|scheduleVideoPrepareRetry|pendingMediaApplyRef/);
  assert.match(videoConfigure, /invoke<unknown>\('configure_realtime_video_cycle'/);
  assert.doesNotMatch(videoConfigure, /clock_health|isMediaProcessingPlaybackActive|isVideoCandidatePlaybackActive/);
});
