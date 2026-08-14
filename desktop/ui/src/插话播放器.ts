export type BaseAudioSource = 'voice_clone' | 'realtime_variant' | 'processed_original' | 'original';

export const INTERLUDE_LIMITS = {
  intervalMinMs: { min: 500, max: 60_000 },
  volumeDb: { min: -60, max: 12 },
  duckingDepthDb: { min: -60, max: 0 },
  duckingAttackMs: { min: 5, max: 1_000 },
  duckingReleaseMs: { min: 10, max: 3_000 },
} as const;

type ResolveBaseAudioSourceInput = {
  voiceCloneActive: boolean;
  realtimeVariantActive: boolean;
  processedOriginalActive: boolean;
};

type ShouldPauseInterludeInput = {
  playbackState: string;
  voiceCloneStatus: string;
};

type ResolvePlaybackAudioSourceInput = {
  effectiveAudioSource?: string | null;
  voiceCloneStatus?: string | null;
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
  voiceCloneActive,
  realtimeVariantActive,
  processedOriginalActive,
}: ResolveBaseAudioSourceInput): BaseAudioSource {
  if (voiceCloneActive) return 'voice_clone';
  if (realtimeVariantActive) return 'realtime_variant';
  if (processedOriginalActive) return 'processed_original';
  return 'original';
}

function isBaseAudioSource(value: string): value is BaseAudioSource {
  return value === 'voice_clone' ||
    value === 'realtime_variant' ||
    value === 'processed_original' ||
    value === 'original';
}

export function resolvePlaybackAudioSource({
  effectiveAudioSource,
  voiceCloneStatus,
  currentAudioSource,
  currentVideoSource,
}: ResolvePlaybackAudioSourceInput): BaseAudioSource {
  if (effectiveAudioSource && isBaseAudioSource(effectiveAudioSource)) {
    return effectiveAudioSource;
  }
  return resolveBaseAudioSource({
    voiceCloneActive: voiceCloneStatus === 'playing',
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

export function shouldPauseInterlude({ playbackState, voiceCloneStatus }: ShouldPauseInterludeInput) {
  if (playbackState !== 'playing') return true;
  return voiceCloneStatus === 'generating' || voiceCloneStatus === 'playing';
}
