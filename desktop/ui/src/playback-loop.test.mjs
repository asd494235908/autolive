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

const { shouldIgnoreLoopBoundaryPause, shouldRestartPlayback } = await loadTypeScriptModule('playback-loop.ts', [
  'shouldIgnoreLoopBoundaryPause',
  'shouldRestartPlayback',
]);

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
    }),
    false,
  );
});

test('循环 seek 前先占住 pause 事件，并在最终效果窗使用边界判断', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const restartStart = source.indexOf('function restartToNextLoop');
  const restartEnd = source.indexOf('function restartAtBoundary', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);

  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(
    restartSource.indexOf('suppressMediaEventRef.current = true')
      < restartSource.indexOf('video.currentTime = 0'),
  );
  assert.match(
    restartSource,
    /video\.play\(\)\.then\(\(\) => \{\s*\/\/[^\n]*\n\s*\/\/[^\n]*\n\s*suppressMediaEventRef\.current = false;/,
  );
  assert.match(
    source,
    /shouldIgnoreLoopBoundaryPause\(\{\s*suppressMediaEvent: suppressMediaEventRef\.current,\s*ended: event\.currentTarget\.ended,/,
  );
});

test('循环提交成功后必须按新轮次零点显式重锚 PortAudio', async () => {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const restartStart = source.indexOf('function restartToNextLoop');
  const restartEnd = source.indexOf('function restartAtBoundary', restartStart);
  const restartSource = source.slice(restartStart, restartEnd);
  const completeLoop = restartSource.indexOf("invoke<PlaybackSnapshot>('complete_playback_loop'");
  const applySnapshot = restartSource.indexOf('applyPlayerSnapshot(nextSnapshot)', completeLoop);
  const reanchorPortAudio = restartSource.indexOf(
    'syncAudioOutputSourceLatest(false, true)',
    applySnapshot,
  );

  assert.ok(restartStart >= 0 && restartEnd > restartStart);
  assert.ok(completeLoop >= 0);
  assert.ok(applySnapshot > completeLoop);
  assert.ok(
    reanchorPortAudio > applySnapshot,
    '循环提交后必须用边界优先级同步新轮次的 PortAudio 源，不能被普通 N+1 候选挡住',
  );
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
    /setPlaybackError\(getDisplayErrorMessage\(cause, '播放轮次同步失败，已保持本地循环。'\)\)/,
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
