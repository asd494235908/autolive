import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = new URL('../third_party/akvirtualcamera/sidecar/src/main.cpp', import.meta.url);
const cmake = new URL('../third_party/akvirtualcamera/sidecar/CMakeLists.txt', import.meta.url);
const build = new URL('../third_party/akvirtualcamera/build-sidecar.ps1', import.meta.url);
const securityPatch = new URL('../third_party/akvirtualcamera/patches/0002-loopback-service-socket.patch', import.meta.url);

test('sidecar 保持 x64/YUY2，兼容 v1 720p 并支持有界 v2 源尺寸', async () => {
  const text = await readFile(source, 'utf8');
  const format = await readFile(new URL('../third_party/akvirtualcamera/sidecar/src/frame_format.h', import.meta.url), 'utf8');
  assert.match(text, /--session-token-stdin/);
  assert.match(text, /readSessionTokenFromStdin/);
  assert.match(text, /first == 0xEF/);
  assert.match(text, /std::fgetc\(stdin\) != 0xBB/);
  assert.match(text, /std::fgetc\(stdin\) != 0xBF/);
  assert.doesNotMatch(text, /wcscmp\(argv\[1\], L"--session-token"\)/);
  assert.match(text, /GpAutoLive Camera/);
  assert.match(text, /kDeviceId\[\] = "GpAutoLiveCamera"/);
  assert.match(text, /vcam_capi\.dll/);
  assert.match(text, /LOAD_LIBRARY_SEARCH_APPLICATION_DIR/);
  assert.match(text, /PIPE_REJECT_REMOTE_CLIENTS/);
  assert.match(text, /kFramePayloadBytes = 1280u \* 720u \* 2u/);
  assert.match(format, /version == 1 \|\| version == 2/);
  assert.match(format, /version != 1 \|\| \(width == 1280 && height == 720\)/);
  assert.match(format, /width % 2 == 0/);
  assert.match(text, /static_cast<int>\(width\)/);
  assert.match(text, /static_cast<int>\(height\)/);
  assert.match(text, /sessionWidth != width \|\| sessionHeight != height/);
  assert.match(text, /sent && !firstFrameSent/);
  assert.match(text, /GPAKVC_OUTPUT_READY/);
  assert.match(text, /"YUY2"/);
  assert.match(text, /vcam_data_mode/);
  assert.match(text, /vcam_direct_mode/);
  assert.doesNotMatch(text, /vcam_set_data_mode/);
  assert.doesNotMatch(text, /vcam_set_direct_mode/);
  assert.match(text, /UAC prompt/);
  assert.match(text, /vcam_clients/);
  assert.match(text, /GPAKVC_CLIENTS/);
  assert.match(text, /fflush\(stdout\)/);
  assert.doesNotMatch(text, /CreateProcess|ShellExecute|system\s*\(/);
});

test('开发尺寸写入只触及专用格式，保留正式路径与下游重开边界', async () => {
  const text = await readFile(source, 'utf8');
  const device = await readFile(new URL('../third_party/akvirtualcamera/sidecar/src/device_format.h', import.meta.url), 'utf8');
  assert.match(text, /!matchingFormat && development && clientCount\(\) != 0/);
  assert.match(text, /--development-format/);
  assert.match(device, /GpAutoLiveCamera/);
  assert.match(device, /installPath/);
  assert.match(device, /KEY_WOW64_64KEY/);
  assert.match(device, /development \? KEY_SET_VALUE : 0/);
  assert.match(device, /GPAKVC_FORMAT_ROLLBACK_FAILED/);
  assert.doesNotMatch(device, /KEY_ALL_ACCESS|RegCreateKey|ShellExecute/);
});

test('sidecar 构建入口保持离线且不安装系统设备', async () => {
  const cmakeText = await readFile(cmake, 'utf8');
  const buildText = await readFile(build, 'utf8');
  const patchText = await readFile(securityPatch, 'utf8');
  const installerText = await readFile(new URL('../third_party/akvirtualcamera/installer/akvirtualcamera-components.nsh', import.meta.url), 'utf8');
  assert.match(cmakeText, /OUTPUT_NAME akvirtualcamera-sidecar-x64/);
  assert.match(cmakeText, /RUNTIME DESTINATION \$\{BINDIR\}/);
  assert.match(cmakeText, /target_compile_features/);
  assert.match(cmakeText, /target_compile_options\(AkVirtualCameraSidecar PRIVATE \/utf-8\)/);
  assert.match(buildText, /CMAKE_PROJECT_INCLUDE/);
  assert.match(buildText, /network = 'none'/);
  assert.match(buildText, /release_ready = \$false/);
  assert.match(buildText, /0002-loopback-service-socket\.patch/);
  assert.match(buildText, /apply --check/);
  assert.match(buildText, /apply --reverse --check/);
  assert.match(patchText, /INADDR_ANY/);
  assert.match(patchText, /INADDR_LOOPBACK/);
  assert.match(installerText, /set-data-mode mmap/);
  assert.doesNotMatch(buildText, /Invoke-WebRequest|Start-BitsTransfer|git\s+clone/);
});
