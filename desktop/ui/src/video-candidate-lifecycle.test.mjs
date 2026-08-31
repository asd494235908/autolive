import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

test('React 不再拥有视频候选、prepare 或 commit 生命周期', () => {
  for (const removed of [
    'runtimeSchedulerRef',
    'videoFuturePlansRef',
    'mediaCandidateSequenceRef',
    'videoCycleRetryRef',
    'activeVideoRenderRef',
    'realtimeVideoCommitInFlightRef',
    'prepareNextVideoMediaCandidate',
    'commitPreparedRealtimeVideoCandidate',
    'prepare_realtime_video_plan',
    'commit_realtime_video_plan',
    'sync_realtime_video_renderer',
  ]) {
    assert.doesNotMatch(app, new RegExp(removed));
  }
});

test('视频周期配置只提交参数与周期范围，播放身份由 Rust 派生', () => {
  const start = app.indexOf("void invoke<unknown>('configure_realtime_video_cycle'");
  const configure = app.slice(app.lastIndexOf('useEffect(() => {', start), start + 1_400);
  assert.notEqual(start, -1);
  assert.match(configure, /params: mediaEffectParams/);
  assert.match(configure, /min_period_ms: videoPeriodRange\.minMs/);
  assert.match(configure, /max_period_ms: videoPeriodRange\.maxMs/);
  assert.doesNotMatch(configure, /clock_epoch|loop_index|backend_epoch|source_revision/);
});

test('暂停和恢复保持同一 Rust 周期配置，不清空 N/N+1/N+2', () => {
  const start = app.indexOf("void invoke<unknown>('configure_realtime_video_cycle'");
  const configure = app.slice(app.lastIndexOf('useEffect(() => {', start), start + 1_400);
  assert.match(configure, /\['playing', 'paused'\]\.includes\(snapshot\.playback_state\.toLowerCase\(\)\)/);
  assert.match(configure, /playbackGeneration: snapshot\.playback_generation/);
  assert.match(configure, /sourcePath: source\.source_path/);
  assert.doesNotMatch(configure, /playbackState:/);
  assert.match(configure, /realtimeVideoCycleConfigurationKeyRef\.current !== configurationKey/);
  assert.doesNotMatch(configure, /cancelled/);
});

test('主页 seek 只发送 Rust 用户意图，BroadcastChannel 仅跟随成功结果', () => {
  const start = app.indexOf("invokePlaybackSnapshot('seek_playback'");
  const seek = app.slice(start, start + 900);
  assert.notEqual(start, -1);
  assert.match(seek, /playbackGeneration: mediaState\.playback_generation/);
  assert.match(seek, /positionMs/);
  assert.match(seek, /applyPlaybackPoolSnapshot\(nextSnapshot\)/);
  assert.match(seek, /postPlaybackMediaControl\(/);
  assert.doesNotMatch(seek, /clockEpoch|loopIndex|backendEpoch/);
});

test('100ms 唤醒器只推进音频周期', () => {
  const start = app.indexOf('// WebView 定时器只唤醒声音候选');
  const scheduler = app.slice(app.indexOf('useEffect(() => {', start), app.indexOf('function publishRuntimeParameterMessage(', start));
  assert.notEqual(start, -1);
  assert.match(scheduler, /initializeFutureAudioCyclePlans|prepareNextAudioMediaCandidate/);
  assert.doesNotMatch(scheduler, /video|Video|configure_realtime_video_cycle/);
});

test('视频展示按 effective 与 Rust apply_state 判定，未确认状态不能冒充已生效', () => {
  assert.match(app, /setLastObservedMediaVideoBackendStatus/);
  assert.match(app, /mediaVideoBackendStatus = acceptedMediaVideoBackendStatus/);
  assert.doesNotMatch(app, /mediaVideoBackendStatus = lastObservedMediaVideoBackendStatus/);
  assert.match(app, /effectiveMediaVideoBackend/);
  assert.match(app, /setEffectiveMediaVideoBackend\(null\)/);
  assert.match(app, /\['source_transitioning', 'ready', 'applying', 'result_unknown', 'readback_confirmed', 'presented_confirmed'\]/);
  assert.match(app, /等待物理呈现确认/);
  assert.match(app, /confirmed_change_count/);
});

test('同代非 Original 进程停在 Source Idle 时明确报告 Rust 周期未建立', () => {
  const start = app.indexOf('const currentVideoCycleMissing');
  const diagnostic = app.slice(start, app.indexOf('const audioProgressPercent', start));
  const status = app.slice(app.indexOf('const videoProcessingStatus'), app.indexOf('const audioProcessingStatus'));

  assert.notEqual(start, -1);
  assert.match(diagnostic, /videoProcessingEnabled/);
  assert.match(diagnostic, /mediaVideoBackendDiagnostic === null/);
  assert.match(diagnostic, /playback_generation === snapshot\?\.playback_generation/);
  assert.match(diagnostic, /process_id !== null/);
  assert.match(diagnostic, /fallback_floor_mode !== 'original'/);
  assert.match(diagnostic, /backend === 'source'/);
  assert.match(diagnostic, /apply_state === 'idle'/);
  assert.match(status, /currentVideoCycleMissing[\s\S]*'Rust 视频周期未建立'/);
  assert.match(status, /currentVideoCycleMissing[\s\S]*\? 'error'/);
});

test('视频状态轮询后台停止，回前台立即拉取且不触发 prepare/commit', () => {
  const marker = 'if (!documentVisible || !currentMediaIsVideo) return;';
  const start = app.indexOf(marker);
  const poll = app.slice(start, app.indexOf('  ]);', start) + 5);
  assert.notEqual(start, -1);
  assert.match(poll, /refreshStatus\(\);[\s\S]*window\.setInterval\(refreshStatus, VIDEO_BACKEND_STATUS_POLL_MS\)/);
  assert.doesNotMatch(poll, /prepare|commit/);
});

test('Original ensure 不再接收 WebView 生成的视频 epoch', () => {
  const start = app.indexOf("invoke<unknown>('ensure_original_video_renderer'");
  const ensure = app.slice(start, start + 180);
  assert.notEqual(start, -1);
  assert.match(ensure, /request: \{\}/);
  assert.doesNotMatch(ensure, /clock_epoch|loop_index|backend_epoch|position_ms/);
});
