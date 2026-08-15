import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);
const mainPath = new URL('./main.tsx', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  const main = await readFile(mainPath, 'utf8');
  let css = '';
  try {
    css = await readFile(layoutPath, 'utf8');
  } catch {
    // Keep the failure an assertion failure until the layout stylesheet exists.
  }

  assert.match(app, /desktop-workspace/);
  assert.match(app, /desktop-column-source/);
  assert.match(app, /desktop-column-audio/);
  assert.match(app, /desktop-column-video/);
  assert.doesNotMatch(app, /单源循环播放/);
  assert.doesNotMatch(app, /不生成 N 个离线视频，不创建版本队列/);
  const sourceIndex = app.indexOf('desktop-column-source');
  const audioIndex = app.indexOf('desktop-column-audio');
  const videoIndex = app.indexOf('desktop-column-video');
  assert.ok(sourceIndex < audioIndex && audioIndex < videoIndex, 'desktop columns stay in source/audio/video order');
  const hiddenVideoStart = app.indexOf('<video\n          ref={pictureInPictureVideoRef}');
  assert.notEqual(hiddenVideoStart, -1, 'Picture-in-Picture media element remains available');
  const hiddenVideoEnd = app.indexOf('/>', hiddenVideoStart);
  const hiddenVideo = app.slice(hiddenVideoStart, hiddenVideoEnd);
  assert.match(hiddenVideo, /aria-hidden="true"/);
  assert.match(hiddenVideo, /position: 'fixed'/);
  assert.match(app, /视频实时参数预览/);
  assert.doesNotMatch(app, /title="视频实时预览"/);
  assert.doesNotMatch(app, /className="desktop-preview-video"/);
  assert.match(app, /aria-label="声音处理"/);
  assert.match(app, /aria-label="实时话术幻化"/);
  assert.match(app, /aria-label="视频处理"/);
  assert.match(app, /<ConfigProvider>/);
  assert.match(app, /<AntApp>/);
  assert.doesNotMatch(main, /darkAlgorithm|colorPrimary|colorBgBase/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*28fr\)\s+minmax\(0,\s*38fr\)\s+minmax\(0,\s*34fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*800px\)/);
  assert.match(css, /grid-template-columns:\s*1fr/);
});
