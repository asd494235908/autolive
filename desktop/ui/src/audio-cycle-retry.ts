const CYCLE_RETRY_DELAYS_MS = [1_000, 2_000, 4_000, 8_000, 15_000] as const;

export type CycleRetryState = {
  identity: string | null;
  failureCount: number;
  retryAtMs: number | null;
  exhausted: boolean;
};

export function createCycleRetryState(): CycleRetryState {
  return { identity: null, failureCount: 0, retryAtMs: null, exhausted: false };
}

export function clearCycleRetry(): CycleRetryState {
  return createCycleRetryState();
}

export function recordCycleRetryFailure(
  current: CycleRetryState,
  identity: string,
  nowMs: number,
): CycleRetryState {
  const failureCount = current.identity === identity ? current.failureCount + 1 : 1;
  const delayMs = CYCLE_RETRY_DELAYS_MS[failureCount - 1];
  return delayMs === undefined
    ? { identity, failureCount, retryAtMs: null, exhausted: true }
    : { identity, failureCount, retryAtMs: nowMs + delayMs, exhausted: false };
}

export function isCycleRetryReady(
  current: CycleRetryState,
  identity: string,
  nowMs: number,
): boolean {
  if (current.identity !== identity) return true;
  if (current.exhausted) return false;
  return current.retryAtMs === null || nowMs >= current.retryAtMs;
}

export type AudioCycleRetryState = CycleRetryState;
export const createAudioCycleRetryState = createCycleRetryState;
export const clearAudioCycleRetry = clearCycleRetry;
export const recordAudioCycleRetryFailure = recordCycleRetryFailure;
export const isAudioCycleRetryReady = isCycleRetryReady;
