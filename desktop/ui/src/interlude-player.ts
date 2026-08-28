export type BaseAudioSource = 'realtime_variant' | 'processed_original' | 'original';

export const INTERLUDE_LIMITS = {
  intervalMinMs: { min: 500, max: 60_000 },
  volumeDb: { min: -60, max: 12 },
  duckingDepthDb: { min: -60, max: 0 },
  duckingAttackMs: { min: 0, max: 1_000 },
  duckingReleaseMs: { min: 0, max: 3_000 },
} as const;

export const INTERLUDE_PRESET_PERIOD_LIMITS = {
  min: 1_000,
  max: 600_000,
} as const;

export type InterludePresetPeriodPlan = {
  segment: number;
  segmentStartMs: number;
  periodMs: number;
  nextBoundaryMs: number;
};

export function interludeIntervalMsToSeconds(milliseconds: number) {
  return milliseconds / 1_000;
}

export function interludeIntervalSecondsToMs(seconds: number) {
  return Math.round(seconds * 1_000);
}

export function interludeVolumeDbToPercent(volumeDb: number) {
  if (!Number.isFinite(volumeDb) || volumeDb <= INTERLUDE_LIMITS.volumeDb.min) return 0;
  return Math.min(100, Math.max(0, Math.round(Math.pow(10, volumeDb / 20) * 100)));
}

export function interludeVolumePercentToDb(volumePercent: number) {
  const percent = Math.min(100, Math.max(0, Math.round(volumePercent)));
  return percent === 0 ? INTERLUDE_LIMITS.volumeDb.min : 20 * Math.log10(percent / 100);
}

export function resolveInterludeClockPlaybackRate(
  playbackSpeed: number,
  usesOriginalMediaClock: boolean,
) {
  if (!usesOriginalMediaClock || !Number.isFinite(playbackSpeed)) return 1;
  return Math.min(2, Math.max(0.5, playbackSpeed));
}

export function interludeFileNameFromPath(filePath: string): string | null {
  if (typeof filePath !== 'string' || filePath.length === 0) return null;
  const parts = filePath.split(/[\\/]/);
  const fileName = parts[parts.length - 1]?.trim() ?? '';
  return fileName.length > 0 && fileName.length <= 512 ? fileName : null;
}

export function createInterludePresetPeriodPlan(
  mediaPositionMs: number,
  minMs: number,
  maxMs: number,
  random: () => number = Math.random,
): InterludePresetPeriodPlan {
  const segmentStartMs = Math.max(0, Math.round(mediaPositionMs));
  const periodMs = randomIntervalMs(minMs, maxMs, random);
  return {
    segment: 1,
    segmentStartMs,
    periodMs,
    nextBoundaryMs: segmentStartMs + periodMs,
  };
}

export function advanceInterludePresetPeriodPlan(
  current: InterludePresetPeriodPlan,
  mediaPositionMs: number,
  minMs: number,
  maxMs: number,
  random: () => number = Math.random,
): InterludePresetPeriodPlan | null {
  const position = Math.max(0, Math.round(mediaPositionMs));
  if (position < current.nextBoundaryMs) return null;
  const periodMs = randomIntervalMs(minMs, maxMs, random);
  return {
    segment: current.segment + 1,
    segmentStartMs: current.nextBoundaryMs,
    periodMs,
    nextBoundaryMs: current.nextBoundaryMs + periodMs,
  };
}

export function interludePresetPeriodProgress(
  plan: InterludePresetPeriodPlan | null,
  mediaPositionMs: number,
): number {
  if (!plan || plan.periodMs <= 0) return 0;
  const elapsed = Math.max(0, Math.round(mediaPositionMs) - plan.segmentStartMs);
  return Math.min(100, Math.max(0, elapsed / plan.periodMs * 100));
}

type ResolveBaseAudioSourceInput = {
  realtimeVariantActive: boolean;
  processedOriginalActive: boolean;
};

type ShouldPauseInterludeInput = {
  playbackState: string;
  fixedSpeechActive: boolean;
};

type ResolvePlaybackAudioSourceInput = {
  effectiveAudioSource?: string | null;
  currentAudioSource?: string | null;
  currentVideoSource?: string | null;
};

function clampRandomUnit(randomValue: number) {
  if (!Number.isFinite(randomValue)) return 0;
  if (randomValue <= 0) return 0;
  if (randomValue >= 1) return 1;
  return randomValue;
}

export function resolveBaseAudioSource({
  realtimeVariantActive,
  processedOriginalActive,
}: ResolveBaseAudioSourceInput): BaseAudioSource {
  if (realtimeVariantActive) return 'realtime_variant';
  if (processedOriginalActive) return 'processed_original';
  return 'original';
}

function isBaseAudioSource(value: string): value is BaseAudioSource {
  return value === 'realtime_variant' ||
    value === 'processed_original' ||
    value === 'original';
}

export function resolvePlaybackAudioSource({
  effectiveAudioSource,
  currentAudioSource,
  currentVideoSource,
}: ResolvePlaybackAudioSourceInput): BaseAudioSource {
  if (effectiveAudioSource && isBaseAudioSource(effectiveAudioSource)) {
    return effectiveAudioSource;
  }
  return resolveBaseAudioSource({
    realtimeVariantActive: currentAudioSource === 'realtime_variant',
    processedOriginalActive: currentVideoSource === 'processed',
  });
}

export function randomIntervalMs(minMs: number, maxMs: number, random: () => number = Math.random) {
  const lower = Math.min(minMs, maxMs);
  const upper = Math.max(minMs, maxMs);
  const ratio = clampRandomUnit(random());
  return Math.round(lower + (upper - lower) * ratio);
}

export function nextInterludeAtMs(
  currentTimeMs: number,
  hasPlayedInterlude: boolean,
  minMs: number,
  maxMs: number,
  random: () => number = Math.random,
) {
  const current = Math.max(0, Math.round(currentTimeMs));
  return hasPlayedInterlude ? current + randomIntervalMs(minMs, maxMs, random) : current;
}

export function interludeIntervalProgress(startTimeMs: number, targetTimeMs: number, currentTimeMs: number) {
  if (![startTimeMs, targetTimeMs, currentTimeMs].every(Number.isFinite)) return 0;
  const durationMs = targetTimeMs - startTimeMs;
  if (durationMs <= 0) return currentTimeMs >= targetTimeMs ? 100 : 0;
  return Math.min(100, Math.max(0, ((currentTimeMs - startTimeMs) / durationMs) * 100));
}

export function buildInterludeScheduleKey(playbackGeneration: number, sourceKey: string) {
  return `${playbackGeneration}:${sourceKey}`;
}

export function chooseInterludeIndex(count: number, previousIndex: number | null, random: () => number = Math.random) {
  if (!Number.isFinite(count) || count <= 0) return null;
  if (count === 1) return 0;
  if (previousIndex === null || previousIndex < 0 || previousIndex >= count) {
    return Math.min(count - 1, Math.floor(clampRandomUnit(random()) * count));
  }

  const ratio = clampRandomUnit(random());
  const nextIndex = Math.min(count - 2, Math.floor(ratio * (count - 1)));
  return nextIndex >= previousIndex ? nextIndex + 1 : nextIndex;
}

export function shouldPauseInterlude({
  playbackState,
  fixedSpeechActive,
}: ShouldPauseInterludeInput) {
  return playbackState !== 'playing' || fixedSpeechActive;
}
