import {
  deleteLegacyActivationCode,
  deleteLoginPassword,
  loadLoginPassword,
  storeLoginPassword,
} from './authCredentialStore';
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
  password: string;
  remember: boolean;
  credentialMissing: boolean;
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
    return { username: '', password: '', remember: false, credentialMissing: false };
  }
  const password = await loadLoginPassword(deviceId);
  return {
    username: preferences.username,
    password: password ?? '',
    remember: true,
    credentialMissing: password === null,
  };
}

export async function saveRememberedLogin(
  deviceId: string,
  username: string,
  password: string,
): Promise<void> {
  await storeLoginPassword(deviceId, password);
  try {
    saveAuthFormPreferences(deviceId, {
      ...preferencesForMutation(deviceId),
      username,
      rememberLogin: true,
    });
  } catch {
    try {
      await deleteLoginPassword(deviceId);
    } catch {
      // 保存偏好失败时尽力回滚钥匙串，原始保存错误更利于界面提示。
    }
    throw new Error('记住账号和密码失败');
  }
}

export async function clearRememberedLogin(deviceId: string): Promise<void> {
  let failed = false;
  try {
    await deleteLoginPassword(deviceId);
  } catch {
    failed = true;
  }
  try {
    saveAuthFormPreferences(deviceId, {
      ...preferencesForMutation(deviceId),
      username: '',
      rememberLogin: false,
    });
  } catch {
    failed = true;
  }
  if (failed) throw new Error('清除已保存的账号和密码失败');
}

export async function clearLegacyRememberedActivationCode(deviceId: string): Promise<void> {
  try {
    await deleteLegacyActivationCode(deviceId);
  } catch {
    // 历史凭据清理不得阻断当前登录。
  }
  try {
    saveAuthFormPreferences(deviceId, preferencesForMutation(deviceId));
  } catch {
    // 旧偏好不是授权依据，本次运行仍继续使用内存表单状态。
  }
}
