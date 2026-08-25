import {
  activateDeviceControlPlane,
  ControlPlaneError,
  getClientProfileControlPlane,
  loginControlPlane,
  logoutControlPlane,
  refreshControlPlane,
  clearSessionRefreshHandler,
  setSessionRefreshHandler,
} from './controlPlaneClient';
import type {
  ClientProfileResponseDto,
  DeviceRegistrationDto,
  DeviceSummaryDto,
  LoginResponseDto,
  RefreshTokenResponseDto,
  UserSummaryDto,
} from './controlPlaneClient';
import { deleteRefreshToken, loadRefreshToken, storeRefreshToken } from './authCredentialStore';
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
  user: UserSummaryDto | null;
  device: DeviceSummaryDto | null;
  error: unknown | null;
  warning: string | null;
};

export type ControlPlaneSessionListener = (snapshot: ControlPlaneSessionSnapshot) => void;

const INITIAL_SNAPSHOT: ControlPlaneSessionSnapshot = {
  status: 'loading',
  accessToken: null,
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
  return error instanceof ControlPlaneError && error.status === 401;
}

function isCredentialStorageError(error: unknown): boolean {
  if (!error || typeof error !== 'object' || !('code' in error)) return false;
  const code = (error as { code?: unknown }).code;
  return typeof code === 'string' && code.startsWith('auth_credential_');
}

export function getControlPlaneErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ControlPlaneError && error.message.trim()) return error.message;
  if (error instanceof Error && error.message.trim()) return error.message;
  return fallback;
}

