import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('候选目标时钟失效会刷新最新快照后重建或推进且不展示为故障', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const classifier = app.slice(
    app.indexOf('function isTransientAudioCandidateCode'),
    app.indexOf('function drawDiagnosticCanvas'),
  );
  const desktopStart = app.indexOf('function DesktopApp()');
  const resultHandlerStart = app.indexOf("if (event.data.action === 'prepare')", desktopStart);
  const resultHandler = app.slice(
    resultHandlerStart,
    app.indexOf("if (event.data.action === 'cancel')", resultHandlerStart),
  );

  assert.match(classifier, /audio_candidate_target_invalid/);
  assert.match(resultHandler, /invoke<PlaybackSnapshot>\('get_snapshot'\)/);
  assert.match(resultHandler, /resolveAudioSyncClock\(latestSnapshot, null, null\)/);
  assert.match(resultHandler, /const targetDeltaMs = latestPlan\.targetAbsolutePositionMs - latestClock\.absolute_position_ms/);
  assert.match(resultHandler, /targetDeltaMs <= 0[\s\S]*advanceRejectedAudioPlan\(\)/);
  assert.match(resultHandler, /targetDeltaMs <= PERIOD_HARD_MAX_MS[\s\S]*bindNextAudioCandidate\(latestPlan\)/);
  assert.match(resultHandler, /clearFutureMediaCyclePlans\(false\)/);
  assert.match(resultHandler, /mediaStateRef\.current = null/);
  assert.match(resultHandler, /bindNextAudioCandidate\(latestPlan\)/);
  assert.match(resultHandler, /advanceRejectedAudioPlan\(\)/);
});
