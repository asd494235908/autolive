import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';

const appSource = fs.readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');

test('视频开关只在权威响应提交后放行 Rust 周期配置', () => {
  const start = appSource.indexOf('async function updateProcessingSwitches(');
  const update = appSource.slice(start, appSource.indexOf('async function resetVideoEffectParams(', start));
  assert.match(update, /videoProcessingSwitchRevisionRef\.current \+= 1/);
  assert.match(update, /videoProcessingSwitchCommittedRevisionRef\.current = videoSwitchRevision/);
  assert.ok(update.indexOf('snapshotRefHome.current = nextSnapshot')
    < update.indexOf('videoProcessingSwitchCommittedRevisionRef.current = videoSwitchRevision'));
  assert.match(appSource, /const videoProcessingSwitchCommitted =/);
  assert.match(appSource, /videoProcessingSwitchCommitted[\s\S]*configure_realtime_video_cycle/);
});

test('WebView 不再创建视频 prepare/commit 候选', () => {
  assert.doesNotMatch(appSource, /prepareNextVideoMediaCandidate|commitPreparedRealtimeVideoCandidate/);
  assert.doesNotMatch(appSource, /prepare_realtime_video_plan|commit_realtime_video_plan/);
  assert.doesNotMatch(appSource, /activeVideoRenderRef|videoFuturePlansRef|realtimeVideoCommitInFlightRef/);
});

test('生产 IPC 边界保留结构化 DTO 诊断并分离 lastObserved、accepted 与 effective', () => {
  assert.doesNotMatch(appSource, /parseMediaVideoBackendStatus\(/);
  assert.match(appSource, /parseMediaVideoBackendStatusResult\(record\.video_backend_status\)/);
  assert.match(appSource, /setLastObservedMediaVideoBackendStatus\(acceptance\.lastObserved\)/);
  assert.match(appSource, /acceptedMediaVideoBackendStatus/);
  assert.match(appSource, /const mediaVideoBackendStatus = acceptedMediaVideoBackendStatus/);
  assert.doesNotMatch(appSource, /const mediaVideoBackendStatus = lastObservedMediaVideoBackendStatus/);
  assert.match(appSource, /effectiveMediaVideoBackend/);
  assert.match(appSource, /setEffectiveMediaVideoBackend\(null\)/);
});

test('WebView 周期唤醒只处理音频，视频配置交给 Rust', () => {
  const start = appSource.indexOf('function wakeMediaCycleScheduling(');
  const wake = appSource.slice(start, appSource.indexOf('function isVideoBackendEofPending(', start));
  assert.match(wake, /initializeFutureAudioCyclePlans\(clock\)/);
  assert.match(wake, /prepareNextAudioMediaCandidate/);
  assert.doesNotMatch(wake, /prepareNextVideoMediaCandidate|configure_realtime_video_cycle/);
  assert.match(appSource, /configure_realtime_video_cycle/);
});

test('Original ensure 与 seek 均不接收 WebView 视频 epoch', () => {
  const ensureStart = appSource.indexOf("invoke<unknown>('ensure_original_video_renderer'");
  const ensure = appSource.slice(ensureStart, ensureStart + 180);
  assert.match(ensure, /request: \{\}/);
  assert.doesNotMatch(ensure, /clock_epoch|loop_index|backend_epoch/);

  const seekStart = appSource.indexOf("invokePlaybackSnapshot('seek_playback'");
  const seek = appSource.slice(seekStart, seekStart + 400);
  assert.match(seek, /playbackGeneration/);
  assert.match(seek, /positionMs/);
  assert.doesNotMatch(seek, /clockEpoch|loopIndex|backendEpoch/);
});
