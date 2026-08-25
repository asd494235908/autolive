import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./audio-cycle-prewarm-coordinator.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const coordinator = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

test('planned N+1 预选后立即准备，不受目标距离和播放倍率影响', () => {
  const long = coordinator.createAudioCycleCandidatePlan(1, 'next', 1_000, 10_000);
  assert.equal(long.targetAbsolutePositionMs, 11_000);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(long, 1_000, 1), 'prepare');
  assert.equal(coordinator.getAudioCycleCoordinatorAction(long, 1_000, 0.5), 'prepare');

  const distant = coordinator.createAudioCycleCandidatePlan(2, 'next', 1_000, 120_000);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(distant, 1_000, 2), 'prepare');
});

test('preparing、prepared 和 committing 不会重复 prepare', () => {
  const planned = coordinator.createAudioCycleCandidatePlan(2, 'next', 1_000, 120_000);
  for (const status of ['preparing', 'prepared', 'committing']) {
    const candidate = { ...planned, status };
    assert.notEqual(
      coordinator.getAudioCycleCoordinatorAction(candidate, 1_000, 1),
      'prepare',
      status,
    );
  }
});

test('N+2 仅保留为调用方元数据，不创建 coordinator plan', () => {
  const nextNextSample = { seed: 3 };
  const coordinatorPlan = null;

  assert.deepEqual(nextNextSample, { seed: 3 });
  assert.equal(coordinator.getAudioCycleCoordinatorAction(coordinatorPlan, 1_000, 1), null);
});

test('只有 prepared 候选到期后才提交', () => {
  const planned = coordinator.createAudioCycleCandidatePlan(3, 'next', 0, 5_000);
  const preparing = coordinator.updateAudioCycleCandidateStatus(planned, 'preparing');
  const prepared = coordinator.updateAudioCycleCandidateStatus(planned, 'prepared');

  assert.equal(coordinator.getAudioCycleCoordinatorAction(preparing, 5_000, 1, 0), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_999, 1, 0), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 5_000, 1, 0), 'commit');
});

test('候选提交会扣除环缓待播和硬件延迟对应的媒体时间', () => {
  const plan = coordinator.createAudioCycleCandidatePlan(4, { seed: 2 }, 0, 5_000);
  const prepared = coordinator.updateAudioCycleCandidateStatus(plan, 'prepared');

  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_749, 1, 250), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_750, 1, 250), 'commit');
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_624, 1.5, 250), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_625, 1.5, 250), 'commit');
});

test('候选错误没有恢复旧 candidate 或旧 revision 的入口', () => {
  assert.equal('recoverAudioCycleCandidate' in coordinator, false);
});

test('旧 candidate ID 的结果可由调用方稳定拒绝', () => {
  const active = coordinator.createAudioCycleCandidatePlan(9, 'new', 0, 5_000);
  const staleResult = { candidate_id: 8 };
  assert.notEqual(staleResult.candidate_id, active.candidateId);
});

test('prepare 消息只接受绝对媒体时间目标，不接受旧墙钟字段', () => {
  const valid = {
    version: 1,
    type: 'audio-cycle-command',
    action: 'prepare',
    candidate_id: 1,
    playback_generation: 2,
    base_audio_stream_revision: 3,
    target_absolute_position_ms: 12_000,
    audio: {},
    audio_variants: [],
  };
  assert.equal(coordinator.isAudioCycleCommandMessage(valid), true);
  assert.equal(coordinator.isAudioCycleCommandMessage({
    ...valid,
    target_absolute_position_ms: undefined,
    target_at_ms: Date.now(),
  }), false);
});

test('候选数字静音作为已接受的跳过结果携带稳定原因码', () => {
  const skipped = {
    version: 1,
    type: 'audio-cycle-result',
    action: 'commit',
    candidate_id: 7,
    accepted: true,
    committed: false,
    reason: '候选音轨首段为数字静音，保持当前音轨',
    error_code: 'audio_mixer_candidate_silent',
  };

  assert.equal(coordinator.isAudioCycleResultMessage(skipped), true);
  assert.equal(coordinator.isAudioCycleResultMessage({ ...skipped, error_code: 7 }), false);
  assert.equal(coordinator.isAudioCycleResultMessage({ ...skipped, reason: {} }), false);
});

test('候选数字静音和过期代次是预期跳过，真实故障仍需展示', () => {
  assert.equal(coordinator.isExpectedAudioCycleSkipCode('audio_mixer_candidate_silent'), true);
  assert.equal(coordinator.isExpectedAudioCycleSkipCode('audio_mixer_candidate_stale'), true);
  assert.equal(coordinator.isExpectedAudioCycleSkipCode('audio_mixer_crossfade_failed'), false);
  assert.equal(coordinator.isExpectedAudioCycleSkipCode(null), false);
});
