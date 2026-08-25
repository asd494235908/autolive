import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('最终效果窗口只渲染媒体并把内部错误留给主窗口诊断', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));

  assert.doesNotMatch(finalEffectWindow, /<Alert\b/);
  assert.match(finalEffectWindow, /error:\s*playbackError \?\? finalEffectResizeError/);
  assert.match(desktopApp, /<PlaybackPoolPanel[\s\S]*error=\{error\}/);
});

test('最终效果窗口不对 Rust 提交的视频再叠加 CSS 视觉变换', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );

  assert.doesNotMatch(finalEffectWindow, /runtimeVideoStyle/);
  assert.doesNotMatch(finalEffectWindow, /filter:\s*`brightness\(/);
  assert.doesNotMatch(finalEffectWindow, /transform:\s*`translate\(/);
  assert.doesNotMatch(app, /buildRuntimePreviewParameters/);
  assert.doesNotMatch(app, /runtimePreview/);
  assert.match(finalEffectWindow, /runtimeAudioParamsRef\.current\s*=\s*event\.data\.payload/);
  assert.match(finalEffectWindow, /resolveSynchronizedVideoPlaybackRate\([\s\S]*params\?\.audio_playback_speed/);
});

test('最终效果媒体源优先使用已处理视频，未提交时明确回退原视频', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const playbackVideoUrl = app.slice(
    app.indexOf('function playbackVideoUrl('),
    app.indexOf('const INTERLUDE_AUDIO_PRESET_IDS'),
  );

  assert.match(playbackVideoUrl, /current_video_source\s*===\s*'processed'/);
  assert.match(playbackVideoUrl, /snapshot\.current_video_reference/);
  assert.match(playbackVideoUrl, /snapshot\?\.source_media\?\.source_path/);
});
