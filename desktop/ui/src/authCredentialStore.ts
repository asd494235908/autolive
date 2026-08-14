import { invoke } from '@tauri-apps/api/core';

interface DeviceCredentialRequestDto {
  device_id: string;
}

interface StoreRefreshTokenRequestDto extends DeviceCredentialRequestDto {
  refresh_token: string;
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
