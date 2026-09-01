import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const hook = new URL('../src-tauri/windows/hooks.nsh', import.meta.url);
const componentInstaller = new URL('../third_party/akvirtualcamera/installer/akvirtualcamera-components.nsh', import.meta.url);

test('NSIS 只在 release-ready 标记存在时注册精确的 x86/x64 组件', async () => {
  const [hookText, installerText] = await Promise.all([
    readFile(hook, 'utf8'),
    readFile(componentInstaller, 'utf8'),
  ]);
  assert.match(hookText, /akvirtualcamera-components\.nsh/);
  assert.match(hookText, /GPAkVCamInstall/);
  assert.match(hookText, /GPAkVCamUninstall/);
  assert.match(installerText, /release-ready\.json/);
  assert.match(installerText, /x64\\AkVirtualCamera\.dll/);
  assert.match(installerText, /x86\\AkVirtualCamera\.dll/);
  assert.match(installerText, /x64\\AkVCamAssistant\.exe/);
  assert.match(installerText, /x64\\AkVCamManager\.exe/);
  assert.match(installerText, /bin\\akvirtualcamera-sidecar-x64\.exe/);
  assert.match(installerText, /bin\\vcam_capi\.dll/);
  assert.match(installerText, /regsvr32\.exe/);
  assert.match(installerText, /GpAutoLiveCamera/);
  assert.match(installerText, /set-data-mode mmap/);
  assert.match(installerText, /set-direct-mode/);
  assert.match(installerText, /GPAkVCamUninstallConfirmed/);
  assert.match(installerText, /请先退出 GpAutoLive/);
  assert.match(installerText, /Abort/);
  assert.match(installerText, /GPAkVCamRollbackInstall/);
  assert.match(installerText, /GPAkVCamRestoreRegistry/);
  assert.doesNotMatch(installerText, /remove-devices/);
  assert.doesNotMatch(installerText, /taskkill/);
});
