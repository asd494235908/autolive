import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-artifact-timeline.ts', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const timeline = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

test('候选片段从绝对目标映射到源文件位置并覆盖下一边界', () => {
  const value = timeline.createMediaArtifactTimeline({
    planId: 'video-1',
    sequence: 1,
    playbackGeneration: 3,
    sourceRevision: 4,
    targetAbsolutePositionMs: 48_000,
    validUntilAbsolutePositionMs: 56_000,
    sourceDurationMs: 45_000,
    safetyTailMs: 500,
  });

  assert.equal(value.sourceStartMs, 3_000);
  assert.equal(value.outputDurationMs, 8_500);
});

test('多项播放池的声音候选不会把容器无声尾部绕回下一轮音轨', () => {
  assert.equal(timeline.doesAudioWindowOverlapArtifact({
    sourceStartMs: 44_200,
    outputDurationMs: 800,
    sourceDurationMs: 44_468,
    audioStartMs: 0,
    audioEndMs: 44_102,
    loopSource: false,
  }), false);
});

test('声音候选与有效音轨部分相交时保留原候选窗口', () => {
  assert.equal(timeline.doesAudioWindowOverlapArtifact({
    sourceStartMs: 43_900,
    outputDurationMs: 500,
    sourceDurationMs: 44_468,
    audioStartMs: 0,
    audioEndMs: 44_102,
    loopSource: false,
  }), true);
});

test('单项循环的声音候选跨 EOF 后按源时长周期判断下一轮音轨', () => {
  assert.equal(timeline.doesAudioWindowOverlapArtifact({
    sourceStartMs: 44_200,
    outputDurationMs: 800,
    sourceDurationMs: 44_468,
    audioStartMs: 0,
    audioEndMs: 44_102,
    loopSource: true,
  }), true);
});

test('未知音轨区间沿用旧候选逻辑', () => {
  assert.equal(timeline.doesAudioWindowOverlapArtifact({
    sourceStartMs: 44_200,
    outputDurationMs: 800,
    sourceDurationMs: 44_468,
    audioStartMs: null,
    audioEndMs: null,
    loopSource: false,
  }), true);
});

test('无音轨交集在 invoke 前静默跳过且只有单项循环推进声音队列', () => {
  const prepareStart = appSource.indexOf('async function prepareNextAudioMediaCandidate(');
  const prepareEnd = appSource.indexOf('async function prepareAudioMediaCandidate(', prepareStart);
  const prepare = appSource.slice(prepareStart, prepareEnd);

  assert.match(prepare, /doesAudioWindowOverlapArtifact/);
  assert.ok(prepare.indexOf('doesAudioWindowOverlapArtifact') < prepare.indexOf('prepareAudioMediaCandidate(candidate)'));
  assert.match(prepare, /if \(planned\.loopSource\)[\s\S]*advanceIndependentAudioQueue\([\s\S]*false[\s\S]*\);[\s\S]*return/);
  assert.doesNotMatch(prepare, /scheduleAudioCycleRetry|setError/);
});

test('Rust 音轨窗口防线返回 skip 时不显示错误或进入退避', () => {
  const prepareStart = appSource.indexOf('async function prepareAudioMediaCandidate(');
  const prepareEnd = appSource.indexOf('function prepareNextVideoMediaCandidate(', prepareStart);
  const prepare = appSource.slice(prepareStart, prepareEnd);

  assert.match(prepare, /AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE/);
  assert.match(prepare, /getCommandErrorCode\(cause\)/);
  const skipStart = prepare.indexOf('getCommandErrorCode(cause) === AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE');
  const retryStart = prepare.indexOf('scheduleAudioCycleRetry()', skipStart);
  assert.ok(skipStart >= 0 && retryStart > skipStart);
  assert.match(prepare.slice(skipStart, retryStart), /return/);
});

test('声音候选失败时保留仍可用的当前处理音轨', () => {
  const failureEffectStart = appSource.indexOf("if (snapshot?.audio_processing_status !== 'failed') return;");
  const failureEffect = appSource.slice(
    failureEffectStart,
    appSource.indexOf('useEffect(() => {', failureEffectStart + 1),
  );

  assert.match(failureEffect, /hasUsableProcessedAudioOutput\(\)/);
  assert.match(failureEffect, /restoreDryAudioOutput\(\)/);
  assert.doesNotMatch(failureEffect, /强制回到源视频声音/);

  const processedAudioElements = appSource.slice(
    appSource.indexOf('ref={processedAudioARef}'),
    appSource.indexOf('ref={interludeAudioRef}'),
  );
  assert.match(processedAudioElements, /onError=\{handleProcessedAudioElementError\}/g);
  assert.doesNotMatch(processedAudioElements, /onError=\{restoreDryAudioOutput\}/);
});
