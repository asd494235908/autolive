export const MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY = 'autolive.microphone-interlude-config.v1';

export type MicrophoneInterludeSensitivity = 'low' | 'standard' | 'high';

export type PersistedMicrophoneInterludeConfig = {
  version: 1;
  device_id: string | null;
  sensitivity: MicrophoneInterludeSensitivity;
  aec_enabled: boolean;
  noise_suppression_enabled: boolean;
  agc_enabled: boolean;
};

export const DEFAULT_MICROPHONE_INTERLUDE_CONFIG: PersistedMicrophoneInterludeConfig = {
  version: 1,
  device_id: null,
  sensitivity: 'standard',
  aec_enabled: true,
  noise_suppression_enabled: true,
  agc_enabled: true,
};

export type MicrophoneInterludeConfigLoadResult = {
  config: PersistedMicrophoneInterludeConfig;
  recovered: boolean;
  error: string | null;
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

function resolveStorage(storage?: StorageLike | null): StorageLike | null {
  if (storage !== undefined) return storage;
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}

function isBoundedDeviceId(value: unknown): value is string | null {
  return value === null || (
    typeof value === 'string'
    && value.length > 0
    && value.length <= 256
    && !/[\u0000-\u001F\u007F/\\]/u.test(value)
  );
}

function isSensitivity(value: unknown): value is MicrophoneInterludeSensitivity {
  return value === 'low' || value === 'standard' || value === 'high';
}

export function isPersistedMicrophoneInterludeConfig(
  value: unknown,
): value is PersistedMicrophoneInterludeConfig {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return record.version === 1
    && isBoundedDeviceId(record.device_id)
    && isSensitivity(record.sensitivity)
    && typeof record.aec_enabled === 'boolean'
    && typeof record.noise_suppression_enabled === 'boolean'
    && typeof record.agc_enabled === 'boolean';
}

function toPersistedMicrophoneInterludeConfig(
  value: PersistedMicrophoneInterludeConfig,
): PersistedMicrophoneInterludeConfig {
  // 明确重建 DTO，避免未来误把 armed/speaking/listening 等运行时状态落盘。
  return {
    version: 1,
    device_id: value.device_id,
    sensitivity: value.sensitivity,
    aec_enabled: value.aec_enabled,
    noise_suppression_enabled: value.noise_suppression_enabled,
    agc_enabled: value.agc_enabled,
  };
}

function recovered(error: string): MicrophoneInterludeConfigLoadResult {
  return {
    config: { ...DEFAULT_MICROPHONE_INTERLUDE_CONFIG },
    recovered: true,
    error,
  };
}

export function loadMicrophoneInterludeConfig(
  storage?: StorageLike | null,
): MicrophoneInterludeConfigLoadResult {
  const target = resolveStorage(storage);
  if (!target) return recovered('麦克风配置本地存储不可用，已使用默认设置。');
  try {
    const raw = target.getItem(MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY);
    if (!raw) {
      return {
        config: { ...DEFAULT_MICROPHONE_INTERLUDE_CONFIG },
        recovered: false,
        error: null,
      };
    }
    const parsed: unknown = JSON.parse(raw);
    return isPersistedMicrophoneInterludeConfig(parsed)
      ? { config: toPersistedMicrophoneInterludeConfig(parsed), recovered: false, error: null }
      : recovered('已保存的麦克风配置损坏或版本未知，已恢复默认设置。');
  } catch {
    return recovered('已保存的麦克风配置无法读取，已恢复默认设置。');
  }
}

export function saveMicrophoneInterludeConfig(
  config: PersistedMicrophoneInterludeConfig,
  storage?: StorageLike | null,
): void {
  if (!isPersistedMicrophoneInterludeConfig(config)) {
    throw new Error('麦克风配置内容无效');
  }
  const target = resolveStorage(storage);
  if (!target) throw new Error('麦克风配置本地保存不可用');
  target.setItem(
    MICROPHONE_INTERLUDE_CONFIG_STORAGE_KEY,
    JSON.stringify(toPersistedMicrophoneInterludeConfig(config)),
  );
}
