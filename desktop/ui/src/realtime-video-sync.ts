export type RealtimeVideoSyncClock = {
  playback_generation: number;
  clock_epoch: number;
  loop_index: number;
  position_ms: number;
};

export type RealtimeVideoSyncBackendIdentity = {
  playback_generation: number | null;
  backend_epoch: number;
};

export type RealtimeVideoSyncRequest = {
  playback_generation: number;
  backend_epoch: number;
  clock_epoch: number;
  loop_index: number;
  position_ms: number;
  paused: boolean;
};

export type RealtimeVideoSyncObservedStatus = {
  playback_generation: number | null;
  backend_epoch: number;
  clock_epoch: number | null;
  loop_index: number | null;
  activation: string;
  process_id: number | null;
  physical_paused: boolean | null;
  eof: unknown | null;
  presented_pts_ms: number | null;
};

type RealtimeVideoPipelineObservedStatus = {
  playback_generation: number | null;
  backend_epoch: number;
  backend: string;
  activation: string;
  process_id: number | null;
  fallback_floor_mode: string;
  n: { sequence: number } | null;
  n1: { sequence: number } | null;
  n2: { sequence: number } | null;
};

export type RealtimeVideoPipelineOwner = Readonly<{
  playback_generation: number;
  backend_epoch: number;
  process_id: number;
  backend: 'realtime_gpu' | 'cpu4';
  launch_mode: string;
  active_sequence: number | null;
  next_sequence: number | null;
  planned_sequence: number | null;
}>;

type RealtimeVideoSyncErrorDisposition =
  | 'stale'
  | 'busy'
  | 'result_unknown'
  | 'transport'
  | 'permanent'
  | 'superseded';

type RealtimeVideoSyncJob = {
  revision: number;
  key: string;
  request: RealtimeVideoSyncRequest;
  retryAttempt: number;
  staleRefreshes: number;
};

export type RealtimeVideoSyncCoordinator = {
  submit: (request: RealtimeVideoSyncRequest) => void;
  dispose: () => void;
};

type RealtimeVideoSyncCoordinatorOptions = {
  invokeSync: (request: RealtimeVideoSyncRequest) => Promise<unknown>;
  readStatus: () => Promise<unknown>;
  refreshLatestDesired: () => Promise<RealtimeVideoSyncRequest | null>;
  isApplied: (status: unknown, request: RealtimeVideoSyncRequest) => boolean;
  classifyError: (cause: unknown) => RealtimeVideoSyncErrorDisposition;
  onStatus: (status: unknown) => void;
  onSettled?: (request: RealtimeVideoSyncRequest, status: unknown) => void;
  onFailure: (
    cause: unknown,
    disposition: RealtimeVideoSyncErrorDisposition,
    request: RealtimeVideoSyncRequest,
  ) => void;
  retryDelaysMs?: readonly number[];
};

export type ManagedVideoPresentedClock = {
  playback_generation: number | null;
  clock_epoch: number | null;
  loop_index: number | null;
  activation: string;
  process_id: number | null;
  presented_pts_ms: number | null;
};

export type RealtimeVideoPhysicalCommitPosition = {
  position_ms: number;
  absolute_position_ms: number;
};

function realtimeVideoPipelineOwnerKey(owner: RealtimeVideoPipelineOwner): string {
  return [
    owner.playback_generation,
    owner.backend_epoch,
    owner.process_id,
    owner.backend,
    owner.launch_mode,
  ].join(':');
}

export function stableRealtimeVideoPipelineOwner(
  status: RealtimeVideoPipelineObservedStatus | null,
): RealtimeVideoPipelineOwner | null {
  if (!status
    || (status.backend !== 'realtime_gpu' && status.backend !== 'cpu4')
    || (status.activation !== 'available' && status.activation !== 'active')
    || status.playback_generation === null
    || !Number.isSafeInteger(status.playback_generation)
    || status.playback_generation < 0
    || !Number.isSafeInteger(status.backend_epoch)
    || status.backend_epoch < 0
    || status.process_id === null
    || !Number.isSafeInteger(status.process_id)
    || status.process_id <= 0
    || status.fallback_floor_mode.length === 0) return null;
  return {
    playback_generation: status.playback_generation,
    backend_epoch: status.backend_epoch,
    process_id: status.process_id,
    backend: status.backend,
    launch_mode: status.fallback_floor_mode,
    active_sequence: status.n?.sequence ?? null,
    next_sequence: status.n1?.sequence ?? null,
    planned_sequence: status.n2?.sequence ?? null,
  };
}

export function realtimeVideoPipelineRecoveryKey(
  previous: RealtimeVideoPipelineOwner | null,
  status: RealtimeVideoPipelineObservedStatus | null,
): string | null {
  const current = stableRealtimeVideoPipelineOwner(status);
  if (current === null || status === null) return null;
  const currentKey = realtimeVideoPipelineOwnerKey(current);
  if (previous === null || previous.playback_generation !== current.playback_generation) {
    return null;
  }
  if (realtimeVideoPipelineOwnerKey(previous) !== currentKey) {
    return `pipeline:${currentKey}`;
  }
  const sustainedOrphan = previous.active_sequence === current.active_sequence
    && previous.next_sequence === null
    && current.next_sequence === null
    && current.planned_sequence !== null;
  return sustainedOrphan
    ? `pipeline:${currentKey}`
    : null;
}

