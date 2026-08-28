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

const {
  resolvePlaybackBoundaryDuration,
  shouldIgnoreLoopBoundaryPause,
  shouldRestartPlayback,
  shouldRestartCurrentSourceImmediately,
} = await loadTypeScriptModule('playback-loop.ts', [
  'resolvePlaybackBoundaryDuration',
  'shouldIgnoreLoopBoundaryPause',
  'shouldRestartPlayback',
  'shouldRestartCurrentSourceImmediately',
]);

test('视频播放使用媒体元素的有限时长作为播放池边界', () => {
  assert.equal(resolvePlaybackBoundaryDuration({
    mediaDurationSeconds: 42.5,
    sourceDurationMs: 60_000,
  }), 42.5);
});

test('只有单项池可以在后端完成响应前即时重播当前源', () => {
  assert.equal(shouldRestartCurrentSourceImmediately(0), false);
  assert.equal(shouldRestartCurrentSourceImmediately(1), true);
  assert.equal(shouldRestartCurrentSourceImmediately(2), false);
  assert.equal(shouldRestartCurrentSourceImmediately(10), false);
});

test('自然结束产生的 pause 不得暂停后端音频出口', () => {
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: true,
    }),
    true,
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: true,
      ended: false,
    }),
    true,
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: false,
      currentTime: 9.98,
      duration: 10,
    }),
    true,
    'WebView2 先发 pause、后更新 ended 时，也必须识别为自然结束',
  );
  assert.equal(
    shouldIgnoreLoopBoundaryPause({
      suppressMediaEvent: false,
      ended: false,
      currentTime: 9.9,
      duration: 10,
    }),
    false,
  );
});

test('循环 seek 前先占住 pause 事件，且最终效果窗不回写内部 pause', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const restartStart = source.indexOf('function restartCurrentPlayback');
  const restartEnd = source.indexOf('function restartToNextLoop', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);
  const finalEffectStart = source.indexOf('function FinalEffectWindow()');
  const videoStart = source.indexOf('<video', finalEffectStart);
  const finalEffectVideo = source.slice(videoStart, source.indexOf('<audio', videoStart));

  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(finalEffectStart >= 0 && videoStart > finalEffectStart);
  assert.ok(
    restartSource.indexOf('suppressMediaEventRef.current = true')
      < restartSource.indexOf('video.currentTime = 0'),
  );
  assert.match(
    restartSource,
    /video\.play\(\)\.then\(\(\) => \{\s*\/\/[^\n]*\n\s*\/\/[^\n]*\n\s*suppressMediaEventRef\.current = false;/,
  );
  assert.doesNotMatch(finalEffectVideo, /onPause=\{[\s\S]*pause_playback/);
});

test('播放项完成后应用权威快照，换源不预先重播旧源', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const restartStart = source.indexOf('function restartToNextLoop');
  const restartEnd = source.indexOf('function restartAtBoundary', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);
  const completeLoop = restartSource.indexOf("invoke<unknown>('complete_playback_item'");
  const applySnapshot = restartSource.indexOf('applyPlayerSnapshot(nextSnapshot)', completeLoop);
  const reanchorPortAudio = restartSource.indexOf(
    'syncAudioOutputSourceLatest(false, true)',
    applySnapshot,
  );

  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(completeLoop >= 0);
  assert.match(restartSource, /isPlaybackItemCompletionResult\(result\)/);
  assert.match(restartSource, /playback_generation:\s*currentSnapshot\.playback_generation/);
  assert.match(restartSource, /loop_index:\s*currentSnapshot\.loop_index/);
  assert.match(restartSource, /source_media_index:\s*currentSnapshot\.source_media_index/);
  assert.match(restartSource, /if \(restartImmediately\)[\s\S]*restartCurrentPlayback/);
  const cancelBeforeCompletion = restartSource.indexOf('cancelFixedSpeech');
  const clearBeforeCompletion = restartSource.indexOf('clearInterludePlayback');
  assert.ok(cancelBeforeCompletion >= 0 && cancelBeforeCompletion < completeLoop);
  assert.ok(clearBeforeCompletion > cancelBeforeCompletion && clearBeforeCompletion < completeLoop);
  assert.ok(applySnapshot > completeLoop);
  assert.ok(
    reanchorPortAudio > applySnapshot,
    '循环提交后必须用边界优先级同步新轮次的 PortAudio 源，不能被普通 N+1 候选挡住',
  );
});

test('播放结束推进使用源身份且不依赖旧视频流状态', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const guardStart = source.indexOf('function isCurrentFinalEffectVideo');
  const guardEnd = source.indexOf('useEffect(() => {', guardStart);
  const guardSource = source.slice(guardStart, guardEnd);

  assert.ok(guardStart >= 0 && guardEnd > guardStart);
  assert.match(guardSource, /sourceIdentityElement\?\.getAttribute\('src'\) === currentSourceIdentity/);
  assert.match(guardSource, /playbackVideoUrl\(currentSnapshot\)/);
  assert.doesNotMatch(guardSource, /VideoMse|videoMse|video_stream/);
  assert.doesNotMatch(guardSource, /video\.currentSrc/);
});

test('同一个结束事件 token 只允许重启一次', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-1',
      lastRestartToken: null,
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-1',
      lastRestartToken: 'loop-1',
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
    }),
    true,
  );
});

test('最终效果窗口只保留 mpv 实时视频入口', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.match(source, /prepare_realtime_video_plan/);
  assert.match(source, /commit_realtime_video_plan/);
  assert.match(source, /stop_realtime_video_renderer/);
  assert.doesNotMatch(source, /prepare_media_video_stream|read_media_video_stream|ack_media_video_stream|commit_media_video_stream|VideoMse|video_stream/);
});

test('循环提交未完成时拒绝 ended 和 timeupdate 重复推进本地轮次', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
      syncInFlight: true,
    }),
    false,
  );
});

test('循环提交失败时保留 Tauri 返回的真实原因', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(
    source,
    /setPlaybackError\(getDisplayErrorMessage\(cause, '播放项切换失败，已重播当前媒体。'\)\)/,
  );
});

test('接近 duration 但未结束时只在 token 更新后重启', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: 10,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-1',
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: 10,
      restartToken: 'loop-2',
      lastRestartToken: 'loop-2',
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.9,
      duration: 10,
      restartToken: 'loop-3',
      lastRestartToken: 'loop-2',
    }),
    false,
  );
});

test('旧的 generation 别名仍然兼容', () => {
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 1,
      ended: true,
      lastRestartGeneration: null,
    }),
    true,
  );
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 1,
      ended: true,
      lastRestartGeneration: 1,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      mediaGeneration: 2,
      ended: false,
      currentTime: 9.95,
      duration: 10,
      lastRestartGeneration: 1,
    }),
    true,
  );
});

test('非法输入不会触发重启', () => {
  assert.equal(
    shouldRestartPlayback({
      ended: true,
      restartToken: null,
      lastRestartToken: null,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: Number.NaN,
      duration: 10,
      restartToken: 'loop-4',
      lastRestartToken: null,
    }),
    false,
  );
  assert.equal(
    shouldRestartPlayback({
      ended: false,
      currentTime: 9.95,
      duration: Number.POSITIVE_INFINITY,
      restartToken: 'loop-4',
      lastRestartToken: null,
    }),
    false,
  );
});
