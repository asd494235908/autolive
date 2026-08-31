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

const {
  buildAuthoritativePlaybackClock,
  didSourceMediaLoopWrap,
  isMediaCycleTargetInAuthoritativeLoop,
  resolvePlaybackBoundaryDuration,
  shouldIgnoreLoopBoundaryPause,
  shouldRestartPlayback,
  shouldRestartCurrentSourceImmediately,
} = await loadTypeScriptModule('playback-loop.ts', [
  'buildAuthoritativePlaybackClock',
  'didSourceMediaLoopWrap',
  'isMediaCycleTargetInAuthoritativeLoop',
  'resolvePlaybackBoundaryDuration',
  'shouldIgnoreLoopBoundaryPause',
  'shouldRestartPlayback',
  'shouldRestartCurrentSourceImmediately',
]);

test('绝对播放时钟只使用 Rust 快照循环，不从媒体位置推导循环', () => {
  assert.deepEqual(buildAuthoritativePlaybackClock({
    loopIndex: 0,
    positionMs: 72_200,
    durationMs: 72_300,
  }), {
    loopIndex: 0,
    positionMs: 72_200,
    absolutePositionMs: 72_200,
  });
  assert.deepEqual(buildAuthoritativePlaybackClock({
    loopIndex: 1,
    positionMs: 120,
    durationMs: 72_300,
  }), {
    loopIndex: 1,
    positionMs: 120,
    absolutePositionMs: 72_420,
  });
});

test('只把接近末尾到接近开头识别为 sourceAudio 原生循环回绕', () => {
  assert.equal(didSourceMediaLoopWrap(72_100, 120, 72_300), true);
  assert.equal(didSourceMediaLoopWrap(40_000, 120, 72_300), false);
  assert.equal(didSourceMediaLoopWrap(72_100, 60_000, 72_300), false);
  assert.equal(didSourceMediaLoopWrap(null, 120, 72_300), false);
});

test('跨循环视频目标等待 Rust 进入目标循环后才允许 prepare', () => {
  assert.equal(isMediaCycleTargetInAuthoritativeLoop({
    targetAbsolutePositionMs: 72_299,
    durationMs: 72_300,
    mediaLoopIndex: 0,
    authoritativeLoopIndex: 0,
  }), true);
  assert.equal(isMediaCycleTargetInAuthoritativeLoop({
    targetAbsolutePositionMs: 72_300,
    durationMs: 72_300,
    mediaLoopIndex: 0,
    authoritativeLoopIndex: 0,
  }), false);
  assert.equal(isMediaCycleTargetInAuthoritativeLoop({
    targetAbsolutePositionMs: 72_300,
    durationMs: 72_300,
    mediaLoopIndex: 1,
    authoritativeLoopIndex: 1,
  }), true);
  assert.equal(isMediaCycleTargetInAuthoritativeLoop({
    targetAbsolutePositionMs: 72_300,
    durationMs: 72_300,
    mediaLoopIndex: 2,
    authoritativeLoopIndex: 1,
  }), false);
});

test('视频播放使用媒体元素的有限时长作为播放池边界', () => {
  assert.equal(resolvePlaybackBoundaryDuration({
    mediaDurationSeconds: 42.5,
    sourceDurationMs: 60_000,
  }), 42.5);
});

test('只有单项池可以在后端完成响应前即时重播当前源', () => {
  assert.equal(shouldRestartCurrentSourceImmediately(0), false);
  assert.equal(shouldRestartCurrentSourceImmediately(1), true);
  assert.equal(shouldRestartCurrentSourceImmediately(2), false);
  assert.equal(shouldRestartCurrentSourceImmediately(10), false);
});

test('自然结束产生的 pause 不得暂停后端音频出口', () => {
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: true,
    }),
    true,
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: true,
      ended: false,
    }),
    true,
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: false,
      currentTime: 9.98,
      duration: 10,
    }),
    true,
    'WebView2 先发 pause、后更新 ended 时，也必须识别为自然结束',
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: false,
      currentTime: 9.9,
      duration: 10,
    }),
    false,
  );
});

