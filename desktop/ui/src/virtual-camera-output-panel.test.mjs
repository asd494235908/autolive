import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('虚拟摄像头 UI 只暴露固定设备和受限启停状态', async () => {
  const panel = await readFile(new URL('./desktop/virtual-camera-output-panel.tsx', import.meta.url), 'utf8');
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.match(panel, /GpAutoLive Camera/);
  assert.match(panel, /YUY2 1280×720@30fps/);
  assert.match(panel, /zero_copy=false/);
  assert.match(panel, /Vendor 0x/);
  assert.match(panel, /Device 0x/);
  assert.match(panel, /捕获 API/);
  assert.match(panel, /GPU 转换/);
  assert.match(panel, /adapter_luid/);
  assert.match(panel, /下游客户端/);
  assert.match(panel, /未探测/);
  assert.match(panel, /Chrome、Teams、Zoom/);
  assert.doesNotMatch(panel, /enumerate|物理摄像头.*输出|device_id.*Input/);
  assert.match(app, /get_virtual_camera_status/);
  assert.match(app, /install_or_repair_virtual_camera/);
  assert.match(app, /start_virtual_camera_output/);
  assert.match(app, /stop_virtual_camera_output/);
  assert.match(app, /windows_graphics_capture/);
  assert.match(app, /akvcam_mmap_cpu/);
  assert.match(app, /config.zero_copy !== false/);
  assert.match(app, /record.downstream_client_count > 1024/);
});