export function buildRealtimeVideoPhysicalCommitPosition(
  status: RealtimeVideoSyncObservedStatus,
  expected: Pick<RealtimeVideoSyncRequest,
    'playback_generation' | 'backend_epoch' | 'clock_epoch' | 'loop_index'>,
  sourceDurationMs: number,
): RealtimeVideoPhysicalCommitPosition | null {
  if (status.playback_generation !== expected.playback_generation
    || status.backend_epoch !== expected.backend_epoch
    || status.clock_epoch !== expected.clock_epoch
    || status.loop_index !== expected.loop_index
    || (status.activation !== 'active' && status.activation !== 'available')
    || status.process_id === null
    || !Number.isSafeInteger(status.process_id)
    || status.process_id <= 0
    || status.physical_paused !== false
    || status.eof !== null
    || status.presented_pts_ms === null
    || !Number.isSafeInteger(status.presented_pts_ms)
    || status.presented_pts_ms < 0
    || !Number.isSafeInteger(sourceDurationMs)
    || sourceDurationMs <= 0
    || status.presented_pts_ms >= sourceDurationMs
    || !Number.isSafeInteger(expected.loop_index)
    || expected.loop_index < 0) return null;
  const absolutePositionMs = expected.loop_index * sourceDurationMs + status.presented_pts_ms;
  if (!Number.isSafeInteger(absolutePositionMs)) return null;
  return {
    position_ms: status.presented_pts_ms,
    absolute_position_ms: absolutePositionMs,
  };
}

export function managedVideoPresentedPosition(
  backend: ManagedVideoPresentedClock | null,
  expected: Pick<RealtimeVideoSyncClock, 'playback_generation' | 'clock_epoch' | 'loop_index'>,
  durationMs: number,
): number | null {
  if (!backend
    || (backend.activation !== 'active' && backend.activation !== 'available')
    || backend.process_id === null
    || backend.playback_generation !== expected.playback_generation
    || backend.clock_epoch !== expected.clock_epoch
    || backend.loop_index !== expected.loop_index
    || backend.presented_pts_ms === null
    || !Number.isSafeInteger(backend.presented_pts_ms)
    || backend.presented_pts_ms < 0
    || !Number.isSafeInteger(durationMs)
    || durationMs <= 0) return null;
  return Math.min(durationMs, backend.presented_pts_ms);
}

export function buildRealtimeVideoSyncRequest(
  clock: RealtimeVideoSyncClock,
  backend: RealtimeVideoSyncBackendIdentity,
  paused: boolean,
): RealtimeVideoSyncRequest | null {
  if (backend.playback_generation !== clock.playback_generation) return null;
  return {
    playback_generation: clock.playback_generation,
    backend_epoch: backend.backend_epoch,
    clock_epoch: clock.clock_epoch,
    loop_index: clock.loop_index,
    position_ms: clock.position_ms,
    paused,
  };
}

export function nextRealtimeVideoSeekClockEpoch(
  ...epochs: Array<number | null | undefined>
): number | null {
  const current = epochs.reduce<number>((maximum, epoch) => (
    typeof epoch === 'number' && Number.isSafeInteger(epoch) && epoch >= 0
      ? Math.max(maximum, epoch)
      : maximum
  ), 0);
  return current < Number.MAX_SAFE_INTEGER ? current + 1 : null;
}

export function clampRealtimeVideoSeekPositionMs(
  requestedSeconds: number,
  sourceDurationMs: number,
): number {
  if (!Number.isSafeInteger(sourceDurationMs) || sourceDurationMs <= 0) return 0;
  const requestedMs = Number.isFinite(requestedSeconds)
    ? Math.round(requestedSeconds * 1_000)
    : 0;
  return Math.min(sourceDurationMs - 1, Math.max(0, requestedMs));
}

export function realtimeVideoSyncPositionToleranceMs(frameRateFps: number | null | undefined): number {
  const frameDurationMs = typeof frameRateFps === 'number'
    && Number.isFinite(frameRateFps)
    && frameRateFps > 0
      ? 1_000 / frameRateFps
      : 1_000 / 30;
  return Math.min(750, Math.max(350, Math.ceil(250 + frameDurationMs * 3)));
}