test('循环 seek 前先占住 pause 事件，且最终效果窗不回写内部 pause', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const restartStart = source.indexOf('function restartCurrentPlayback');
  const restartEnd = source.indexOf('function restartToNextLoop', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);
  const finalEffectStart = source.indexOf('function FinalEffectWindow()');
  const videoStart = source.indexOf('<video', finalEffectStart);
  const finalEffectVideo = source.slice(videoStart, source.indexOf('<audio', videoStart));

  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(finalEffectStart >= 0 && videoStart > finalEffectStart);
  assert.ok(
    restartSource.indexOf('suppressMediaEventRef.current = true')
      < restartSource.indexOf('video.currentTime = 0'),
  );
  assert.match(
    restartSource,
    /video\.play\(\)\.then\(\(\) => \{\s*\/\/[^\n]*\n\s*\/\/[^\n]*\n\s*suppressMediaEventRef\.current = false;/,
  );
  assert.doesNotMatch(finalEffectVideo, /onPause=\{[\s\S]*pause_playback/);
});

test('播放项完成后应用权威快照，换源不预先重播旧源', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const publishStart = source.indexOf('function publishMediaState');
  const publishEnd = source.indexOf('function applyPlaybackMediaControl', publishStart);
  const publishSource = source.slice(publishStart, publishEnd);
  const restartStart = source.indexOf('function restartToNextLoop');
  const restartEnd = source.indexOf('function restartAtBoundary', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);
  const completeLoop = restartSource.indexOf("invoke<unknown>('complete_playback_item'");
  const applySnapshot = restartSource.indexOf('applyPlayerSnapshot(nextSnapshot)', completeLoop);
  const reanchorPortAudio = restartSource.indexOf(
    'syncAudioOutputSourceLatest(false, true)',
    applySnapshot,
  );

  assert.ok(publishStart >= 0 && publishEnd > publishStart);
  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(completeLoop >= 0);
  assert.match(publishSource, /if \(loopSyncPromiseRef\.current !== null\) return;/);
  assert.match(restartSource, /isPlaybackItemCompletionResult\(result\)/);
  assert.match(restartSource, /playback_generation:\s*currentSnapshot\.playback_generation/);
  assert.match(restartSource, /loop_index:\s*currentSnapshot\.loop_index/);
  assert.match(restartSource, /source_media_index:\s*currentSnapshot\.source_media_index/);
  assert.doesNotMatch(restartSource, /loopSequenceRef\.current \+= 1/);
  assert.match(restartSource, /if \(restartImmediately\) \{\s*if \(!sourceAlreadyLooped\) \{\s*restartCurrentPlayback/);
  assert.doesNotMatch(restartSource, /clockEpochRef\.current \+= 1/);
  const cancelBeforeCompletion = restartSource.indexOf('cancelFixedSpeech');
  const clearBeforeCompletion = restartSource.indexOf('clearInterludePlayback');
  assert.ok(cancelBeforeCompletion >= 0 && cancelBeforeCompletion < completeLoop);
  assert.ok(clearBeforeCompletion > cancelBeforeCompletion && clearBeforeCompletion < completeLoop);
  assert.ok(applySnapshot > completeLoop);
  assert.ok(
    reanchorPortAudio > applySnapshot,
    '循环提交后必须用边界优先级同步新轮次的 PortAudio 源，不能被普通 N+1 候选挡住',
  );
  assert.match(
    restartSource,
    /if \(loopSyncPromiseRef\.current === completion\) \{\s*loopSyncPromiseRef\.current = null;\s*publishMediaState\(\);\s*\}/,
  );
});

test('仅 WebView 兼容画面允许 sourceAudio 回绕幂等推进 Rust，原生 mpv 等待自身 EOF', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const publishStart = source.indexOf('function publishMediaState');
  const publishEnd = source.indexOf('function applyPlaybackMediaControl', publishStart);
  const publishSource = source.slice(publishStart, publishEnd);

  assert.ok(publishStart >= 0 && publishEnd > publishStart);
  assert.match(publishSource, /didSourceMediaLoopWrap\(/);
  assert.match(publishSource, /completeCurrentPlaybackLoop\(/);
  assert.match(publishSource, /!managedNativeVideoOwnsPlayback/);
  assert.match(publishSource, /if \(loopSyncPromiseRef\.current !== null\) return;/);
  assert.doesNotMatch(publishSource, /Math\.max\(loopSequenceRef\.current/);
  assert.doesNotMatch(publishSource, /loopSequenceRef\.current\s*=\s*sourceClock\.loopIndex/);
});

test('受管 mpv 活跃或 EOF 过渡时共享边界拒绝 WebView 与 sourceAudio 推进', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const completeStart = source.indexOf('function completeCurrentPlaybackLoop');
  const completeEnd = source.indexOf('function restartToNextLoop', completeStart);
  const completeSource = source.slice(completeStart, completeEnd);
  const boundaryStart = source.indexOf('function restartAtBoundary');
  const boundaryEnd = source.indexOf('\n\n  return (', boundaryStart);
  const boundarySource = source.slice(boundaryStart, boundaryEnd);

  assert.ok(completeStart >= 0 && completeEnd > completeStart);
  assert.ok(boundaryStart >= 0 && boundaryEnd > boundaryStart);
  assert.match(completeSource, /if \(\s*managedNativeVideoOwnsPlayback[\s\S]*\) return;/);
  assert.match(boundarySource, /\|\| managedNativeVideoOwnsPlayback/);
  assert.match(boundarySource, /realtimeVideoStreamActive:\s*managedNativeVideoOwnsPlayback/);
  assert.ok(
    completeSource.indexOf('managedNativeVideoOwnsPlayback')
      < completeSource.indexOf('restartToNextLoop('),
    '共享入口必须在调用 complete_playback_item 链路前拒绝受管 mpv',
  );
});

test('EOF 推进守卫使用黏性 managed 所有权，瞬时 DTO 丢失不会交给 WebView', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const publishStart = source.indexOf('function publishMediaState');
  const publishEnd = source.indexOf('function applyPlaybackMediaControl', publishStart);
  const publishSource = source.slice(publishStart, publishEnd);
  const completeStart = source.indexOf('function completeCurrentPlaybackLoop');
  const completeEnd = source.indexOf('function restartToNextLoop', completeStart);
  const completeSource = source.slice(completeStart, completeEnd);

  assert.match(source, /resolveManagedNativeVideoOwnership\(/);
  assert.match(source, /managedNativeVideoOwnershipRef/);
  assert.match(publishSource, /!managedNativeVideoOwnsPlayback/);
  assert.match(completeSource, /managedNativeVideoOwnsPlayback/);
  assert.doesNotMatch(completeSource, /managedNativeVideoActive/);
});

test('最终效果窗口的 loop ref 只镜像 Rust snapshot', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const loopSyncStart = source.indexOf('// N/N+1 临时文件切换不是换源');
  const loopSyncEnd = source.indexOf('function restartCurrentPlayback', loopSyncStart);
  const loopSyncSource = source.slice(loopSyncStart, loopSyncEnd);

  assert.match(loopSyncSource, /loopSequenceRef\.current = snapshot\?\.loop_index \?\? 0/);
  assert.doesNotMatch(loopSyncSource, /snapshot\?\.loop_index[^\n]*> loopSequenceRef\.current/);
});

test('WebView scheduler 只处理音频，Rust 视频配置自行派生循环身份', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const schedulerStart = source.indexOf('// WebView 定时器只唤醒声音候选');
  const schedulerEnd = source.indexOf('function publishRuntimeParameterMessage', schedulerStart);
  const scheduler = source.slice(schedulerStart, schedulerEnd);
  assert.match(scheduler, /initializeFutureAudioCyclePlans\(clock\)/);
  assert.doesNotMatch(scheduler, /prepareNextVideoMediaCandidate\(/);
  assert.match(source, /configure_realtime_video_cycle/);
  assert.doesNotMatch(source, /const loopIndex = sameGenerationClock|clock_epoch: request\.clock_epoch/);
});

test('视频 prepare 和 commit 已从 WebView 生产链删除', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /prepareNextVideoMediaCandidate|commitPreparedRealtimeVideoCandidate/);
  assert.doesNotMatch(source, /prepare_realtime_video_plan|commit_realtime_video_plan/);
});

test('播放结束推进使用源身份且不依赖旧视频流状态', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const guardStart = source.indexOf('function isCurrentFinalEffectVideo');
  const guardEnd = source.indexOf('useEffect(() => {', guardStart);
  const guardSource = source.slice(guardStart, guardEnd);

  assert.ok(guardStart >= 0 && guardEnd > guardStart);
  assert.match(guardSource, /sourceIdentityElement\?\.getAttribute\('src'\) === currentSourceIdentity/);
  assert.match(guardSource, /playbackVideoUrl\(currentSnapshot\)/);
  assert.doesNotMatch(guardSource, /VideoMse|videoMse|video_stream/);
  assert.doesNotMatch(guardSource, /video\.currentSrc/);
});

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

