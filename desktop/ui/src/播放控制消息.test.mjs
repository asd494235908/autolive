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
  const { isPlaybackMediaStateMessage } = await loadTypeScriptModule('播放控制消息.ts', [
    'isPlaybackMediaStateMessage',
  ]);
  assert.equal(isPlaybackMediaStateMessage({
    version: 1,
    type: 'playback-media-state',
    current_time: 2,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
  }), true);
  assert.equal(isPlaybackMediaStateMessage({
    version: 1,
    type: 'playback-media-state',
    current_time: -1,
    duration: 10,
    volume: 0.8,
    muted: false,
    paused: false,
  }), false);
});

test('validates control actions and clamps media values', async () => {
  const { isPlaybackMediaControlMessage, clampMediaTime, clampVolume, formatMediaTime } =
    await loadTypeScriptModule('播放控制消息.ts', [
      'isPlaybackMediaControlMessage',
      'clampMediaTime',
      'clampVolume',
      'formatMediaTime',
    ]);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: 4,
  }), true);
  assert.equal(isPlaybackMediaControlMessage({
    version: 1,
    type: 'playback-media-control',
    action: 'seek',
    current_time: -1,
  }), false);
  assert.equal(clampMediaTime(-1, 10), 0);
  assert.equal(clampMediaTime(20, 10), 10);
  assert.equal(clampVolume(2), 1);
  assert.equal(formatMediaTime(72.4), '01:12');
});

test('uses the Rust playback position when the player window state is unavailable', async () => {
  const { resolvePlaybackPositionMs } = await loadTypeScriptModule('播放控制消息.ts', [
    'resolvePlaybackPositionMs',
  ]);
  assert.equal(resolvePlaybackPositionMs(2.4, 1_000), 2_400);
  assert.equal(resolvePlaybackPositionMs(null, 1_000), 1_000);
  assert.equal(resolvePlaybackPositionMs(undefined, undefined), null);
});

test('the final-effect video does not enable browser-native controls', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /<video[\s\S]*?\bcontrols\b[\s\S]*?\/>/);
});

test('the home page exposes media controls through the existing UI', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(source, /Slider/);
  assert.match(source, /播放进度/);
  assert.match(source, /音量/);
  assert.match(source, /静音/);
  assert.match(source, /画中画/);
  assert.match(source, /pictureInPictureVideoRef/);
});
