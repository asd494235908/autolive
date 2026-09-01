export type MicrophoneInterludeState =
  | 'disabled'
  | 'opening'
  | 'armed'
  | 'speaking'
  | 'hangover'
  | 'stopping'
  | 'failed';

export type MicrophoneSensitivity = 'low' | 'standard' | 'high';

export const MICROPHONE_SENSITIVITY_OPTIONS = [
  { value: 'low', label: '低' },
  { value: 'standard', label: '标准' },
  { value: 'high', label: '高' },
] as const satisfies ReadonlyArray<{ value: MicrophoneSensitivity; label: string }>;

export type MicrophoneInputDevice = {
  id: string;
  name: string;
  host_api: string;
  max_input_channels: number;
  default_sample_rate_hz: number;
};

export type MicrophoneInterludeStatus = {
  state: MicrophoneInterludeState;
  generation: number;
  selected_device_id: string | null;
  selected_device_name: string | null;
  host_api: string | null;
  actual_sample_rate_hz: number | null;
  input_level: number;
  speech_probability: number;
  aec_active: boolean;
  noise_suppression_active: boolean;
  agc_active: boolean;
  input_overflow_count: number;
  output_underflow_count: number;
  dropped_frame_count: number;
  media_muted: boolean;
  error_code: string | null;
  error_message: string | null;
};

export type MicrophoneInterludeView = {
  stateLabel: string;
  stateColor: 'default' | 'processing' | 'success' | 'warning' | 'error';
  detail: string;
  available: boolean;
  listening: boolean;
  speaking: boolean;
  mediaMuted: boolean;
  inputLevelPercent: number;
  sampleRateLabel: string;
  error: string | null;
};

/**
 * 最终效果窗口的本地优先级通知。
 * 麦克风状态本身由 Rust 控制面维护，这条消息只负责让独立的 WebView
 * 播放窗口同步停止固定话术和插话文件，不承载音频数据或用户语音。
 */
export type MicrophonePriorityMessage = {
  version: 1;
  type: 'microphone-priority';
  speaking: boolean;
};

/**
 * 最终效果窗口启动后请求当前的优先级快照，避免错过主页已经发送的状态边沿。
 * 请求和响应都只包含布尔状态，不承载音频或用户语音。
 */
export type MicrophonePriorityRequestMessage = {
  version: 1;
  type: 'microphone-priority-request';
};

export function isMicrophonePriorityMessage(value: unknown): value is MicrophonePriorityMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const message = value as Record<string, unknown>;
  return message.version === 1
    && message.type === 'microphone-priority'
    && typeof message.speaking === 'boolean';
}

export function isMicrophonePriorityRequestMessage(
  value: unknown,
): value is MicrophonePriorityRequestMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const message = value as Record<string, unknown>;
  return message.version === 1 && message.type === 'microphone-priority-request';
}

const MICROPHONE_STATES = new Set<MicrophoneInterludeState>([
  'disabled',
  'opening',
  'armed',
  'speaking',
  'hangover',
  'stopping',
  'failed',
]);

const MICROPHONE_HOST_APIS = new Set(['default', 'wasapi', 'asio', 'mme', 'dsound', 'wdmks', 'other']);

export function isMicrophoneSensitivity(value: unknown): value is MicrophoneSensitivity {
  return value === 'low' || value === 'standard' || value === 'high';
}