export function isRealtimeVideoSyncApplied(
  status: RealtimeVideoSyncObservedStatus,
  request: RealtimeVideoSyncRequest,
  positionToleranceMs = 500,
): boolean {
  if (status.playback_generation !== request.playback_generation
    || status.backend_epoch !== request.backend_epoch
    || status.clock_epoch !== request.clock_epoch
    || status.loop_index !== request.loop_index
    || (status.activation !== 'active' && status.activation !== 'available')
    || status.process_id === null
    || !Number.isSafeInteger(status.process_id)
    || status.process_id <= 0
    || status.eof !== null
    || status.presented_pts_ms === null
    || !Number.isSafeInteger(status.presented_pts_ms)
    || status.presented_pts_ms < 0
    || !Number.isFinite(positionToleranceMs)
    || positionToleranceMs < 0
    || Math.abs(status.presented_pts_ms - request.position_ms) > positionToleranceMs
  ) return false;
  return request.paused
    ? status.physical_paused === true
    : status.physical_paused === false;
}

function realtimeVideoSyncRequestKey(request: RealtimeVideoSyncRequest): string {
  return [
    request.playback_generation,
    request.backend_epoch,
    request.clock_epoch,
    request.loop_index,
    request.position_ms,
    request.paused ? 1 : 0,
  ].join(':');
}

export function createRealtimeVideoSyncCoordinator(
  options: RealtimeVideoSyncCoordinatorOptions,
): RealtimeVideoSyncCoordinator {
  const retryDelaysMs = options.retryDelaysMs ?? [100, 250, 500];
  let disposed = false;
  let revision = 0;
  let active: RealtimeVideoSyncJob | null = null;
  let pending: RealtimeVideoSyncJob | null = null;
  let settledKey: string | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;

  const clearRetryTimer = () => {
    if (retryTimer !== null) clearTimeout(retryTimer);
    retryTimer = null;
  };

  const queueJob = (
    request: RealtimeVideoSyncRequest,
    retryAttempt = 0,
    staleRefreshes = 0,
  ) => {
    pending = {
      revision: ++revision,
      key: realtimeVideoSyncRequestKey(request),
      request,
      retryAttempt,
      staleRefreshes,
    };
  };

  const scheduleRetry = (
    job: RealtimeVideoSyncJob,
    cause: unknown,
    disposition: 'busy' | 'result_unknown',
  ) => {
    const delay = retryDelaysMs[job.retryAttempt];
    if (delay === undefined) {
      settledKey = job.key;
      options.onFailure(cause, disposition, job.request);
      return;
    }
    retryTimer = setTimeout(() => {
      retryTimer = null;
      if (disposed || revision !== job.revision) return;
      pending = { ...job, retryAttempt: job.retryAttempt + 1 };
      void pump();
    }, delay);
  };

  const handleFailure = async (job: RealtimeVideoSyncJob, cause: unknown) => {
    const disposition = options.classifyError(cause);
    if (disposition === 'superseded') return;
    if (disposition === 'stale') {
      if (job.staleRefreshes >= 1) {
        settledKey = job.key;
        options.onFailure(cause, disposition, job.request);
        return;
      }
      try {
        const refreshed = await options.refreshLatestDesired();
        if (disposed || revision !== job.revision || refreshed === null) return;
        queueJob(refreshed, 0, job.staleRefreshes + 1);
      } catch (refreshCause) {
        if (!disposed && revision === job.revision) {
          options.onFailure(refreshCause, 'transport', job.request);
        }
      }
      return;
    }
    if (disposition === 'busy') {
      scheduleRetry(job, cause, disposition);
      return;
    }
    if (disposition === 'result_unknown') {
      try {
        const status = await options.readStatus();
        if (disposed || revision !== job.revision) return;
        if (options.isApplied(status, job.request)) {
          settledKey = job.key;
          options.onStatus(status);
          options.onSettled?.(job.request, status);
          return;
        }
      } catch (statusCause) {
        if (!disposed && revision === job.revision) {
          options.onFailure(statusCause, 'transport', job.request);
        }
        return;
      }
      scheduleRetry(job, cause, disposition);
      return;
    }
    if (disposition === 'permanent') settledKey = job.key;
    options.onFailure(cause, disposition, job.request);
  };

  async function pump() {
    if (disposed || active !== null || pending === null || retryTimer !== null) return;
    const job = pending;
    pending = null;
    active = job;
    try {
      const status = await options.invokeSync(job.request);
      if (disposed || revision !== job.revision) return;
      if (options.isApplied(status, job.request)) {
        settledKey = job.key;
        options.onStatus(status);
        options.onSettled?.(job.request, status);
      } else {
        scheduleRetry(job, {
          code: 'realtime_video_sync_result_unknown',
          message: 'mpv 物理位置尚未确认',
        }, 'result_unknown');
      }
    } catch (cause) {
      if (!disposed && revision === job.revision) await handleFailure(job, cause);
    } finally {
      if (active?.revision === job.revision) active = null;
      if (!disposed && pending !== null && retryTimer === null) void pump();
    }
  }

  return {
    submit(request) {
      if (disposed) return;
      const key = realtimeVideoSyncRequestKey(request);
      if (key === settledKey || key === active?.key || key === pending?.key) return;
      clearRetryTimer();
      queueJob(request);
      void pump();
    },
    dispose() {
      disposed = true;
      revision += 1;
      pending = null;
      clearRetryTimer();
    },
  };
}
