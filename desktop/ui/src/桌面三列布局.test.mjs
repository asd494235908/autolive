import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const audioCapabilityPath = new URL('./音频处理能力.ts', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);
const mainPath = new URL('./main.tsx', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  const audioCapability = await readFile(audioCapabilityPath, 'utf8');
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
  const hiddenVideoRef = app.indexOf('ref={pictureInPictureVideoRef}');
  const hiddenVideoStart = app.lastIndexOf('<video', hiddenVideoRef);
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
  assert.match(app, /应用视频参数/);
  assert.match(app, /应用音频参数/);
  assert.doesNotMatch(app, /应用当前处理参数/);
  assert.match(app, /video_processing_status/);
  assert.match(app, /audio_processing_status/);
  assert.match(app, /applyMediaProcessing\('video'\)/);
  assert.match(app, /applyMediaProcessing\('audio'\)/);
  assert.match(app, /applyMediaProcessing\('both'\)/);
  assert.match(app, /声音状态：\{audioProcessingStatus\}/);
  assert.match(app, /视频状态：\{videoProcessingStatus\}/);
  assert.match(app, /音频实时参数与生效状态/);
  assert.doesNotMatch(app, /title="声音处理参数"/);
  assert.doesNotMatch(app, /音频实时参数预览/);
  assert.doesNotMatch(app, /音频处理能力与生效范围/);
  assert.match(app, /audioCapabilityRows\.map/);
  assert.doesNotMatch(app, /unavailableAudioCapabilityRows/);
  assert.match(app, /基线/);
  assert.match(app, /变化/);
  for (const label of ['总增益', '输入增益', '输出增益', '响度调整', '低频 EQ', '中频 EQ', '高频 EQ', '音高', '变速', '淡入', '淡出', '轻混响', '采样率', '输出码率']) {
    assert.match(audioCapability, new RegExp(label), `audio capability exposes ${label}`);
  }
  assert.doesNotMatch(audioCapability, /buildUnavailableAudioCapabilityRows/);
  assert.match(app, /scope === 'video'[\s\S]*videoProcessingEnabled && !audioProcessingEnabled/);
  assert.match(app, /scope === 'audio'[\s\S]*audioProcessingEnabled && !videoProcessingEnabled/);
  assert.match(app, /scopeEnabled = scope === 'video'/);
  assert.match(app, /<ConfigProvider\b/);
  assert.match(app, /<AntApp>/);
  assert.doesNotMatch(main, /darkAlgorithm|colorPrimary|colorBgBase/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*28fr\)\s+minmax\(0,\s*38fr\)\s+minmax\(0,\s*34fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*800px\)/);
  assert.match(css, /grid-template-columns:\s*1fr/);
});
