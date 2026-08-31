type 播放重启判断输入 = {
  restartToken?: string | number | null;
  lastRestartToken?: string | number | null;
  mediaGeneration?: number | null;
  lastRestartGeneration?: number | null;
  syncInFlight?: boolean;
  ended: boolean;
  endedRequiresDurationBoundary?: boolean;
  currentTime?: number;
  duration?: number;
};

const 接近结束阈值秒 = 0.05;

type 权威播放时钟输入 = {
  loopIndex: number;
  positionMs: number;
  durationMs: number;
};

export function buildAuthoritativePlaybackClock({
  loopIndex,
  positionMs,
  durationMs,
}: 权威播放时钟输入): {
  loopIndex: number;
  positionMs: number;
  absolutePositionMs: number;
} | null {
  if (
    !Number.isSafeInteger(loopIndex)
    || loopIndex < 0
    || !Number.isSafeInteger(positionMs)
    || positionMs < 0
    || !Number.isSafeInteger(durationMs)
    || durationMs <= 0
  ) return null;
  const boundedPositionMs = Math.min(positionMs, durationMs);
  const absolutePositionMs = loopIndex * durationMs + boundedPositionMs;
  if (!Number.isSafeInteger(absolutePositionMs)) return null;
  return { loopIndex, positionMs: boundedPositionMs, absolutePositionMs };
}

export function didSourceMediaLoopWrap(
  previousPositionMs: number | null,
  currentPositionMs: number,
  durationMs: number,
): boolean {
  if (
    previousPositionMs === null
    || !Number.isFinite(previousPositionMs)
    || !Number.isFinite(currentPositionMs)
    || !Number.isFinite(durationMs)
    || durationMs <= 0
  ) return false;
  const boundaryWindowMs = Math.min(1_000, Math.max(100, durationMs * 0.05));
  return previousPositionMs >= durationMs - boundaryWindowMs
    && currentPositionMs <= boundaryWindowMs;
}

export function isMediaCycleTargetInAuthoritativeLoop({
  targetAbsolutePositionMs,
  durationMs,
  mediaLoopIndex,
  authoritativeLoopIndex,
}: {
  targetAbsolutePositionMs: number;
  durationMs: number;
  mediaLoopIndex: number;
  authoritativeLoopIndex: number;
}): boolean {
  const targetLoopIndex = resolveAbsolutePlaybackLoopIndex(targetAbsolutePositionMs, durationMs);
  if (!Number.isSafeInteger(mediaLoopIndex) || !Number.isSafeInteger(authoritativeLoopIndex)) return false;
  return mediaLoopIndex === authoritativeLoopIndex
    && targetLoopIndex === authoritativeLoopIndex;
}

export function resolveAbsolutePlaybackLoopIndex(
  absolutePositionMs: number,
  durationMs: number,
): number | null {
  if (
    !Number.isSafeInteger(absolutePositionMs)
    || absolutePositionMs < 0
    || !Number.isSafeInteger(durationMs)
    || durationMs <= 0
  ) return null;
  return Math.floor(absolutePositionMs / durationMs);
}

export function resolveMseSourceTimeSeconds(
  streamTimeSeconds: number,
  sourceStreamOriginMs: number,
): number {
  if (!Number.isFinite(streamTimeSeconds) || streamTimeSeconds < 0) return 0;
  if (!Number.isSafeInteger(sourceStreamOriginMs) || sourceStreamOriginMs < 0) return streamTimeSeconds;
  return Math.max(0, streamTimeSeconds - sourceStreamOriginMs / 1_000);
}

export function resolvePlaybackBoundaryDuration({
  mediaDurationSeconds,
  sourceDurationMs,
  realtimeVideoStreamActive,
}: {
  mediaDurationSeconds: number;
  sourceDurationMs?: number | null;
  realtimeVideoStreamActive: boolean;
}): number {
  if (
    realtimeVideoStreamActive
    && Number.isSafeInteger(sourceDurationMs)
    && (sourceDurationMs as number) > 0
  ) {
    return (sourceDurationMs as number) / 1_000;
  }
  return Number.isFinite(mediaDurationSeconds) && mediaDurationSeconds > 0
    ? mediaDurationSeconds
    : 0;
}

export function shouldIgnoreLoopBoundaryPause({
  suppressMediaEvent,
  ended,
  currentTime,
  duration,
}: {
  suppressMediaEvent: boolean;
  ended: boolean;
  currentTime?: number;
  duration?: number;
}): boolean {
  return suppressMediaEvent || 已到播放边界({ ended, currentTime, duration });
}

export function shouldRestartCurrentSourceImmediately(sourceCount: number): boolean {
  return Number.isSafeInteger(sourceCount) && sourceCount === 1;
}

function 读取当前重启令牌({
  restartToken,
  mediaGeneration,
}: Pick<播放重启判断输入, 'restartToken' | 'mediaGeneration'>): string | number | null {
  if (restartToken !== undefined) {
    return restartToken;
  }
  return mediaGeneration ?? null;
}

function 读取上次重启令牌({
  lastRestartToken,
  lastRestartGeneration,
}: Pick<播放重启判断输入, 'lastRestartToken' | 'lastRestartGeneration'>): string | number | null {
  if (lastRestartToken !== undefined) {
    return lastRestartToken;
  }
  return lastRestartGeneration ?? null;
}

function 令牌有效(令牌: string | number | null): 令牌 is string | number {
  return typeof 令牌 === 'string' ? 令牌.length > 0 : Number.isFinite(令牌);
}

function 已到播放边界({
  ended,
  endedRequiresDurationBoundary,
  currentTime,
  duration,
}: Pick<播放重启判断输入, 'ended' | 'endedRequiresDurationBoundary' | 'currentTime' | 'duration'>): boolean {
  const reachedDurationBoundary = (
    Number.isFinite(currentTime) &&
    Number.isFinite(duration) &&
    (duration as number) > 0 &&
    (currentTime as number) >= (duration as number) - 接近结束阈值秒
  );
  return reachedDurationBoundary || (ended && !endedRequiresDurationBoundary);
}

export function shouldRestartPlayback({
  restartToken,
  lastRestartToken,
  mediaGeneration,
  lastRestartGeneration,
  syncInFlight,
  ended,
  endedRequiresDurationBoundary,
  currentTime,
  duration,
}: 播放重启判断输入): boolean {
  if (syncInFlight) {
    return false;
  }
  const currentRestartToken = 读取当前重启令牌({ restartToken, mediaGeneration });
  const previousRestartToken = 读取上次重启令牌({
    lastRestartToken,
    lastRestartGeneration,
  });

  if (!令牌有效(currentRestartToken)) {
    return false;
  }

  if (
    previousRestartToken !== null &&
    previousRestartToken !== undefined &&
    (!令牌有效(previousRestartToken) || currentRestartToken === previousRestartToken)
  ) {
    return false;
  }

  return 已到播放边界({
    ended,
    endedRequiresDurationBoundary,
    currentTime,
    duration,
  });
}
