import {
  activateDeviceControlPlane,
  ControlPlaneError,
  getClientProfileControlPlane,
  clearSessionRefreshHandler,
  setSessionRefreshHandler,
} from './controlPlaneClient';
import type {
  ClientProfileResponseDto,
  DeviceRegistrationDto,
  DeviceSummaryDto,
  UserSummaryDto,
} from './controlPlaneClient';
import {
  loginControlPlaneSession,
  logoutControlPlaneSession,
  refreshControlPlaneSession,
  restoreControlPlaneSession,
  retryPendingControlPlaneLogout,
} from './controlPlaneAuth';
import { isConfirmedDesktopAccess, shouldAutomaticallyRegisterDevice } from './controlPlaneGatePolicy';

export type ControlPlaneSessionStatus =
  | 'loading'
  | 'unauthenticated'
  | 'activation_required'
  | 'ready'
  | 'error';

export type ControlPlaneSessionSnapshot = {
  status: ControlPlaneSessionStatus;
  accessToken: string | null;
  accessExpiresAt: string | null;
  user: UserSummaryDto | null;
  device: DeviceSummaryDto | null;
  error: unknown | null;
  warning: string | null;
};

export type ControlPlaneSessionListener = (snapshot: ControlPlaneSessionSnapshot) => void;

const INITIAL_SNAPSHOT: ControlPlaneSessionSnapshot = {
  status: 'loading',
  accessToken: null,
  accessExpiresAt: null,
  user: null,
  device: null,
  error: null,
  warning: null,
};

const MAX_ACTIVATION_RECHECK_DELAY_MS = 2_147_000_000;

function isActivationRequiredError(error: unknown): boolean {
  if (!(error instanceof ControlPlaneError)) return false;
  return error.code === 'DEVICE_NOT_FOUND' || error.code === 'DEVICE_BINDING_REQUIRED';
}

function isDeviceAuthorizationError(error: unknown): boolean {
  if (!(error instanceof ControlPlaneError)) return false;
  return [
    'ACCOUNT_ACTIVATION_REQUIRED',
    'ACCOUNT_ACTIVATION_EXPIRED',
    'DEVICE_LIMIT_EXCEEDED',
    'DEVICE_BINDING_CONFLICT',
    'DEVICE_DISABLED',
  ].includes(error.code);
}

function isInvalidSessionError(error: unknown): boolean {
  return error instanceof ControlPlaneError && error.status === 401
    || Boolean(error && typeof error === 'object' && 'status' in error && (error as { status?: unknown }).status === 401);
}

function isCredentialStorageError(error: unknown): boolean {
  if (!error || typeof error !== 'object' || !('code' in error)) return false;
  const code = (error as { code?: unknown }).code;
  return typeof code === 'string' && code.startsWith('auth_credential_');
}

export function getControlPlaneErrorMessage(error: unknown, fallback: string): string {
  const code = readErrorString(error, 'code')?.toUpperCase();
  const requestId = error instanceof ControlPlaneError
    ? error.requestId
    : readErrorString(error, 'request_id') ?? readErrorString(error, 'requestId');
  let message = fallback;
  if (code === 'DEVICE_BINDING_CONFLICT') {
    message = '当前设备已绑定到其他账号，请联系管理员先解绑该设备，再使用当前账号登录。';
  } else if (code === 'RATE_LIMITED') {
    message = '登录尝试过于频繁，请停止重复提交并等待几分钟后再试；同一网络的其他设备连续输错密码也可能触发限制。';
  } else if (error instanceof ControlPlaneError && error.message.trim()) {
    message = error.message;
  } else if (error instanceof Error && error.message.trim()) {
    message = error.message;
  } else {
    message = readErrorString(error, 'message') ?? fallback;
  }
  return requestId ? `${message}（请求 ID：${requestId}）` : message;
}

function readErrorString(error: unknown, property: string): string | null {
  if (error && typeof error === 'object') {
    const value = (error as Record<string, unknown>)[property];
    if (typeof value === 'string' && value.trim()) return value;
  }
  return null;
}

export class ControlPlaneSession {
  private readonly listeners = new Set<ControlPlaneSessionListener>();
  private snapshot: ControlPlaneSessionSnapshot = INITIAL_SNAPSHOT;
  private deviceId = '';
  private deviceRegistration: DeviceRegistrationDto | null = null;
  private generation = 0;
  private activationRecheckTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly refreshHandler = () => this.refreshAccessToken();

