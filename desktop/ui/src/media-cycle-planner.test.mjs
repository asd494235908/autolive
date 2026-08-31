import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-cycle-planner.ts', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const planner = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const seed = (planId, periodMediaMs, payload) => ({ planId, periodMediaMs, payload });

test('初始化只创建 N+1 和 N+2 两个绝对媒体时间计划', () => {
  const queue = planner.createMediaCycleQueue(12_000, [
    seed('audio-1', 5_000, { seed: 11 }),
    seed('audio-2', 8_000, { seed: 22 }),
  ]);
  assert.deepEqual(queue.map(({ planId, sequence, targetAbsolutePositionMs }) => ({
    planId,
    sequence,
    targetAbsolutePositionMs,
  })), [
    { planId: 'audio-1', sequence: 1, targetAbsolutePositionMs: 17_000 },
    { planId: 'audio-2', sequence: 2, targetAbsolutePositionMs: 25_000 },
  ]);
});

test('推进时旧 N+2 原样晋升 N+1，并只补一个新的 N+2', () => {
  const initial = planner.createMediaCycleQueue(0, [
    seed('one', 5_000, 'one'),
    seed('two', 6_000, 'two'),
  ]);
  const advanced = planner.advanceMediaCycleQueue(initial, seed('three', 7_000, 'three'));
  assert.strictEqual(advanced[0], initial[1]);
  assert.deepEqual(advanced[1], {
    planId: 'three',
    sequence: 3,
    periodMediaMs: 7_000,
    targetAbsolutePositionMs: 18_000,
    payload: 'three',
  });
});

test('独立队列只对齐视频目标，音频目标保持原值', () => {
  const queues = planner.createIndependentMediaCycleQueues(2_001, {
    audio: [seed('a1', 5_000, 'audio-1'), seed('a2', 7_000, 'audio-2')],
    video: [seed('v1', 8_000, 'video-1'), seed('v2', 9_000, 'video-2')],
  }, 1, {
    video: (targetMs) => planner.alignMediaPositionToVideoFrame(targetMs, 30),
  });
  assert.deepEqual(queues.audio.map((item) => item.targetAbsolutePositionMs), [7_001, 14_001]);
  assert.deepEqual(queues.video.map((item) => item.targetAbsolutePositionMs), [10_033, 19_033]);
});

test('源边界视频目标仍保持有界，供 Rust 迁移期间复用', () => {
  assert.deepEqual(
    planner.resolveSourceBoundedVideoCycleQueueTargets(55_000, 10_000, 13_000, 72_300),
    [
      { targetAbsolutePositionMs: 65_000, skipVideoProcessing: true },
      { targetAbsolutePositionMs: 72_300, skipVideoProcessing: false },
    ],
  );
});

test('主页视频周期只读 Rust N、PTS 与 confirmed_change_count', () => {
  assert.match(appSource, /const backendVideoCycle = mediaVideoBackendDiagnostic === null/);
  assert.match(appSource, /mediaVideoBackendStatus\.presented_pts_ms !== null/);
  assert.match(appSource, /backendVideoCycle\?\.confirmed_change_count \?\? 0/);
  assert.match(appSource, /mediaVideoPresentationPtsMs\(\s*backendVideoCycle,\s*snapshot\?\.source_media\?\.duration_ms,\s*\)/);
  assert.match(appSource, /backendVideoPresentationPtsMs - backendVideoCycle\.n\.target_pts_ms/);
  assert.doesNotMatch(appSource, /scheduledVideoProgressPercent|videoFuturePlansRef/);
});

test('React 视频周期只配置一次，prepare/commit 已退出生产链', () => {
  assert.match(appSource, /configure_realtime_video_cycle/);
  assert.match(appSource, /realtimeVideoCycleConfigurationKeyRef/);
  assert.doesNotMatch(appSource, /prepare_realtime_video_plan|commit_realtime_video_plan/);
  assert.doesNotMatch(appSource, /prepareNextVideoMediaCandidate|commitPreparedRealtimeVideoCandidate/);
});

test('音频候选仍保留独立 N+1/N+2 队列与提交', () => {
  assert.match(appSource, /audioFuturePlansRef\.current = createMediaCycleQueue\(/);
  assert.match(appSource, /prepareNextAudioMediaCandidate/);
  assert.match(appSource, /advanceIndependentAudioQueue/);
});
