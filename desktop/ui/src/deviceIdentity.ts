const DEVICE_ID_STORAGE_KEY = 'autolive.desktop.device-id.v1';
// 与 Tauri auth_session.rs 的系统钥匙串 account 上限保持一致。
const MAX_DEVICE_ID_LENGTH = 64;

function isValidDeviceId(value: unknown): value is string {
  return typeof value === 'string'
    && value.length > 0
    && value.length <= MAX_DEVICE_ID_LENGTH
    && /^[A-Za-z0-9_-]+$/.test(value);
}

function createDeviceId(): string {
  const randomUuid = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 14)}`;
  return `desktop-${randomUuid}`;
}

function resolveDefaultStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function getOrCreateDeviceId(storage?: Storage | null): string {
  const targetStorage = storage === undefined ? resolveDefaultStorage() : storage;
  if (targetStorage) {
    try {
      const existing = targetStorage.getItem(DEVICE_ID_STORAGE_KEY);
      if (isValidDeviceId(existing)) return existing;
    } catch {
      // 本地存储不可用时仍允许本次会话完成登录；下次启动会重新生成设备标识。
    }
  }

  const deviceId = createDeviceId();
  if (targetStorage) {
    try {
      targetStorage.setItem(DEVICE_ID_STORAGE_KEY, deviceId);
    } catch {
      // 本地存储不可用时不阻断登录，但无法保证跨重启保持同一设备标识。
    }
  }
  return deviceId;
}

export function buildDeviceRegistration(deviceId: string, appVersion = __APP_VERSION__) {
  return {
    product: 'autolive' as const,
    device_id: deviceId,
    device_name: 'GPAL Desktop',
    platform: typeof navigator !== 'undefined' ? navigator.platform || 'desktop' : 'desktop',
    app_version: appVersion,
    ...(typeof navigator !== 'undefined' && navigator.userAgent ? { os_version: navigator.userAgent.slice(0, 256) } : {}),
  };
}
