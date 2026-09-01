import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = new URL('../third_party/akvirtualcamera/sidecar/src/main.cpp', import.meta.url);
const cmake = new URL('../third_party/akvirtualcamera/sidecar/CMakeLists.txt', import.meta.url);
const build = new URL('../third_party/akvirtualcamera/build-sidecar.ps1', import.meta.url);
const securityPatch = new URL('../third_party/akvirtualcamera/patches/0002-loopback-service-socket.patch', import.meta.url);

test('sidecar 固定为 x64、YUY2 720p30，并只从应用目录加载 C API', async () => {
  const text = await readFile(source, 'utf8');
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
