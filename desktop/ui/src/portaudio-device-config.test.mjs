import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

test('PortAudio 卡分离编辑草稿、配置目标和运行状态', async () => {
  const panel = await readSource('./desktop/portaudio-device-panel.tsx');

  assert.match(panel, /appliedDeviceId: string \| null/);
  assert.match(panel, /appliedMemoryBufferKib: number/);
  assert.match(panel, /const configurationDirty = deviceId !== appliedDeviceId[\s\S]*memoryBufferKib !== appliedMemoryBufferKib/);
  assert.match(panel, />配置目标<[\s\S]*\{appliedConfiguration\}/);
  assert.match(panel, />实际出口<[\s\S]*\{actualOutput\}/);
  assert.match(panel, />处理状态<[\s\S]*configurationDirty \? '待应用' : processingStatus/);
  assert.match(panel, />128–2048 KiB<\/Typography\.Text>/);
  assert.match(panel, /desktop-portaudio-status[\s\S]*title=\{appliedConfiguration\}/);
});

test('PortAudio 应用按钮覆盖能力、设备、缓冲、运行状态和忙碌禁用条件', async () => {
  const panel = await readSource('./desktop/portaudio-device-panel.tsx');

  assert.match(panel, /const validDevice = deviceId === null/);
  assert.match(panel, /const invalidBuffer = typeof memoryBufferKib !== 'number'/);
  assert.match(panel, /const applyDisabled = !available \|\| invalidBuffer \|\| !validDevice \|\| \(!configurationDirty && running\) \|\| busy \|\| devicesRefreshing/);
  assert.match(panel, /disabled=\{applyDisabled\}[\s\S]*onClick=\{onApply\}>应用设置<\/Button>/);
  assert.match(panel, /disabled=\{!running \|\| busy \|\| devicesRefreshing\}[\s\S]*onClick=\{onTestTone\}>播放测试音<\/Button>/);
});

test('PortAudio 兼容回退可用原配置重试，并显式提供系统默认设备', async () => {
  const app = await readSource('./App.tsx');
  const panel = await readSource('./desktop/portaudio-device-panel.tsx');

  assert.match(app, /const PORTAUDIO_FORMAL_SOURCE_SYNC_READY = true/);
  assert.match(panel, /SYSTEM_DEFAULT_OUTPUT_DEVICE_ID/);
  assert.match(panel, /label: '系统默认输出设备'/);
  assert.match(panel, /value=\{deviceId \?\? SYSTEM_DEFAULT_OUTPUT_DEVICE_ID\}/);
  assert.match(panel, /const applyDisabled = !available \|\| invalidBuffer \|\| !validDevice \|\| \(!configurationDirty && running\) \|\| busy \|\| devicesRefreshing/);
});

test('PortAudio 设备枚举失败原因保留在设备卡而不是静默清空', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /const \[audioOutputDevicesError, setAudioOutputDevicesError\]/);
  assert.match(app, /setAudioOutputDevicesError\('PortAudio 输出设备列表响应无效'\)/);
  assert.match(app, /setAudioOutputDevicesError\(getDisplayErrorMessage\(cause, 'PortAudio 输出设备枚举失败'\)\)/);
  assert.match(app, /processingStatus=\{audioOutputDevicesError \?\?/);
});