  constructor() {
    setSessionRefreshHandler(this.refreshHandler);
  }

  getSnapshot(): ControlPlaneSessionSnapshot {
    return this.snapshot;
  }

  subscribe(listener: ControlPlaneSessionListener): () => void {
    this.listeners.add(listener);
    listener(this.snapshot);
    return () => this.listeners.delete(listener);
  }

  async restore(device: DeviceRegistrationDto): Promise<ControlPlaneSessionSnapshot> {
    const generation = ++this.generation;
    setSessionRefreshHandler(this.refreshHandler);
    this.deviceId = device.device_id;
    this.deviceRegistration = device;
    this.publish({ ...INITIAL_SNAPSHOT });
    try {
      const pendingLogoutWarning = await retryPendingControlPlaneLogout(device.device_id);
      if (!this.isCurrent(generation)) return this.snapshot;
      const session = await restoreControlPlaneSession(device.device_id);
      if (!this.isCurrent(generation)) return this.snapshot;
      if (!session) {
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated', warning: pendingLogoutWarning });
        return this.snapshot;
      }
      await this.resolveProfile(
        generation,
        session.access_token,
        session.expires_at,
        session.user,
        session.warning ?? pendingLogoutWarning,
      );
    } catch (error) {
      if (!this.isCurrent(generation)) return this.snapshot;
      if (isInvalidSessionError(error)) {
        await logoutControlPlaneSession(device.device_id).catch(() => undefined);
        if (!this.isCurrent(generation)) return this.snapshot;
      }
      const credentialUnavailable = isCredentialStorageError(error);
      this.publish({
        ...this.snapshot,
        status: isInvalidSessionError(error) || credentialUnavailable ? 'unauthenticated' : 'error',
        accessToken: isInvalidSessionError(error) || credentialUnavailable ? null : this.snapshot.accessToken,
        accessExpiresAt: isInvalidSessionError(error) || credentialUnavailable ? null : this.snapshot.accessExpiresAt,
        error: credentialUnavailable ? null : error,
        warning: credentialUnavailable ? '系统钥匙串不可用，无法安全恢复登录会话。' : null,
      });
    }
    return this.snapshot;
  }

  async login(username: string, password: string, device: DeviceRegistrationDto): Promise<ControlPlaneSessionSnapshot> {
    const generation = ++this.generation;
    this.deviceId = device.device_id;
    this.deviceRegistration = device;
    this.publish({ ...INITIAL_SNAPSHOT, status: 'loading' });
    try {
      const response = await loginControlPlaneSession(device.device_id, username, password);
      if (!this.isCurrent(generation)) return this.snapshot;
      await this.resolveProfile(
        generation,
        response.access_token,
        response.expires_at,
        response.user,
        response.warning,
      );
    } catch (error) {
      if (this.isCurrent(generation)) this.publish({ ...this.snapshot, status: 'unauthenticated', error });
    }
    return this.snapshot;
  }

  async logout(): Promise<void> {
    ++this.generation;
    const deviceId = this.deviceId;
    const localSnapshot: ControlPlaneSessionSnapshot = {
      ...INITIAL_SNAPSHOT,
      status: 'unauthenticated',
      warning: null,
    };
    this.publish(localSnapshot);
    try {
      const warning = deviceId ? await logoutControlPlaneSession(deviceId) : null;
      if (warning) this.publish({ ...localSnapshot, warning });
    } catch {
      this.publish({
        ...localSnapshot,
        warning: '本机内存会话已清理，但远端撤销状态未能确认。',
      });
    }
  }

  dispose(): void {
    this.generation += 1;
    this.clearActivationRecheck();
    this.listeners.clear();
    clearSessionRefreshHandler(this.refreshHandler);
  }

  private async refreshAccessToken(): Promise<string | null> {
    const generation = this.generation;
    const deviceId = this.deviceId;
    if (!deviceId) return null;
    try {
      const response = await refreshControlPlaneSession(deviceId);
      if (!this.isCurrent(generation)) return null;
      if (!response) {
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated' });
        return null;
      }
      this.publish({
        ...this.snapshot,
        accessToken: response.access_token,
        accessExpiresAt: response.expires_at,
        error: null,
        warning: response.warning ?? this.snapshot.warning,
      });
      return response.access_token;
    } catch (error) {
      if (isInvalidSessionError(error)) {
        if (!this.isCurrent(generation)) return null;
        await logoutControlPlaneSession(deviceId).catch(() => undefined);
        if (!this.isCurrent(generation)) return null;
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated', error });
      }
      return null;
    }
  }