test('最终效果窗口只保留受管 mpv 视频入口', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.match(source, /ensure_original_video_renderer/);
  assert.match(source, /configure_realtime_video_cycle/);
  assert.match(source, /get_media_video_backend_status/);
  assert.doesNotMatch(source, /prepare_realtime_video_plan|commit_realtime_video_plan|sync_realtime_video_renderer/);
  assert.doesNotMatch(source, /stop_realtime_video_renderer/);
  assert.doesNotMatch(source, /prepare_media_video_stream|read_media_video_stream|ack_media_video_stream|commit_media_video_stream|VideoMse|video_stream/);
});

test('循环提交未完成时拒绝 ended 和 timeupdate 重复推进本地轮次', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
      syncInFlight: true,
    }),
    false,
  );
});

test('循环提交失败时保留 Tauri 返回的真实原因', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(
    source,
    /setPlaybackError\(getDisplayErrorMessage\(cause, '播放项切换失败，已重播当前媒体。'\)\)/,
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

test('Original 与 GPU 切换竞态按 Rust revision 接收并 fail closed', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const originalStart = source.indexOf("void invoke<unknown>('ensure_original_video_renderer'");
  const originalSource = source.slice(originalStart, originalStart + 1_800);

  assert.match(source, /acceptedMediaVideoBackendStatusRef/);
  assert.match(source, /setLastObservedMediaVideoBackendStatus\(acceptance\.lastObserved\)/);
  assert.match(source, /mediaVideoBackendStatusRef\.current = acceptance\.status/);
  assert.match(source, /setMediaVideoBackendDiagnostic\(acceptance\.diagnostic\)/);
  assert.match(source, /setEffectiveMediaVideoBackend\(null\)/);
  assert.match(
    originalSource,
    /originalVideoEnsureAttemptRef\.current !== attemptKey[\s\S]*return;[\s\S]*parseMediaVideoBackendStatusResult\(response\)/,
    '开关已切到 GPU 后，陈旧 Original 响应不得覆盖最新后端事实',
  );

  const startPlaybackStart = source.indexOf('async function startPlaybackFromHome');
  const startPlaybackEnd = source.indexOf('async function updateProcessingSwitches', startPlaybackStart);
  const startPlaybackSource = source.slice(startPlaybackStart, startPlaybackEnd);
  assert.match(startPlaybackSource, /await processingSwitchQueueRef\.current/);
  assert.ok(
    startPlaybackSource.indexOf('await processingSwitchQueueRef.current')
      < startPlaybackSource.indexOf('openFinalEffectWindowFromHome()'),
    '播放必须等待视频处理开关提交完成，不能先启动错误的 Original 链',
  );
});

test('主页 seek 草稿只由同身份 mpv 物理确认结算，失败恢复权威位置', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const seekStateStart = source.indexOf('const [mediaSeekDraft');
  const seekStateEnd = source.indexOf('const playbackClockNotice', seekStateStart);
  const seekStateSource = source.slice(seekStateStart, seekStateEnd);
  assert.doesNotMatch(seekStateSource, /clockEpoch|loopIndex/);
  assert.match(source, /invokePlaybackSnapshot\('seek_playback'/);
  assert.match(source, /playbackGeneration: mediaState\.playback_generation[\s\S]*positionMs/);
  assert.match(source, /settlePendingMediaSeek\(event\.data\)/);
  assert.match(source, /视频定位尚未得到 Rust\/mpv 物理确认/);
});