test('PortAudio 每次打开设备选择框自动刷新且失败时原子保留旧列表', async () => {
  const app = await readSource('./App.tsx');
  const panel = await readSource('./desktop/portaudio-device-panel.tsx');
  const refreshFunction = app.slice(
    app.indexOf('async function refreshAudioOutputDevices'),
    app.indexOf('function syncAudioOutputConfiguration'),
  );

  assert.match(app, /const \[audioOutputDevicesRefreshing, setAudioOutputDevicesRefreshing\] = useState\(false\)/);
  assert.match(app, /const audioOutputDevicesRefreshInFlightRef = useRef\(false\)/);
  assert.match(refreshFunction, /audioOutputDevicesRefreshInFlightRef\.current/);
  assert.match(refreshFunction, /invoke<unknown>\('list_audio_output_devices'\)/);
  assert.match(refreshFunction, /if \(!isAudioOutputDeviceList\(devices\)\)[\s\S]*setAudioOutputDevicesError\('PortAudio 输出设备列表响应无效'\)/);
  assert.match(refreshFunction, /setAudioOutputDevices\(devices\)[\s\S]*setAudioOutputDevicesError\(null\)/);
  assert.match(refreshFunction, /catch \(cause\)[\s\S]*setAudioOutputDevicesError\(getDisplayErrorMessage\(cause, 'PortAudio 输出设备枚举失败'\)\)/);
  assert.doesNotMatch(refreshFunction, /setAudioOutputDevices\(\[\]\)/);
  assert.match(app, /devicesRefreshing=\{audioOutputDevicesRefreshing\}/);
  assert.match(app, /onRefreshDevices=\{\(\) => void refreshAudioOutputDevices\(\)\}/);
  assert.match(panel, /devicesRefreshing: boolean/);
  assert.match(panel, /onRefreshDevices: \(\) => void/);
  assert.match(panel, /aria-label="PortAudio 输出设备"[\s\S]*loading=\{devicesRefreshing\}[\s\S]*onOpenChange=\{\(open\) => \{ if \(open\) onRefreshDevices\(\); \}\}/);
  assert.doesNotMatch(panel, />刷新设备<\/Button>/);
});

test('PortAudio 仅在真实应用成功后提交配置，失败保留草稿与旧配置', async () => {
  const app = await readSource('./App.tsx');
  const publishFunction = app.slice(
    app.indexOf('function publishAudioOutputBackend'),
    app.indexOf('async function applyAudioOutputBackend'),
  );
  const applyFunction = app.slice(
    app.indexOf('async function applyAudioOutputBackend'),
    app.indexOf('const [mediaEffectParams'),
  );
  const panelStart = app.indexOf('<PortAudioDevicePanel');
  const applyHandler = app.slice(
    app.indexOf('onApply={() => {', panelStart),
    app.indexOf('onTestTone={() => {', panelStart),
  );

  assert.match(app, /const \[audioOutputDeviceIdInput, setAudioOutputDeviceIdInput\]/);
  assert.match(app, /const \[audioOutputMemoryKibInput, setAudioOutputMemoryKibInput\]/);
  assert.match(app, /deviceId=\{audioOutputDeviceIdInput\}/);
  assert.match(app, /memoryBufferKib=\{audioOutputMemoryKibInput\}/);
  assert.match(app, /appliedDeviceId=\{audioOutputDeviceId\}/);
  assert.match(app, /appliedMemoryBufferKib=\{audioOutputMemoryKib\}/);
  assert.match(applyFunction, /invokeAudioOutputBackendStatus\('set_audio_output_backend'/);
  assert.match(applyFunction, /publishAudioOutputBackend\(status\);/);
  assert.match(applyFunction, /return status;/);
  assert.match(applyFunction, /catch \(cause\)[\s\S]*return null;/);
  assert.match(publishFunction, /getActualAudioOutputLabel\(status\) === 'PortAudio'[\s\S]*syncAudioOutputConfiguration\(status\)/);
  assert.doesNotMatch(applyHandler, /setAudioOutputMemoryKib\(value\)/);
  assert.match(applyHandler, /applyAudioOutputBackend\(true, audioOutputDeviceIdInput, value\)\.then\(\(status\) => \{/);
  assert.match(applyHandler, /if \(!status \|\| getActualAudioOutputLabel\(status\) !== 'PortAudio'\) return;[\s\S]*syncAudioOutputConfiguration\(status, true\);/);
});

test('PortAudio 继续使用真实设备枚举、后端应用与固定测试音命令', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /invoke<unknown>\('list_audio_output_devices'/);
  assert.match(app, /invokeAudioOutputBackendStatus\('set_audio_output_backend'/);
  assert.match(app, /invokeAudioOutputBackendStatus\('play_portaudio_test_tone',[\s\S]*frequency_hz: 440[\s\S]*duration_ms: 400/);
  assert.match(app, /processingStatus=\{audioOutputDevicesError \?\? audioOutputBackend\?\.reason \?\? \(!currentSource \? '等待媒体'/);
});
