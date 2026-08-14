export type PlaybackMediaStateMessage = {
  version: 1;
  type: 'playback-media-state';
  current_time: number;
  duration: number;
  volume: number;
  muted: boolean;
  paused: boolean;
};

export type PlaybackMediaControlMessage =
  | { version: 1; type: 'playback-media-control'; action: 'seek'; current_time: number }
  | { version: 1; type: 'playback-media-control'; action: 'set-volume'; volume: number }
  | { version: 1; type: 'playback-media-control'; action: 'toggle-muted' };

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function isVolume(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1;
}

export function isPlaybackMediaStateMessage(value: unknown): value is PlaybackMediaStateMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    record.version === 1 &&
    record.type === 'playback-media-state' &&
    isFiniteNonNegative(record.current_time) &&
    isFiniteNonNegative(record.duration) &&
    isVolume(record.volume) &&
    typeof record.muted === 'boolean' &&
    typeof record.paused === 'boolean'
  );
}

export function isPlaybackMediaControlMessage(value: unknown): value is PlaybackMediaControlMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || record.type !== 'playback-media-control') return false;
  if (record.action === 'seek') return isFiniteNonNegative(record.current_time);
  if (record.action === 'set-volume') return isVolume(record.volume);
  return record.action === 'toggle-muted';
}

export function clampMediaTime(value: number, duration: number): number {
  const safeValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  if (!Number.isFinite(duration) || duration < 0) return safeValue;
  return Math.min(safeValue, duration);
}

export function resolvePlaybackPositionMs(
  mediaCurrentTime: number | null | undefined,
  snapshotPositionMs: number | null | undefined,
): number | null {
  if (isFiniteNonNegative(mediaCurrentTime)) return Math.round(mediaCurrentTime * 1000);
  if (isFiniteNonNegative(snapshotPositionMs)) return Math.round(snapshotPositionMs);
  return null;
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
