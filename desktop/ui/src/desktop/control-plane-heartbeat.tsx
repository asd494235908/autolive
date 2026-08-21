import { invoke } from '@tauri-apps/api/core';
import { useEffect } from 'react';
import {
  buildIdempotencyKey,
  sendHeartbeatControlPlane,
  type HeartbeatRequestDto,
} from '../controlPlaneClient';
import {
  clearHeartbeatOutbox,
  queueLatestHeartbeat,
  readHeartbeatOutbox,
  shouldQueueHeartbeat,
} from '../heartbeatOutbox';
import type { ControlPlaneSession, ControlPlaneSessionSnapshot } from '../controlPlaneSession';

const HEARTBEAT_INTERVAL_MS = 30_000;

function resolveLocalStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

type DeviceRuntimeInfo = {
  disk_free_bytes: number;
  memory_total_bytes: number;
  memory_available_bytes: number;
  cpu_logical_cores: number;
  os_name?: string;
  os_version?: string;
  kernel_version?: string;
};

export function ControlPlaneHeartbeat({
  session,
  snapshot,
}: {
  session: ControlPlaneSession;
  snapshot: ControlPlaneSessionSnapshot;
}) {
  useEffect(() => {
    if (snapshot.status !== 'ready' || !snapshot.accessToken || !snapshot.device) return undefined;
    let cancelled = false;
    const storage = resolveLocalStorage();
    const deviceId = snapshot.device.id;

    const send = async () => {
      if (cancelled) return;
      const current = session.getSnapshot();
      if (current.status !== 'ready' || !current.accessToken || !current.device) return;
      let runtime: DeviceRuntimeInfo;
      try {
        runtime = await invoke<DeviceRuntimeInfo>('get_device_runtime_info', { request: { path: null } });
      } catch {
        return;
      }
      const queued = readHeartbeatOutbox(storage, current.user?.id ?? '', deviceId);
      if (queued) {
        try {
          await sendHeartbeatControlPlane(current.accessToken, queued.request, queued.idempotency_key);
          clearHeartbeatOutbox(storage, queued.user_id, queued.device_id, queued.idempotency_key);
        } catch (error) {
          if (shouldQueueHeartbeat(error)) return;
          clearHeartbeatOutbox(storage, queued.user_id, queued.device_id, queued.idempotency_key);
        }
      }
      const request: HeartbeatRequestDto = {
        device_id: deviceId,
        sent_at: new Date().toISOString(),
        status: {
          disk_free_bytes: runtime.disk_free_bytes,
          memory_total_bytes: runtime.memory_total_bytes,
          memory_available_bytes: runtime.memory_available_bytes,
          cpu_logical_cores: runtime.cpu_logical_cores,
          os_name: runtime.os_name,
          os_version: runtime.os_version,
          kernel_version: runtime.kernel_version,
        },
      };
      const idempotencyKey = buildIdempotencyKey();
      try {
        await sendHeartbeatControlPlane(current.accessToken, request, idempotencyKey);
      } catch (error) {
        if (shouldQueueHeartbeat(error)) {
          queueLatestHeartbeat(storage, current.user?.id ?? '', deviceId, request, idempotencyKey);
        }
      }
    };

    void send();
    const timer = window.setInterval(() => void send(), HEARTBEAT_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [session, snapshot.accessToken, snapshot.device, snapshot.status]);

  return null;
}
