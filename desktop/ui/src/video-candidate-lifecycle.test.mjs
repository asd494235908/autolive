import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

function section(start, end) {
  const body = app.split(start)[1]?.split(end)[0] ?? '';
  assert.notEqual(body, '', `missing section: ${start}`);
  return body;
}

test('mpv 视频候选先预占唯一 owner，失败与失效路径对称释放', () => {
  const apply = section('async function applyVideoProcessing(', 'function commitPreparedRealtimeVideoCandidate(');
  assert.match(apply, /activeVideoRenderRef\.current = mediaCandidate/);
  assert.match(
    apply,
    /function releaseUnstartedCandidate\([\s\S]*pendingMediaApplyRef\.current = null[\s\S]*activeVideoRenderRef\.current = null/,
  );
  assert.match(apply, /prepare_realtime_video_plan/);
  assert.match(apply, /stop_realtime_video_renderer/);
  assert.doesNotMatch(apply, /MediaSource|SourceBuffer|video_stream|media_video_stream|activate_ffmpeg_video_backend/);
});

test('mpv prepare 成功后只在权威时钟到点提交并推进视频队列', () => {
  const commit = section('function commitPreparedRealtimeVideoCandidate(', 'function commitCompletedAudioRender(');
  assert.match(commit, /candidate\.realtimePrepared/);
  assert.match(commit, /clock\.absolute_position_ms < candidate\.timeline\.targetAbsolutePositionMs/);
  assert.match(commit, /commit_realtime_video_plan/);
  assert.match(commit, /applyPlannedVideoCycle\(candidate\.videoCyclePlan\)/);
  assert.match(commit, /advanceIndependentVideoQueue\(clock\.absolute_position_ms\)/);
});

test('视频候选只绑定当前播放源，不再跨源预热旧 period', () => {
  const binding = section('function isVideoCandidateBoundToCurrentSource(', 'function initializeFutureMediaCyclePlans(');
  assert.match(binding, /candidate\.playbackGeneration === currentSnapshot\.playback_generation/);
  assert.match(binding, /candidate\.sourceMediaIndex === currentIndex/);
  assert.doesNotMatch(app, /prewarmNextPlaybackPoolVideoSource|handleBufferedVideoPeriod|bufferedVideoRendersRef/);
});