export class ControlPlaneSession {
  private readonly listeners = new Set<ControlPlaneSessionListener>();
  private snapshot: ControlPlaneSessionSnapshot = INITIAL_SNAPSHOT;
  private deviceId = '';
  private deviceRegistration: DeviceRegistrationDto | null = null;
  private refreshToken: string | null = null;
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
      const refreshToken = await loadRefreshToken(device.device_id);
      if (!this.isCurrent(generation)) return this.snapshot;
      if (!refreshToken) {
        this.refreshToken = null;
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated' });
        return this.snapshot;
      }
      this.refreshToken = refreshToken;
      const tokens = await refreshControlPlane(refreshToken);
      if (!this.isCurrent(generation)) return this.snapshot;
      const warning = await this.persistRefreshToken(device.device_id, tokens.tokens.refresh_token);
      if (!this.isCurrent(generation)) return this.snapshot;
      this.refreshToken = tokens.tokens.refresh_token;
      if (warning) this.publish({ ...this.snapshot, warning });
      await this.resolveProfile(generation, tokens.tokens.access_token, null);
    } catch (error) {
      if (!this.isCurrent(generation)) return this.snapshot;
      if (isInvalidSessionError(error)) {
        await this.clearStoredCredential(device.device_id);
        if (!this.isCurrent(generation)) return this.snapshot;
      }
      this.refreshToken = isInvalidSessionError(error) ? null : this.refreshToken;
      const credentialUnavailable = isCredentialStorageError(error);
      this.publish({
        ...this.snapshot,
        status: isInvalidSessionError(error) || credentialUnavailable ? 'unauthenticated' : 'error',
        accessToken: isInvalidSessionError(error) || credentialUnavailable ? null : this.snapshot.accessToken,
        error: credentialUnavailable ? null : error,
        warning: credentialUnavailable ? '系统钥匙串不可用，登录后仅保留本次运行会话。' : null,
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
      const response = await loginControlPlane({ username, password, product: 'autolive' });
      if (!this.isCurrent(generation)) return this.snapshot;
      const warning = await this.persistTokens(device.device_id, response);
      if (!this.isCurrent(generation)) return this.snapshot;
      if (warning) this.publish({ ...this.snapshot, warning });
      await this.resolveProfile(generation, response.tokens.access_token, response.user);
    } catch (error) {
      if (this.isCurrent(generation)) this.publish({ ...this.snapshot, status: 'unauthenticated', error });
    }
    return this.snapshot;
  }

  async logout(): Promise<void> {
    ++this.generation;
    const accessToken = this.snapshot.accessToken;
    const refreshToken = this.refreshToken;
    const deviceId = this.deviceId;
    const localSnapshot: ControlPlaneSessionSnapshot = {
      ...INITIAL_SNAPSHOT,
      status: 'unauthenticated',
      warning: null,
    };
    this.refreshToken = null;
    this.publish(localSnapshot);
    await this.clearStoredCredential(deviceId);
    try {
      if (accessToken) await logoutControlPlane(accessToken, refreshToken ?? undefined);
    } catch {
      // 本地会话必须清理，即使控制面当前不可达。
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
    const refreshToken = this.refreshToken;
    const deviceId = this.deviceId;
    if (!refreshToken || !deviceId) return null;
    try {
      const response = await refreshControlPlane(refreshToken);
      if (!this.isCurrent(generation)) return null;
      const warning = await this.persistRefreshToken(deviceId, response.tokens.refresh_token);
      if (!this.isCurrent(generation)) return null;
      this.refreshToken = response.tokens.refresh_token;
      this.publish({ ...this.snapshot, accessToken: response.tokens.access_token, error: null, warning: warning ?? this.snapshot.warning });
      return response.tokens.access_token;
    } catch (error) {
      if (isInvalidSessionError(error)) {
        if (!this.isCurrent(generation)) return null;
        await this.clearStoredCredential(deviceId);
        if (!this.isCurrent(generation)) return null;
        this.refreshToken = null;
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated', error });
      }
      return null;
    }
  }

  private async resolveProfile(
    generation: number,
    accessToken: string,
    fallbackUser: UserSummaryDto | null,
  ): Promise<void> {
    if (!this.isCurrent(generation)) return;
    this.publish({
      ...this.snapshot,
      status: 'loading',
      accessToken,
      user: fallbackUser ?? this.snapshot.user,
      error: null,
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
      this.publish(this.snapshotFromProfile(accessToken, profile));
    } catch (error) {
      if (!this.isCurrent(generation)) return;
      if (isActivationRequiredError(error) || isDeviceAuthorizationError(error)) {
        this.publish({ ...this.snapshot, status: 'activation_required', accessToken, error });
        return;
      }
      if (isInvalidSessionError(error)) {
        await this.clearStoredCredential(this.deviceId);
        if (!this.isCurrent(generation)) return;
        this.refreshToken = null;
        this.publish({ ...INITIAL_SNAPSHOT, status: 'unauthenticated', error });
        return;
      }
      this.publish({ ...this.snapshot, status: 'error', accessToken, error });
    }
  }

  private snapshotFromProfile(accessToken: string, profile: ClientProfileResponseDto): ControlPlaneSessionSnapshot {
    const confirmed = isConfirmedDesktopAccess(profile.user, profile.device, this.deviceId);
    return {
      status: confirmed ? 'ready' : 'activation_required',
      accessToken,
      user: profile.user,
      device: profile.device,
      error: confirmed ? null : new Error('当前设备授权状态无效或已过期，请联系管理员'),
      warning: this.snapshot.warning,
    };
  }

  private async persistTokens(deviceId: string, response: LoginResponseDto | { tokens: RefreshTokenResponseDto['tokens'] }): Promise<string | null> {
    const warning = await this.persistRefreshToken(deviceId, response.tokens.refresh_token);
    this.refreshToken = response.tokens.refresh_token;
    return warning;
  }

  private async persistRefreshToken(deviceId: string, refreshToken: string): Promise<string | null> {
    try {
      await storeRefreshToken(deviceId, refreshToken);
      return null;
    } catch {
      return '系统钥匙串不可用，登录后仅保留本次运行会话。';
    }
  }

  private async clearStoredCredential(deviceId: string): Promise<void> {
    if (!deviceId) return;
    try {
      await deleteRefreshToken(deviceId);
    } catch {
      // 钥匙串不可用时保留错误边界，但不能阻止清空内存会话。
    }
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
