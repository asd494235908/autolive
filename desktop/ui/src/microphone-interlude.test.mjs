import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const require = createRequire(import.meta.url);
const typescript = require('../node_modules/typescript');

async function readSource(path) {
  return readFile(new URL(path, import.meta.url), 'utf8');
}

async function loadMicrophoneModule() {
  const source = await readFile(new URL('./microphone-interlude.ts', import.meta.url), 'utf8');
  const transpiled = typescript.transpileModule(source, {
    compilerOptions: { module: typescript.ModuleKind.ESNext, target: typescript.ScriptTarget.ES2022 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(transpiled)}#${Date.now()}`);
}

test('麦克风插话纯函数投影未知状态并限制敏感度枚举', async () => {
  const source = await readSource('./microphone-interlude.ts');

  assert.match(source, /export type MicrophoneInterludeState/);
  assert.match(source, /'disabled'[\s\S]*'opening'[\s\S]*'armed'[\s\S]*'speaking'[\s\S]*'hangover'[\s\S]*'stopping'[\s\S]*'failed'/);
  assert.match(source, /export const MICROPHONE_SENSITIVITY_OPTIONS/);
  assert.match(source, /value: 'standard'/);
  assert.match(source, /export function projectMicrophoneInterludeStatus/);
  assert.match(source, /未取得/);
  assert.match(source, /available: false/);
});

test('麦克风插话 DTO 守卫拒绝未知响应并接受有界输入设备', async () => {
  const source = await readSource('./microphone-interlude.ts');

  assert.match(source, /export function isMicrophoneInputDeviceList/);
  assert.match(source, /max_input_channels/);
  assert.match(source, /default_sample_rate_hz/);
  assert.match(source, /value\.length > 512/);
  assert.match(source, /export function isMicrophoneInterludeStatus/);
  assert.match(source, /export function isSelectedMicrophoneDeviceAvailable/);
  assert.match(source, /selected_device_name/);
  assert.match(source, /input_level/);
  assert.match(source, /actual_sample_rate_hz/);
  assert.match(source, /error_message/);
  assert.match(source, /media_muted/);
  assert.match(source, /mediaMuted/);
  assert.match(source, /export type MicrophonePriorityMessage/);
  assert.match(source, /export function isMicrophonePriorityMessage/);
  assert.match(source, /export type MicrophonePriorityRequestMessage/);
  assert.match(source, /export function isMicrophonePriorityRequestMessage/);
});

test('首页把随机插话入口统一改为插话文件，并接入麦克风 IPC', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, />插话文件<\/Button>/);
  assert.match(app, /title="插话文件"/);
  assert.match(app, /aria-label="启用插话文件"/);
  assert.match(app, /list_portaudio_input_devices/);
  assert.match(app, /set_microphone_interlude_config/);
  assert.match(app, /start_microphone_interlude/);
  assert.match(app, /set_microphone_interlude_config', \{ config: request \}/);
  assert.match(app, /start_microphone_interlude', \{ request: \{ config: request \} \}/);
  assert.match(app, /stop_microphone_interlude/);
  assert.match(app, /get_microphone_interlude_status/);
  assert.match(app, /selected_device_name/);
  assert.match(app, /error_message/);
  assert.doesNotMatch(app, /getUserMedia/);
});

test('麦克风插话入口具备设备、灵敏度、监听操作、真实状态和隐私提示', async () => {
  const app = await readSource('./App.tsx');

  assert.match(app, /麦克风插话/);
  assert.match(app, /开始监听/);
  assert.match(app, /停止监听/);
  assert.match(app, /麦克风设备/);
  assert.match(app, /VAD 灵敏度/);
  assert.match(app, /代码已接入·待实机验收/);
  assert.match(app, /音频仅在本机内存中处理，不录音、不上传/);
  assert.match(app, /MICROPHONE_AUDIO_PRIORITY_LABEL/);
  assert.match(app, /projectMicrophoneInterludeStatus/);
  assert.match(app, /microphone-priority/);
  assert.match(app, /microphone-priority-request/);
  assert.match(app, /clearInterludePlayback\(\{ releaseMs: 0, resetSchedule: true \}\)/);
  assert.match(app, /microphonePriorityActiveRef/);
  assert.match(app, /publishFixedSpeechStatus\(message\.operation_id, 'cancelled'\)/);
  assert.match(app, /microphoneDevicesDisplayError/);
  assert.match(app, /已保存的麦克风设备当前不可用，请重新选择输入设备/);
  assert.match(app, /未检测到可用的麦克风输入设备/);
  assert.match(app, /label="输入溢出"/);
  assert.match(app, /label="输出欠载"/);
});

test('保存的麦克风设备不在最新枚举列表时保持失效并阻止静默切换', async () => {
  const { isSelectedMicrophoneDeviceAvailable } = await loadMicrophoneModule();
  const devices = [{ id: 'mic-current' }];

  assert.equal(isSelectedMicrophoneDeviceAvailable('mic-current', devices), true);
  assert.equal(isSelectedMicrophoneDeviceAvailable('mic-removed', devices), false);
  assert.equal(isSelectedMicrophoneDeviceAvailable(null, []), true);
});
