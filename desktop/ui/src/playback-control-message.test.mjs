import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadTypeScriptModule(fileName, exports) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

test('accepts a valid media state and rejects malformed values', async () => {
  const { isPlaybackMediaStateMessage } = await loadTypeScriptModule('playback-control-message.ts', [
    'isPlaybackMediaStateMessage',
  ]);
  assert.equal(isPlaybackMediaStateMessage({
    version: 2,
    type: 'playback-media-state',
    current_time: 2,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
    playback_generation: 3,
    source_revision: 3,
    clock_epoch: 1,
    clock_sequence: 7,
    loop_index: 2,
    position_ms: 2_000,
    duration_ms: 10_000,
    absolute_position_ms: 22_000,
    playback_rate: 1,
  }), true);
  assert.equal(isPlaybackMediaStateMessage({
    version: 2,
    type: 'playback-media-state',
    current_time: -1,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
    playback_generation: 3,
    source_revision: 3,
    clock_epoch: 1,
    clock_sequence: 7,
    loop_index: 2,
    position_ms: 2_000,
    duration_ms: 10_000,
    absolute_position_ms: 22_000,
    playback_rate: 1,
  }), false);
  assert.equal(isPlaybackMediaStateMessage({
    version: 2,
    type: 'playback-media-state',
    current_time: 2,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
    playback_generation: 3,
    source_revision: 3,
    clock_epoch: 1,
    clock_sequence: 7,
    loop_index: 2,
    position_ms: 2_000,
    duration_ms: 10_000,
    absolute_position_ms: 22_000,
    playback_rate: Number.NaN,
  }), false);
  assert.equal(isPlaybackMediaStateMessage({
    version: 2,
    type: 'playback-media-state',
    current_time: 2,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
    playback_generation: 3,
    source_revision: 3,
    clock_epoch: 1,
    clock_sequence: 7,
    loop_index: 2,
    position_ms: 2_000,
    duration_ms: 10_000,
    absolute_position_ms: 12_000,
    playback_rate: 1,
  }), false);
});

test('validates control actions and clamps media values', async () => {
  const {
    isPlaybackMediaControlMessage,
    shouldApplyPlaybackSeek,
    clampMediaTime,
    clampVolume,
    formatMediaTime,
  } =
    await loadTypeScriptModule('playback-control-message.ts', [
      'isPlaybackMediaControlMessage',
      'shouldApplyPlaybackSeek',
      'clampMediaTime',
      'clampVolume',
      'formatMediaTime',
    ]);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: 4,
    playback_generation: 3,
  }), true);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: 4,
  }), false);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: -1,
    playback_generation: 3,
  }), false);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: 4,
    playback_generation: -1,
  }), false);
  const seek = {
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: 4,
    playback_generation: 3,
  };
  assert.equal(shouldApplyPlaybackSeek(seek, 3), true);
  assert.equal(shouldApplyPlaybackSeek(seek, 4), false);
  assert.equal(shouldApplyPlaybackSeek(seek, undefined), false);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'set-volume',
    volume: 0.5,
  }), true);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'set-playback-rate',
    playback_rate: 1.5,
  }), false);
  assert.equal(clampMediaTime(-1, 10), 0);
  assert.equal(clampMediaTime(20, 10), 10);
  assert.equal(clampVolume(2), 1);
  assert.equal(formatMediaTime(72.4), '01:12');
});

test('creates an unwrapped playback clock for PortAudio IPC', async () => {
  const { createAudioSyncClock } = await loadTypeScriptModule('playback-control-message.ts', [
    'createAudioSyncClock',
  ]);
  assert.deepEqual(createAudioSyncClock({
    playbackGeneration: 9,
    loopIndex: 1,
    positionMs: 2_700,
    durationMs: 72_300,
  }), {
    playback_generation: 9,
    loop_index: 1,
    position_ms: 2_700,
    duration_ms: 72_300,
    absolute_position_ms: 75_000,
  });
  assert.throws(() => createAudioSyncClock({
    playbackGeneration: 9,
    loopIndex: 1,
    positionMs: 72_301,
    durationMs: 72_300,
  }), RangeError);
});

test('does not reissue playback commands whose target state is already active', async () => {
  const { shouldIssuePlaybackCommand } = await loadTypeScriptModule('playback-control-message.ts', [
    'shouldIssuePlaybackCommand',
  ]);
  assert.equal(shouldIssuePlaybackCommand('start_playback', 'playing'), false);
  assert.equal(shouldIssuePlaybackCommand('resume_playback', 'playing'), false);
  assert.equal(shouldIssuePlaybackCommand('pause_playback', 'paused'), false);
  assert.equal(shouldIssuePlaybackCommand('stop_playback', 'stopped'), false);
  assert.equal(shouldIssuePlaybackCommand('pause_playback', 'playing'), true);
  assert.equal(shouldIssuePlaybackCommand('start_playback', 'ready'), true);
  assert.equal(shouldIssuePlaybackCommand('stop_playback', null), true);
});

test('playback actions preserve serialized Tauri command errors', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(source, /setError\(getDisplayErrorMessage\(cause, '更新播放状态失败'\)\)/);
  assert.doesNotMatch(source, /setError\(cause instanceof Error \? cause\.message : '更新播放状态失败'\)/);
});

test('the final-effect video does not enable browser-native controls', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /<video[\s\S]*?\bcontrols\b[\s\S]*?\/>/);
});

test('the home page exposes media controls but no manual playback-rate control', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(source, /Slider/);
  assert.match(source, /播放进度/);
  assert.match(source, /音量/);
  assert.match(source, /静音/);
  assert.match(source, /画中画/);
  assert.match(source, /pictureInPictureVideoRef/);
  assert.doesNotMatch(source, /action: 'set-playback-rate'/);
  assert.doesNotMatch(source, /video\.playbackRate = message\.playback_rate/);
  assert.match(source, /playback_generation: mediaState\.playback_generation/);
  assert.match(source, /shouldApplyPlaybackSeek\(message, snapshotRef\.current\?\.playback_generation\)/);
});

test('does not carry an old video position into the next playback generation', async () => {
  const { capturePlaybackPosition, resolvePlaybackResumePosition } = await loadTypeScriptModule(
    'playback-control-message.ts',
    ['capturePlaybackPosition', 'resolvePlaybackResumePosition'],
  );
  const previous = { playbackGeneration: 7, positionSec: 215 };
  const ignored = capturePlaybackPosition(previous, {
    playbackGeneration: 8,
    loadedPlaybackGeneration: 7,
    positionSec: 215,
    transitionInFlight: false,
  });
  assert.deepEqual(ignored, previous);
  assert.equal(resolvePlaybackResumePosition(ignored, 8), 0);

  const current = capturePlaybackPosition(ignored, {
    playbackGeneration: 8,
    loadedPlaybackGeneration: 8,
    positionSec: 3.5,
    transitionInFlight: false,
  });
  assert.deepEqual(current, { playbackGeneration: 8, positionSec: 3.5 });
  assert.equal(resolvePlaybackResumePosition(current, 8), 3.5);
  assert.equal(resolvePlaybackResumePosition(current, 9), 0);
});

test('stale audio candidates are recovered or advanced without a global error', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(
    source,
    /function isTransientAudioCandidateCode[\s\S]*audio_candidate_stale[\s\S]*audio_mixer_candidate_superseded/,
  );
  assert.match(
    source,
    /event\.data\.reason\s*&&\s*!isTransientAudioCandidateCode\(event\.data\.error_code\)/,
  );
});