  private async resolveProfile(
    generation: number,
    accessToken: string,
    accessExpiresAt: string,
    fallbackUser: UserSummaryDto | null,
    warning: string | null,
  ): Promise<void> {
    if (!this.isCurrent(generation)) return;
    this.publish({
      ...this.snapshot,
      status: 'loading',
      accessToken,
      accessExpiresAt,
      user: fallbackUser ?? this.snapshot.user,
      error: null,
      warning: warning ?? this.snapshot.warning,
    });
    try {
      let profile: ClientProfileResponseDto;
      let registrationAttempted = false;
      try {
        profile = await getClientProfileControlPlane(accessToken);
      } catch (error) {
        if (!isActivationRequiredError(error) || !this.deviceRegistration) throw error;
        await activateDeviceControlPlane(accessToken, { device: this.deviceRegistration });
        registrationAttempted = true;
        if (!this.isCurrent(generation)) return;
        profile = await getClientProfileControlPlane(accessToken);
      }
      if (
        !registrationAttempted
        && this.deviceRegistration
        && shouldAutomaticallyRegisterDevice(profile, this.deviceId)
      ) {
        await activateDeviceControlPlane(accessToken, { device: this.deviceRegistration });
        if (!this.isCurrent(generation)) return;
        profile = await getClientProfileControlPlane(accessToken);
      }
      if (!this.isCurrent(generation)) return;
      this.publish(this.snapshotFromProfile(accessToken, accessExpiresAt, profile));
    } catch (error) {
      if (!this.isCurrent(generation)) return;
      if (isActivationRequiredError(error) || isDeviceAuthorizationError(error)) {
        this.publish({ ...this.snapshot, status: 'activation_required', accessToken, error });
        return;
      }
      if (isInvalidSessionError(error)) {
        await logoutControlPlaneSession(this.deviceId).catch(() => undefined);
        if (!this.isCurrent(generation)) return;
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated', error });
        return;
      }
      this.publish({ ...this.snapshot, status: 'error', accessToken, error });
    }
  }

  private snapshotFromProfile(
    accessToken: string,
    accessExpiresAt: string,
    profile: ClientProfileResponseDto,
  ): ControlPlaneSessionSnapshot {
    const confirmed = isConfirmedDesktopAccess(profile.user, profile.device, this.deviceId);
    return {
      status: confirmed ? 'ready' : 'activation_required',
      accessToken,
      accessExpiresAt,
      user: profile.user,
      device: profile.device,
      error: confirmed ? null : new Error('当前设备授权状态无效或已过期，请联系管理员'),
      warning: this.snapshot.warning,
    };
  }

  private isCurrent(generation: number): boolean {
    return generation === this.generation;
  }

  private publish(snapshot: ControlPlaneSessionSnapshot): void {
    this.clearActivationRecheck();
    const guardedSnapshot = snapshot.status === 'ready'
      && !isConfirmedDesktopAccess(snapshot.user, snapshot.device, this.deviceId)
      ? {
          ...snapshot,
          status: 'activation_required' as const,
          error: new Error('设备授权状态已失效，请联系管理员'),
        }
      : snapshot;
    this.snapshot = guardedSnapshot;
    for (const listener of this.listeners) listener(guardedSnapshot);
    if (guardedSnapshot.status === 'ready') this.scheduleActivationRecheck();
  }

  private scheduleActivationRecheck(): void {
    const expiresAt = Date.parse(this.snapshot.device?.activation_expires_at ?? '');
    if (!Number.isFinite(expiresAt)) return;
    const remaining = Math.max(0, expiresAt - Date.now());
    const delay = Math.min(remaining, MAX_ACTIVATION_RECHECK_DELAY_MS);
    this.activationRecheckTimer = setTimeout(() => {
      this.activationRecheckTimer = null;
      if (this.snapshot.status !== 'ready') return;
      if (!isConfirmedDesktopAccess(this.snapshot.user, this.snapshot.device, this.deviceId)) {
        this.publish({
          ...this.snapshot,
          status: 'activation_required',
          error: new Error('设备授权已过期，请联系管理员'),
        });
        return;
      }
      this.scheduleActivationRecheck();
    }, delay);
  }

  private clearActivationRecheck(): void {
    if (this.activationRecheckTimer === null) return;
    clearTimeout(this.activationRecheckTimer);
    this.activationRecheckTimer = null;
  }
}
