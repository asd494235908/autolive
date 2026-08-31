import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');

test('最终效果窗口严格解析原生视频宿主就绪响应', () => {
  const parser = app.slice(
    app.indexOf('function parseFinalEffectWindowResult('),
    app.indexOf('function isPlaybackItemCompletionResult('),
  );

  assert.match(parser, /record\.label === 'final-effect'/);
  assert.match(parser, /typeof record\.created === 'boolean'/);
  assert.match(parser, /typeof record\.video_host_ready === 'boolean'/);
  assert.match(parser, /throw new Error\('独立播放器窗口响应无效/);
});

test('主页只在视频宿主已就绪时允许启动播放', () => {
  const openWindow = app.slice(
    app.indexOf('async function openFinalEffectWindowFromHome('),
    app.indexOf('function postPlaybackMediaControl('),
  );
  const startPlayback = app.slice(
    app.indexOf('async function startPlaybackFromHome()'),
    app.indexOf('async function updateProcessingSwitches('),
  );

  assert.match(openWindow, /invoke<unknown>\('open_final_effect_window'/);
  assert.match(openWindow, /parseFinalEffectWindowResult\(response\)/);
  assert.match(openWindow, /if \(!result\.video_host_ready\) \{[\s\S]*setError\('视频显示表面尚未就绪，请关闭最终效果窗口后重新打开再试。'\);[\s\S]*return false;/);
  assert.match(startPlayback, /const opened = await openFinalEffectWindowFromHome\(\);[\s\S]*if \(!opened\) return;[\s\S]*runPlaybackAction\('resume', 'start_playback'\)/);
});