function isBoundedString(value: unknown, allowEmpty = true): value is string {
  return typeof value === 'string'
    && value.length <= 512
    && (allowEmpty || value.length > 0)
    && !/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/u.test(value);
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isNullableBoundedString(value: unknown): value is string | null {
  return value === null || isBoundedString(value);
}

function isNullableSampleRate(value: unknown): value is number | null {
  return value === null || (
    isSafeNonNegativeInteger(value)
    && value >= 8_000
    && value <= 384_000
  );
}

function isProbability(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isFinite(value)
    && value >= 0
    && value <= 1;
}

export function isMicrophoneInputDeviceList(value: unknown): value is MicrophoneInputDevice[] {
  if (!Array.isArray(value) || value.length > 512) return false;
  return value.every((item) => {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return false;
    const device = item as Record<string, unknown>;
    return isBoundedString(device.id, false)
      && isBoundedString(device.name, false)
      && isBoundedString(device.host_api, false)
      && MICROPHONE_HOST_APIS.has(device.host_api.trim().toLowerCase())
      && isSafeNonNegativeInteger(device.max_input_channels)
      && device.max_input_channels > 0
      && device.max_input_channels <= 32
      && isSafeNonNegativeInteger(device.default_sample_rate_hz)
      && device.default_sample_rate_hz >= 8_000
      && device.default_sample_rate_hz <= 384_000;
  });
}

export function isSelectedMicrophoneDeviceAvailable(
  deviceId: string | null,
  devices: readonly MicrophoneInputDevice[],
): boolean {
  return deviceId === null || devices.some((device) => device.id === deviceId);
}

export function isMicrophoneInterludeStatus(value: unknown): value is MicrophoneInterludeStatus {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const status = value as Record<string, unknown>;
  return typeof status.state === 'string'
    && MICROPHONE_STATES.has(status.state as MicrophoneInterludeState)
    && isSafeNonNegativeInteger(status.generation)
    && isNullableBoundedString(status.selected_device_id)
    && isNullableBoundedString(status.selected_device_name)
    && isNullableBoundedString(status.host_api)
    && isNullableSampleRate(status.actual_sample_rate_hz)
    && typeof status.input_level === 'number'
    && Number.isFinite(status.input_level)
    && status.input_level >= 0
    && status.input_level <= 1
    && isProbability(status.speech_probability)
    && typeof status.aec_active === 'boolean'
    && typeof status.noise_suppression_active === 'boolean'
    && typeof status.agc_active === 'boolean'
    && isSafeNonNegativeInteger(status.input_overflow_count)
    && isSafeNonNegativeInteger(status.output_underflow_count)
    && isSafeNonNegativeInteger(status.dropped_frame_count)
    && typeof status.media_muted === 'boolean'
    && isNullableBoundedString(status.error_code)
    && isNullableBoundedString(status.error_message);
}

export function projectMicrophoneInterludeStatus(
  status: MicrophoneInterludeStatus | null | undefined,
): MicrophoneInterludeView {
  if (!status) {
    return {
      stateLabel: '未取得',
      stateColor: 'default',
      detail: '尚未取得麦克风状态，请稍后重试。',
      available: false,
      listening: false,
      speaking: false,
      mediaMuted: false,
      inputLevelPercent: 0,
      sampleRateLabel: '未取得',
      error: null,
    };
  }

  const stateLabel: Record<MicrophoneInterludeState, string> = {
    disabled: '未监听',
    opening: '正在打开',
    armed: '监听中',
    speaking: '检测到说话',
    hangover: '等待尾音',
    stopping: '正在停止',
    failed: '故障',
  };
  const stateColor: MicrophoneInterludeView['stateColor'] = status.state === 'failed'
    ? 'error'
    : status.state === 'speaking' || status.state === 'hangover'
      ? 'success'
      : status.state === 'opening' || status.state === 'stopping'
        ? 'processing'
        : 'default';

  return {
    stateLabel: stateLabel[status.state],
    stateColor,
    detail: status.error_message
      ?? (status.state === 'disabled' ? '未开始监听。' : '麦克风仅在本机内存中处理，不录音、不上传。'),
    available: status.state !== 'failed',
    listening: ['opening', 'armed', 'speaking', 'hangover', 'stopping'].includes(status.state),
    speaking: status.state === 'speaking' || status.state === 'hangover',
    mediaMuted: status.media_muted,
    inputLevelPercent: Math.round(status.input_level * 100),
    sampleRateLabel: status.actual_sample_rate_hz ? `${status.actual_sample_rate_hz} Hz` : '未取得',
    error: status.error_message,
  };
}

export const MICROPHONE_AUDIO_PRIORITY_LABEL = '麦克风说话 > 固定话术 > 插话文件 > 主媒体';
