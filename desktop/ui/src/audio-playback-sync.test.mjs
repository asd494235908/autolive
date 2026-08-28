import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./audio-playback-sync.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const syncModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

test('音高变化不改变视频时钟，只有音频变速与画面共用播放倍速', () => {
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(false, 1.08), 1);
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, 1.08), 1.08);
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, undefined), 1);
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, Number.NaN), 1);
});

test('浏览器播放倍速被限制在 HTMLMediaElement 的安全范围', () => {
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, 0), 1);
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, 0.1), 0.25);
  assert.equal(syncModule.resolveSynchronizedVideoPlaybackRate(true, 10), 4);
});

test('源音轨小漂移使用轻量变速，严重漂移或跨循环才硬对齐', () => {
  assert.deepEqual(syncModule.resolveSourceAudioSync(1, 10, false, false), {
    hardRealign: false,
    playbackRate: 1,
  });
  assert.deepEqual(syncModule.resolveSourceAudioSync(1, 40, false, false), {
    hardRealign: false,
    playbackRate: 0.98,
  });
  assert.deepEqual(syncModule.resolveSourceAudioSync(1, -40, false, false), {
    hardRealign: false,
    playbackRate: 1.02,
  });
  assert.deepEqual(syncModule.resolveSourceAudioSync(1, 120, false, false), {
    hardRealign: true,
    playbackRate: 1,
  });
  assert.equal(syncModule.resolveSourceAudioSync(1, 0, true, false).hardRealign, true);
  assert.equal(syncModule.resolveSourceAudioSync(1, 0, false, true).hardRealign, true);
});
