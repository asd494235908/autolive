import type { PlaybackClockStatus } from './playback-clock-health';

export type PlaybackMediaStateMessage = {
  version: 2;
  type: 'playback-media-state';
  current_time: number;
  duration: number;
  volume: number;
  muted: boolean;
  paused: boolean;
  playback_generation: number;
  source_revision: number;
  clock_session: string;
  clock_epoch: number;
  clock_sequence: number;
  loop_index: number;
  position_ms: number;
  duration_ms: number;
  absolute_position_ms: number;
  playback_rate: number;
  clock_health: PlaybackClockStatus;
};

export type PlaybackMediaControlMessage =
  | { version: 1; type: 'playback-media-control'; action: 'seek'; current_time: number; playback_generation: number }
  | { version: 1; type: 'playback-media-control'; action: 'set-volume'; volume: number }
  | { version: 1; type: 'playback-media-control'; action: 'toggle-muted' };

export type AudioSyncClock = {
  playback_generation: number;
  loop_index: number;
  position_ms: number;
  duration_ms: number;
  absolute_position_ms: number;
};

export type PlaybackCommand =
  | 'pause_playback'
  | 'resume_playback'
  | 'stop_playback'
  | 'start_playback';

export type PlaybackPositionCheckpoint = {
  playbackGeneration: number;
  positionSec: number;
};

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function isVolume(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1;
}

function isSafeNonNegativeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isPlaybackClockStatus(value: unknown): value is PlaybackClockStatus {
  return value === 'healthy'
    || value === 'buffering'
    || value === 'recovering'
    || value === 'stalled';
}

function hasConsistentAbsoluteClock(
  loopIndex: number,
  positionMs: number,
  durationMs: number,
  absolutePositionMs: number,
): boolean {
  if (positionMs > durationMs || (durationMs === 0 && loopIndex !== 0)) return false;
  const expected = loopIndex * durationMs + positionMs;
  return Number.isSafeInteger(expected) && expected === absolutePositionMs;
}

export function isPlaybackMediaStateMessage(value: unknown): value is PlaybackMediaStateMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    record.version === 2 &&
    record.type === 'playback-media-state' &&
    isFiniteNonNegative(record.current_time) &&
    isFiniteNonNegative(record.duration) &&
    isVolume(record.volume) &&
    typeof record.muted === 'boolean' &&
    typeof record.paused === 'boolean' &&
    isSafeNonNegativeInteger(record.playback_generation) &&
    isSafeNonNegativeInteger(record.source_revision) &&
    typeof record.clock_session === 'string' &&
    record.clock_session.length > 0 &&
    record.clock_session.length <= 128 &&
    isSafeNonNegativeInteger(record.clock_epoch) &&
    isSafeNonNegativeInteger(record.clock_sequence) &&
    isSafeNonNegativeInteger(record.loop_index) &&
    isSafeNonNegativeInteger(record.position_ms) &&
    isSafeNonNegativeInteger(record.duration_ms) &&
    isSafeNonNegativeInteger(record.absolute_position_ms) &&
    hasConsistentAbsoluteClock(
      record.loop_index,
      record.position_ms,
      record.duration_ms,
      record.absolute_position_ms,
    ) &&
    typeof record.playback_rate === 'number' &&
    Number.isFinite(record.playback_rate) &&
    record.playback_rate > 0 &&
    isPlaybackClockStatus(record.clock_health)
  );
}

export function shouldAcceptPlaybackMediaState(
  previous: PlaybackMediaStateMessage | null,
  next: PlaybackMediaStateMessage,
): boolean {
  if (
    !previous
    || previous.playback_generation !== next.playback_generation
    || previous.clock_session !== next.clock_session
    || previous.source_revision !== next.source_revision
  ) {
    return true;
  }
  return next.clock_epoch > previous.clock_epoch
    || (
      next.clock_epoch === previous.clock_epoch
      && next.clock_sequence > previous.clock_sequence
    );
}

export function isPlaybackMediaControlMessage(value: unknown): value is PlaybackMediaControlMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || record.type !== 'playback-media-control') return false;
  if (record.action === 'seek') {
    return isFiniteNonNegative(record.current_time)
      && isSafeNonNegativeInteger(record.playback_generation);
  }
  if (record.action === 'set-volume') return isVolume(record.volume);
  return record.action === 'toggle-muted';
}

export function shouldApplyPlaybackSeek(
  message: PlaybackMediaControlMessage,
  currentPlaybackGeneration: number | null | undefined,
): boolean {
  return message.action === 'seek'
    && isSafeNonNegativeInteger(currentPlaybackGeneration)
    && message.playback_generation === currentPlaybackGeneration;
}

export function capturePlaybackPosition(
  current: PlaybackPositionCheckpoint | null,
  input: {
    playbackGeneration: number;
    loadedPlaybackGeneration: number | null;
    positionSec: number;
    transitionInFlight: boolean;
  },
): PlaybackPositionCheckpoint | null {
  if (
    input.transitionInFlight
    || input.loadedPlaybackGeneration !== input.playbackGeneration
    || !Number.isFinite(input.positionSec)
    || input.positionSec < 0
  ) {
    return current;
  }
  return {
    playbackGeneration: input.playbackGeneration,
    positionSec: input.positionSec,
  };
}

export function resolvePlaybackResumePosition(
  checkpoint: PlaybackPositionCheckpoint | null,
  playbackGeneration: number | null | undefined,
): number {
  return checkpoint && checkpoint.playbackGeneration === playbackGeneration
    ? checkpoint.positionSec
    : 0;
}

export function clampMediaTime(value: number, duration: number): number {
  const safeValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  if (!Number.isFinite(duration) || duration < 0) return safeValue;
  return Math.min(safeValue, duration);
}

export function clampVolume(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

export function formatMediaTime(seconds: number): string {
  const safeSeconds = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  const minutes = Math.floor(safeSeconds / 60);
  const remainder = safeSeconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`;
}

export function shouldIssuePlaybackCommand(
  command: PlaybackCommand,
  playbackState: string | null | undefined,
): boolean {
  const state = playbackState?.toLowerCase();
  if ((command === 'start_playback' || command === 'resume_playback') && state === 'playing') return false;
  if (command === 'pause_playback' && state === 'paused') return false;
  return command !== 'stop_playback' || state !== 'stopped';
}

export function createAudioSyncClock(input: {
  playbackGeneration: number;
  loopIndex: number;
  positionMs: number;
  durationMs: number;
}): AudioSyncClock {
  const { playbackGeneration, loopIndex, positionMs, durationMs } = input;
  if (
    !isSafeNonNegativeInteger(playbackGeneration)
    || !isSafeNonNegativeInteger(loopIndex)
    || !isSafeNonNegativeInteger(positionMs)
    || !isSafeNonNegativeInteger(durationMs)
    || durationMs <= 0
    || positionMs > durationMs
  ) {
    throw new RangeError('播放绝对媒体时钟参数无效');
  }
  const absolutePositionMs = loopIndex * durationMs + positionMs;
  if (!Number.isSafeInteger(absolutePositionMs)) {
    throw new RangeError('播放绝对媒体时钟超出安全整数范围');
  }
  return {
    playback_generation: playbackGeneration,
    loop_index: loopIndex,
    position_ms: positionMs,
    duration_ms: durationMs,
    absolute_position_ms: absolutePositionMs,
  };
}
