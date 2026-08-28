import { clearLegacyAuthCredentials } from './controlPlaneAuth';
import {
  loadAuthFormPreferences,
  saveAuthFormPreferences,
  type AuthFormPreferences,
} from './authFormPreferences';

const EMPTY_PREFERENCES: AuthFormPreferences = {
  username: '',
  rememberLogin: false,
};

export type RememberedLogin = {
  username: string;
  remember: boolean;
};

function preferencesForMutation(deviceId: string): AuthFormPreferences {
  try {
    return loadAuthFormPreferences(deviceId);
  } catch {
    return { ...EMPTY_PREFERENCES };
  }
}

export async function loadRememberedLogin(deviceId: string): Promise<RememberedLogin> {
  const preferences = loadAuthFormPreferences(deviceId);
  if (!preferences.rememberLogin) {
    return { username: '', remember: false };
  }
  return {
    username: preferences.username,
    remember: true,
  };
}

export async function saveRememberedLogin(
  deviceId: string,
  username: string,
): Promise<void> {
  saveAuthFormPreferences(deviceId, {
    ...preferencesForMutation(deviceId),
    username,
    rememberLogin: true,
  });
}

export async function clearRememberedLogin(deviceId: string): Promise<void> {
  saveAuthFormPreferences(deviceId, {
    ...preferencesForMutation(deviceId),
    username: '',
    rememberLogin: false,
  });
}

export async function clearLegacyRememberedAuthCredentials(deviceId: string): Promise<void> {
  await clearLegacyAuthCredentials(deviceId);
  try {
    saveAuthFormPreferences(deviceId, preferencesForMutation(deviceId));
  } catch {
    // 旧偏好不是授权依据，本次运行仍继续使用内存表单状态。
  }
}
