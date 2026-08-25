export type AuthFormPreferences = {
  username: string;
  rememberLogin: boolean;
};

type AuthPreferenceStorage = Pick<Storage, 'getItem' | 'setItem'>;

const DEFAULT_AUTH_FORM_PREFERENCES: AuthFormPreferences = {
  username: '',
  rememberLogin: false,
};

function preferenceKey(deviceId: string): string {
  return `autolive.desktop.auth-form.${encodeURIComponent(deviceId)}.v1`;
}

function resolveStorage(storage?: AuthPreferenceStorage): AuthPreferenceStorage {
  if (storage) return storage;
  if (typeof window === 'undefined') throw new Error('账号偏好存储不可用');
  return window.localStorage;
}

function isAuthFormPreferences(value: unknown): value is AuthFormPreferences {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Partial<AuthFormPreferences>;
  return typeof candidate.username === 'string'
    && candidate.username.length <= 64
    && typeof candidate.rememberLogin === 'boolean';
}

export function loadAuthFormPreferences(
  deviceId: string,
  storage?: AuthPreferenceStorage,
): AuthFormPreferences {
  try {
    const serialized = resolveStorage(storage).getItem(preferenceKey(deviceId));
    if (!serialized) return { ...DEFAULT_AUTH_FORM_PREFERENCES };
    const parsed: unknown = JSON.parse(serialized);
    if (!isAuthFormPreferences(parsed)) throw new Error('invalid preference payload');
    return {
      username: parsed.username,
      rememberLogin: parsed.rememberLogin,
    };
  } catch {
    throw new Error('账号偏好读取失败');
  }
}

export function saveAuthFormPreferences(
  deviceId: string,
  preferences: AuthFormPreferences,
  storage?: AuthPreferenceStorage,
): void {
  if (!isAuthFormPreferences(preferences)) throw new Error('账号偏好内容无效');
  try {
    resolveStorage(storage).setItem(preferenceKey(deviceId), JSON.stringify(preferences));
  } catch {
    throw new Error('账号偏好保存失败');
  }
}
