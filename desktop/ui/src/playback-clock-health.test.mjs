import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadModule() {
  const source = await readFile(new URL('./playback-clock-health.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('playing clock requests one recovery after 1500ms without progress', async () => {
  const { createPlaybackClockHealth, observePlaybackClock } = await loadModule();
  let state = createPlaybackClockHealth('source-a:1', 8, 1_000);

  let result = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_499,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'healthy');
  assert.equal(result.recoveryRequested, false);

  result = observePlaybackClock(result.state, {
    identity: 'source-a:1',
    nowMs: 2_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'recovering');
  assert.equal(result.recoveryRequested, true);

  result = observePlaybackClock(result.state, {
    identity: 'source-a:1',
    nowMs: 2_750,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'recovering');
  assert.equal(result.recoveryRequested, false);
});

test('pause, seek and loading grace cannot trigger a false stall', async () => {
  const { createPlaybackClockHealth, observePlaybackClock } = await loadModule();
  const base = createPlaybackClockHealth('source-a:1', 8, 1_000);
  for (const ignored of [
    { expectedPlaying: false, paused: true, seeking: false, ended: false, loadingGrace: false },
    { expectedPlaying: true, paused: false, seeking: true, ended: false, loadingGrace: false },
    { expectedPlaying: true, paused: false, seeking: false, ended: false, loadingGrace: true },
    { expectedPlaying: true, paused: true, seeking: false, ended: true, loadingGrace: false },
  ]) {
    const result = observePlaybackClock(base, {
      identity: 'source-a:1',
      nowMs: 10_000,
      currentTime: 8,
      ...ignored,
    });
    assert.equal(result.recoveryRequested, false);
    assert.notEqual(result.state.status, 'stalled');
  }
});

test('a paused media element still stalls when the backend says playing', async () => {
  const { createPlaybackClockHealth, observePlaybackClock } = await loadModule();
  const state = createPlaybackClockHealth('source-a:1', 8, 1_000);
  const result = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: true,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'recovering');
  assert.equal(result.recoveryRequested, true);
});

test('real media progress recovers health and a new source gets its own retry', async () => {
  const { createPlaybackClockHealth, observePlaybackClock } = await loadModule();
  let state = createPlaybackClockHealth('source-a:1', 8, 1_000);
  state = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  }).state;

  let result = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_750,
    currentTime: 8.04,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'healthy');
  assert.equal(result.state.recoveryAttempted, false);

  result = observePlaybackClock(result.state, {
    identity: 'source-a:1',
    nowMs: 4_300,
    currentTime: 8.04,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.status, 'recovering');
  assert.equal(result.recoveryRequested, true);

  result = observePlaybackClock(result.state, {
    identity: 'source-b:2',
    nowMs: 20_000,
    currentTime: 0,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(result.state.identity, 'source-b:2');
  assert.equal(result.state.recoveryAttempted, false);
});

test('pause and resume reset the baseline without granting the same source another recovery', async () => {
  const {
    createPlaybackClockHealth,
    observePlaybackClock,
    resetPlaybackClockObservation,
  } = await loadModule();
  let state = createPlaybackClockHealth('source-a:1', 8, 1_000);
  state = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  }).state;

  const resumed = resetPlaybackClockObservation(state, 'source-a:1', 8, 10_000);
  assert.equal(resumed.status, 'healthy');
  assert.equal(resumed.recoveryAttempted, true);
  const stalledAgain = observePlaybackClock(resumed, {
    identity: 'source-a:1',
    nowMs: 11_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(stalledAgain.state.status, 'stalled');
  assert.equal(stalledAgain.recoveryRequested, false);

  const nextSource = resetPlaybackClockObservation(state, 'source-b:2', 0, 20_000);
  assert.equal(nextSource.recoveryAttempted, false);

  const failed = { ...state, status: 'stalled' };
  assert.equal(
    resetPlaybackClockObservation(failed, 'source-a:1', 8, 30_000).status,
    'stalled',
  );
});

test('failed or ineffective recovery becomes a truthful stalled state', async () => {
  const {
    createPlaybackClockHealth,
    failPlaybackClockRecovery,
    observePlaybackClock,
  } = await loadModule();
  let state = createPlaybackClockHealth('source-a:1', 8, 1_000);
  state = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 2_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  }).state;

  assert.equal(failPlaybackClockRecovery(state, 'source-a:1').status, 'stalled');
  assert.equal(failPlaybackClockRecovery(state, 'source-b:2').status, 'recovering');

  const ineffective = observePlaybackClock(state, {
    identity: 'source-a:1',
    nowMs: 6_500,
    currentTime: 8,
    expectedPlaying: true,
    paused: false,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(ineffective.state.status, 'stalled');
  assert.equal(ineffective.recoveryRequested, false);
});

test('backend pause keeps stalled sticky until real media progress or a new identity', async () => {
  const { createPlaybackClockHealth, observePlaybackClock } = await loadModule();
  const stalled = {
    ...createPlaybackClockHealth('source-a:1', 8, 1_000),
    status: 'stalled',
    recoveryAttempted: true,
  };
  const paused = observePlaybackClock(stalled, {
    identity: 'source-a:1',
    nowMs: 2_000,
    currentTime: 8,
    expectedPlaying: false,
    paused: true,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(paused.state.status, 'stalled');

  const progressed = observePlaybackClock(paused.state, {
    identity: 'source-a:1',
    nowMs: 2_100,
    currentTime: 8.04,
    expectedPlaying: false,
    paused: true,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(progressed.state.status, 'healthy');

  const nextSource = observePlaybackClock(paused.state, {
    identity: 'source-b:2',
    nowMs: 2_100,
    currentTime: 0,
    expectedPlaying: false,
    paused: true,
    seeking: false,
    ended: false,
    loadingGrace: false,
  });
  assert.equal(nextSource.state.status, 'healthy');
  assert.equal(nextSource.state.recoveryAttempted, false);
});

test('App wires buffering events and publishes clock health', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(source, /'waiting',[\s\S]*'stalled',[\s\S]*'canplay',[\s\S]*'playing'/);
  assert.match(source, /clock_health: playbackClockHealthRef\.current\.status/);
  assert.match(source, /syncAudioOutputSourceLatest\(true, true\)/);
  assert.match(source, /clockRecoveryRef\.current\.cancel\(\)/);
  assert.match(source, /invokePlaybackSnapshot\('pause_playback'\)/);
  assert.match(source, /loadingEvents\.has\(event\.type\) && !loadingEpisode/);
  assert.match(source, /const playbackClockBlocked = playbackRequested && !playbackActive/);
  assert.match(source, /resetPlaybackClockObservation\(/);
  assert.match(source, /if \(previousIdentity !== expectedIdentity\)/);
  assert.match(source, /previousHealth\.status !== 'stalled' && result\.state\.status === 'stalled'/);
});
