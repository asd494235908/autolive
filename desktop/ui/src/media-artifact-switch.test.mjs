import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-artifact-switch.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const switching = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

const identity = (overrides = {}) => ({
  playbackGeneration: 7,
  sourceRevision: 3,
  planId: 'video-12',
  sequence: 12,
  ...overrides,
});

const candidate = (overrides = {}) => ({
  ...identity(),
  targetAbsolutePositionMs: 8_000,
  validUntilAbsolutePositionMs: 17_000,
  outputDurationMs: 9_500,
  ...overrides,
});

test('绝对位置与候选本地时间互相映射，并拒绝到期前和安全整数溢出', () => {
  assert.equal(switching.mapAbsolutePositionToCandidateTime(8_000, 8_000), 0);
  assert.equal(switching.mapAbsolutePositionToCandidateTime(8_450, 8_000), 450);
  assert.equal(switching.mapAbsolutePositionToCandidateTime(7_999, 8_000), null);
  assert.equal(
    switching.mapAbsolutePositionToCandidateTime(Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER - 10),
    10,
  );

  assert.equal(switching.mapCandidateTimeToAbsolutePosition(450, 8_000), 8_450);
  assert.equal(
    switching.mapCandidateTimeToAbsolutePosition(11, Number.MAX_SAFE_INTEGER - 10),
    null,
  );
});

test('绝对位置按源时长映射到规范化 position 和 loop，跨 EOF 不回退时钟', () => {
  assert.deepEqual(switching.mapAbsolutePositionToSourceClock(22_000, 10_000), {
    loopIndex: 2,
    positionMs: 2_000,
  });
  assert.deepEqual(switching.mapAbsolutePositionToSourceClock(20_000, 10_000), {
    loopIndex: 2,
    positionMs: 0,
  });
  assert.equal(switching.mapAbsolutePositionToSourceClock(1_000, 0), null);
  assert.equal(switching.mapAbsolutePositionToSourceClock(-1, 10_000), null);
});

test('候选身份必须完整匹配当前时间轴和计划', () => {
  const expected = identity();
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity()), true);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ playbackGeneration: 8 })), false);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ sourceRevision: 4 })), false);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ planId: 'video-13' })), false);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ sequence: 13 })), false);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ planId: '' })), false);
  assert.equal(switching.isMediaArtifactIdentityCurrent(expected, identity({ sequence: 0 })), false);
});

test('切换门禁校验身份和媒体覆盖范围，但候选迟到不按 validUntil 作废', () => {
  const current = identity();
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 8_000, 9_500), true);
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 16_999, 9_500), true);
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 7_999, 9_500), false);
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 17_000, 9_500), true);
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 17_500, 9_500), false);
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 16_000, 8_000), false);
  assert.equal(
    switching.canSwitchToMediaArtifact(candidate({ outputDurationMs: 8_000 }), current, 16_000, 9_500),
    false,
  );
  assert.equal(
    switching.canSwitchToMediaArtifact(candidate(), identity({ sourceRevision: 4 }), 8_000, 9_500),
    false,
  );
  assert.equal(switching.canSwitchToMediaArtifact(candidate(), current, 8_000, Number.NaN), false);
});

test('整源候选迟到后按当前源时间对齐，不回跳到旧目标片段', () => {
  const current = identity();
  const wholeSource = candidate({
    sourceStartMs: 0,
    outputDurationMs: 13_500,
  });
  assert.equal(
    switching.resolveMediaArtifactSwitchTime(
      wholeSource,
      current,
      25_000,
      13_500,
      13_500,
      true,
    ),
    11_500,
  );
});

test('无效候选窗口和超出安全整数范围的输入不能进入切换', () => {
  const current = identity();
  assert.equal(
    switching.canSwitchToMediaArtifact(
      candidate({ validUntilAbsolutePositionMs: 8_000 }),
      current,
      8_000,
      9_500,
    ),
    false,
  );
  assert.equal(
    switching.canSwitchToMediaArtifact(
      candidate({ targetAbsolutePositionMs: Number.MAX_SAFE_INTEGER + 1 }),
      current,
      8_000,
      9_500,
    ),
    false,
  );
});
