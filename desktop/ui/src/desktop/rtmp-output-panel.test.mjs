import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('RTMP 推流面板使用受限地址和轨道配置，不捕获桌面', async () => {
  const panel = await readFile(new URL('./rtmp-output-panel.tsx', import.meta.url), 'utf8');
  const app = await readFile(new URL('../App.tsx', import.meta.url), 'utf8');

  assert.match(panel, /RTMP 或 RTMPS 推流地址/);
  assert.match(panel, /rtmp:\/\/127\.0\.0\.1\/live\/stream/);
  assert.match(panel, /正式需求·待实施\/未接入/);
  assert.match(panel, /PortAudio/);
  assert.match(panel, /checked=\{config\.video_enabled\}/);
  assert.match(panel, /checked=\{config\.audio_enabled\}/);
  assert.match(panel, /case 'original':/);
  assert.match(panel, /原始链/);
  assert.doesNotMatch(panel, /getUserMedia|desktopCapture|captureDesktop|windowCapture/);

  assert.match(app, /validate_rtmp_output_config/);
  assert.match(app, /get_rtmp_output_status/);
  assert.match(app, /start_rtmp_output/);
  assert.match(app, /stop_rtmp_output/);
});
