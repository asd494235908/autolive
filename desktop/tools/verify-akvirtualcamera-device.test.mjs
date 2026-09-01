import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('AkVirtualCamera 设备验收脚本只读检查固定设备并对未运行门禁 fail-closed', async () => {
  const source = await readFile(new URL('../third_party/akvirtualcamera/verify-akvirtualcamera-device.ps1', import.meta.url), 'utf8');
  assert.match(source, /GpAutoLive Camera/);
  assert.match(source, /GpAutoLiveCamera/);
  assert.match(source, /Get-PnpDevice/);
  assert.match(source, /AkVCamManager\.exe/);
  assert.match(source, /x64 注册表所有者/);
  assert.match(source, /x86 注册表所有者/);
  assert.match(source, /InstallRoot 必须是绝对路径/);
  assert.match(source, /testPattern = .*not_run/s);
  assert.match(source, /downstream = .*not_run/s);
  assert.match(source, /cleanup = .*not_run/s);
  assert.match(source, /exit 2/);
  assert.doesNotMatch(source, /regsvr32|add-device|remove-device/);
});
