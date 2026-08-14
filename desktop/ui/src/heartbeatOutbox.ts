import type { HeartbeatRequestDto } from './controlPlaneClient';

const HEARTBEAT_OUTBOX_VERSION = 1;
const HEARTBEAT_OUTBOX_KEY_PREFIX = 'autolive.desktop.heartbeat-outbox.v1:';

export interface HeartbeatOutboxEntry {
  version: 1;
  user_id: string;
  device_id: string;
  idempotency_key: string;
  request: HeartbeatRequestDto;
  queued_at: string;
}

function storageKey(userId: string, deviceId: string): string {
  return `${HEARTBEAT_OUTBOX_KEY_PREFIX}${encodeURIComponent(userId)}:${encodeURIComponent(deviceId)}`;
}

export function readHeartbeatOutbox(
  storage: Storage | null,
  userId: string,
  deviceId: string,
): HeartbeatOutboxEntry | null {
  if (!storage || !userId || !deviceId) {
    return null;
  }

  try {
    const raw = storage.getItem(storageKey(userId, deviceId));
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw) as Partial<HeartbeatOutboxEntry>;
    if (
      parsed.version !== HEARTBEAT_OUTBOX_VERSION ||
      parsed.user_id !== userId ||
      parsed.device_id !== deviceId ||
      typeof parsed.idempotency_key !== 'string' ||
      !parsed.request ||
      parsed.request.device_id !== deviceId ||
      typeof parsed.queued_at !== 'string'
    ) {
      storage.removeItem(storageKey(userId, deviceId));
      return null;
    }
    return parsed as HeartbeatOutboxEntry;
  } catch {
    return null;
  }
}

export function queueLatestHeartbeat(
  storage: Storage | null,
  userId: string,
  deviceId: string,
  request: HeartbeatRequestDto,
  idempotencyKey: string,
  now = new Date().toISOString(),
): HeartbeatOutboxEntry | null {
  if (!storage || !userId || !deviceId || request.device_id !== deviceId || !idempotencyKey) {
    return null;
  }

  const entry: HeartbeatOutboxEntry = {
    version: 1,
    user_id: userId,
    device_id: deviceId,
    idempotency_key: idempotencyKey,
    request,
    queued_at: now,
  };

  try {
    storage.setItem(storageKey(userId, deviceId), JSON.stringify(entry));
    return entry;
  } catch {
    return null;
  }
}

export function clearHeartbeatOutbox(
  storage: Storage | null,
  userId: string,
  deviceId: string,
  idempotencyKey?: string,
): void {
  if (!storage || !userId || !deviceId) {
    return;
  }

  const existing = readHeartbeatOutbox(storage, userId, deviceId);
  if (idempotencyKey && existing?.idempotency_key !== idempotencyKey) {
    return;
  }
  try {
    storage.removeItem(storageKey(userId, deviceId));
  } catch {
    // 本地存储不可用时不阻断心跳成功结果。
  }
}

export function shouldQueueHeartbeat(error: unknown): boolean {
  if (!error || typeof error !== 'object' || !('status' in error)) {
    return true;
  }
  const status = (error as { status?: unknown }).status;
  return status === 408 || status === 429 || (typeof status === 'number' && status >= 500);
}
