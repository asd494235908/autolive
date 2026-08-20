import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-cycle-planner.ts', import.meta.url), 'utf8');
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

  assert.equal(queue.length, 2);
  assert.deepEqual(queue.map(({ planId, sequence, targetAbsolutePositionMs }) => ({
    planId,
    sequence,
    targetAbsolutePositionMs,
  })), [
    { planId: 'audio-1', sequence: 1, targetAbsolutePositionMs: 17_000 },
    { planId: 'audio-2', sequence: 2, targetAbsolutePositionMs: 25_000 },
  ]);
  assert.deepEqual(Object.keys(queue[1]).sort(), [
    'payload',
    'periodMediaMs',
    'planId',
    'sequence',
    'targetAbsolutePositionMs',
  ]);
});

test('推进时旧 N+2 原样晋升 N+1，并只补一个新的 N+2', () => {
  const initial = planner.createMediaCycleQueue(0, [
    seed('one', 5_000, 'one'),
    seed('two', 6_000, 'two'),
  ]);
  const advanced = planner.advanceMediaCycleQueue(initial, seed('three', 7_000, 'three'));

  assert.equal(advanced.length, 2);
  assert.strictEqual(advanced[0], initial[1]);
  assert.deepEqual(advanced[1], {
    planId: 'three',
    sequence: 3,
    periodMediaMs: 7_000,
    targetAbsolutePositionMs: 18_000,
    payload: 'three',
  });
});

test('独立模式为声音和视频维护不同的两周期队列', () => {
  const queues = planner.createIndependentMediaCycleQueues(2_000, {
    audio: [seed('a1', 5_000, 'audio-1'), seed('a2', 7_000, 'audio-2')],
    video: [seed('v1', 8_000, 'video-1'), seed('v2', 9_000, 'video-2')],
  });

  assert.deepEqual(queues.audio.map((item) => item.targetAbsolutePositionMs), [7_000, 14_000]);
  assert.deepEqual(queues.video.map((item) => item.targetAbsolutePositionMs), [10_000, 19_000]);
});

test('联动模式让声音和视频共享 planId、sequence 和绝对目标', () => {
  const queues = planner.createLinkedMediaCycleQueues(3_000, [
    { planId: 'linked-1', periodMediaMs: 8_000, audioPayload: 'a1', videoPayload: 'v1' },
    { planId: 'linked-2', periodMediaMs: 9_000, audioPayload: 'a2', videoPayload: 'v2' },
  ]);

  for (let index = 0; index < 2; index += 1) {
    assert.equal(queues.audio[index].planId, queues.video[index].planId);
    assert.equal(queues.audio[index].sequence, queues.video[index].sequence);
    assert.equal(
      queues.audio[index].targetAbsolutePositionMs,
      queues.video[index].targetAbsolutePositionMs,
    );
  }
  assert.deepEqual(queues.audio.map((item) => item.payload), ['a1', 'a2']);
  assert.deepEqual(queues.video.map((item) => item.payload), ['v1', 'v2']);
});

test('联动周期取音视频范围交集，无交集时明确失败', () => {
  assert.deepEqual(
    planner.intersectPeriodRanges({ minMs: 5_000, maxMs: 10_000 }, { minMs: 8_000, maxMs: 15_000 }),
    { ok: true, range: { minMs: 8_000, maxMs: 10_000 } },
  );
  const noIntersection = planner.intersectPeriodRanges(
    { minMs: 1_000, maxMs: 4_000 },
    { minMs: 5_000, maxMs: 8_000 },
  );
  assert.equal(noIntersection.ok, false);
  assert.match(noIntersection.reason, /没有交集/);
});
