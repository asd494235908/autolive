import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./audio-cycle-prewarm-coordinator.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const coordinator = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

test('下一周期在切换前四秒准备，短周期立即准备', () => {
  const long = coordinator.createAudioCycleCandidatePlan(1, 'next', 1_000, 10_000);
  assert.equal(long.prepareAtMs, 7_000);
  assert.equal(long.targetAtMs, 11_000);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(long, 6_999), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(long, 7_000), 'prepare');

  const short = coordinator.createAudioCycleCandidatePlan(2, 'next', 1_000, 3_000);
  assert.equal(short.prepareAtMs, 1_000);
});

test('只有 prepared 候选到期后才提交', () => {
  const planned = coordinator.createAudioCycleCandidatePlan(3, 'next', 0, 5_000);
  const preparing = coordinator.updateAudioCycleCandidateStatus(planned, 'preparing');
  const prepared = coordinator.updateAudioCycleCandidateStatus(planned, 'prepared');

  assert.equal(coordinator.getAudioCycleCoordinatorAction(preparing, 5_000), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 4_999), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(prepared, 5_000), 'commit');
});

test('失败保留 current，并只在 500ms 宽限内重试同一候选', () => {
  const current = { seed: 1 };
  const plan = coordinator.createAudioCycleCandidatePlan(4, { seed: 2 }, 0, 5_000);
  const committing = coordinator.updateAudioCycleCandidateStatus(plan, 'committing');
  const retry = coordinator.recoverAudioCycleCandidate(committing, 'commit', 5_300);

  assert.deepEqual(current, { seed: 1 });
  assert.equal(retry?.sample.seed, 2);
  assert.equal(retry?.status, 'prepared');
  assert.equal(coordinator.recoverAudioCycleCandidate(committing, 'commit', 5_501), null);
  assert.equal(coordinator.getAudioCycleCoordinatorAction(committing, 5_501), 'expire');
});

test('旧 candidate ID 的结果可由调用方稳定拒绝', () => {
  const active = coordinator.createAudioCycleCandidatePlan(9, 'new', 0, 5_000);
  const staleResult = { candidate_id: 8 };
  assert.notEqual(staleResult.candidate_id, active.candidateId);
});
