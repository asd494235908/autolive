import { invoke } from '@tauri-apps/api/core';
import type { UserSummaryDto } from './controlPlaneClient';

export type ControlPlaneAccessSession = {
  access_token: string;
  expires_at: string;
  audience: 'desktop';
  user: UserSummaryDto | null;
  warning: string | null;
};

type DeviceSessionRequest = {
  device_id: string;
};

type StableDeviceIdRequest = {
  migration_device_id: string;
};

type LoginSessionRequest = DeviceSessionRequest & {
  username: string;
  password: string;
};

function isUserSummary(value: unknown): value is UserSummaryDto {
  if (!value || typeof value !== 'object') return false;
  const user = value as Partial<UserSummaryDto>;
  return typeof user.id === 'string'
    && typeof user.username === 'string'
    && user.role === 'user'
    && (user.status === 'active' || user.status === 'disabled')
    && typeof user.created_at === 'string';
}

function parseAccessSession(value: unknown, nullable: boolean): ControlPlaneAccessSession | null {
  if (value === null && nullable) return null;
  if (!value || typeof value !== 'object') throw new Error('桌面会话响应无效');
  const session = value as Partial<ControlPlaneAccessSession>;
  if (
    typeof session.access_token !== 'string'
    || session.access_token.length === 0
    || session.access_token.length > 16 * 1024
    || typeof session.expires_at !== 'string'
    || !Number.isFinite(Date.parse(session.expires_at))
    || session.audience !== 'desktop'
    || (session.user !== null && !isUserSummary(session.user))
    || (session.warning !== null && typeof session.warning !== 'string')
  ) {
    throw new Error('桌面会话响应无效');
  }
  return session as ControlPlaneAccessSession;
}

export async function getOrCreateControlPlaneDeviceId(migrationDeviceId: string): Promise<string> {
  const deviceId: unknown = await invoke<unknown>('get_or_create_control_plane_device_id', {
    request: { migration_device_id: migrationDeviceId } satisfies StableDeviceIdRequest,
  });
  if (typeof deviceId !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(deviceId)) {
    throw new Error('稳定设备标识响应无效');
  }
  return deviceId;
}

export async function loginControlPlaneSession(
  deviceId: string,
  username: string,
  password: string,
): Promise<ControlPlaneAccessSession> {
  const value: unknown = await invoke<unknown>('login_control_plane_session', {
    request: { device_id: deviceId, username, password } satisfies LoginSessionRequest,
  });
  return parseAccessSession(value, false) as ControlPlaneAccessSession;
}

export async function restoreControlPlaneSession(deviceId: string): Promise<ControlPlaneAccessSession | null> {
  const value: unknown = await invoke<unknown>('restore_control_plane_session', {
    request: { device_id: deviceId } satisfies DeviceSessionRequest,
  });
  return parseAccessSession(value, true);
}

export async function refreshControlPlaneSession(deviceId: string): Promise<ControlPlaneAccessSession | null> {
  const value: unknown = await invoke<unknown>('refresh_control_plane_session', {
    request: { device_id: deviceId } satisfies DeviceSessionRequest,
  });
  return parseAccessSession(value, true);
}

export async function logoutControlPlaneSession(deviceId: string): Promise<string | null> {
  const value: unknown = await invoke<unknown>('logout_control_plane_session', {
    request: { device_id: deviceId } satisfies DeviceSessionRequest,
  });
  if (!value || typeof value !== 'object') throw new Error('退出响应无效');
  const warning = (value as { warning?: unknown }).warning;
  if (warning !== null && typeof warning !== 'string') throw new Error('退出响应无效');
  return warning;
}

export async function retryPendingControlPlaneLogout(deviceId: string): Promise<string | null> {
  const warning: unknown = await invoke<unknown>('retry_pending_control_plane_logout', {
    request: { device_id: deviceId } satisfies DeviceSessionRequest,
  });
  if (warning !== null && typeof warning !== 'string') throw new Error('待撤销会话重试响应无效');
  return warning;
}

export function clearLegacyAuthCredentials(deviceId: string): Promise<void> {
  return invoke<void>('clear_legacy_auth_credentials', {
    request: { device_id: deviceId } satisfies DeviceSessionRequest,
  });
}
