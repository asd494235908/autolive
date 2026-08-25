import { invoke } from '@tauri-apps/api/core';

interface DeviceCredentialRequestDto {
  device_id: string;
}

interface StoreRefreshTokenRequestDto extends DeviceCredentialRequestDto {
  refresh_token: string;
}

type DeletableAuthFormCredentialKind = 'login_password' | 'activation_code';

interface AuthFormCredentialRequestDto extends DeviceCredentialRequestDto {
  kind: DeletableAuthFormCredentialKind;
}

interface StoreAuthFormCredentialRequestDto extends DeviceCredentialRequestDto {
  value: string;
}

export function storeRefreshToken(deviceId: string, refreshToken: string): Promise<void> {
  return invoke<void>('store_refresh_token', {
    request: {
      device_id: deviceId,
      refresh_token: refreshToken,
    } satisfies StoreRefreshTokenRequestDto,
  });
}

export function loadRefreshToken(deviceId: string): Promise<string | null> {
  return invoke<string | null>('load_refresh_token', {
    request: { device_id: deviceId } satisfies DeviceCredentialRequestDto,
  });
}

export function deleteRefreshToken(deviceId: string): Promise<void> {
  return invoke<void>('delete_refresh_token', {
    request: { device_id: deviceId } satisfies DeviceCredentialRequestDto,
  });
}

export function storeLoginPassword(deviceId: string, value: string): Promise<void> {
  return invoke<void>('store_auth_form_credential', {
    request: {
      device_id: deviceId,
      value,
    } satisfies StoreAuthFormCredentialRequestDto,
  });
}

export async function loadLoginPassword(deviceId: string): Promise<string | null> {
  const value: unknown = await invoke<unknown>('load_auth_form_credential', {
    request: { device_id: deviceId } satisfies DeviceCredentialRequestDto,
  });
  if (value === null) return null;
  const minimumLength = 8;
  const maximumLength = 256;
  const length = typeof value === 'string' ? Array.from(value).length : 0;
  if (
    typeof value !== 'string'
    || length < minimumLength
    || length > maximumLength
    || value.includes('\0')
  ) {
    throw new Error('系统钥匙串返回了无效的表单凭据');
  }
  return value;
}

function deleteAuthFormCredential(deviceId: string, kind: DeletableAuthFormCredentialKind): Promise<void> {
  return invoke<void>('delete_auth_form_credential', {
    request: { device_id: deviceId, kind } satisfies AuthFormCredentialRequestDto,
  });
}

export function deleteLoginPassword(deviceId: string): Promise<void> {
  return deleteAuthFormCredential(deviceId, 'login_password');
}

export function deleteLegacyActivationCode(deviceId: string): Promise<void> {
  return deleteAuthFormCredential(deviceId, 'activation_code');
}
