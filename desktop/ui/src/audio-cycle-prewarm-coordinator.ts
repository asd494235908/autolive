export const AUDIO_CYCLE_COMMIT_GRACE_MS = 500;

export type AudioCycleCandidateStatus = 'planned' | 'preparing' | 'prepared' | 'committing';

export type AudioCycleCandidatePlan<T> = {
  candidateId: number;
  sample: T;
  targetAbsolutePositionMs: number;
  status: AudioCycleCandidateStatus;
};

export type AudioCycleCoordinatorAction = 'prepare' | 'commit' | 'expire' | null;

export function createAudioCycleCandidatePlan<T>(
  candidateId: number,
  sample: T,
  baseAbsolutePositionMs: number,
  periodMediaMs: number,
): AudioCycleCandidatePlan<T> {
  const safeBaseMs = Math.max(0, Math.round(baseAbsolutePositionMs));
  const safePeriodMs = Math.max(1, Math.round(periodMediaMs));
  return {
    candidateId,
    sample,
    targetAbsolutePositionMs: safeBaseMs + safePeriodMs,
    status: 'planned',
  };
}

export function getAudioCycleCoordinatorAction<T>(
  plan: AudioCycleCandidatePlan<T> | null,
  currentAbsolutePositionMs: number,
  playbackRate: number,
  queuedPlaybackMs = 0,
): AudioCycleCoordinatorAction {
  if (!plan) return null;
  const safePlaybackRate = Number.isFinite(playbackRate) && playbackRate > 0 ? playbackRate : 1;
  const safeQueuedPlaybackMs = Number.isFinite(queuedPlaybackMs) && queuedPlaybackMs > 0
    ? queuedPlaybackMs
    : 0;
  const graceMediaMs = Math.round(AUDIO_CYCLE_COMMIT_GRACE_MS * safePlaybackRate);
  if (plan.status === 'committing') return null;
  if (currentAbsolutePositionMs > plan.targetAbsolutePositionMs + graceMediaMs) return 'expire';
  const mediaLeadMs = plan.targetAbsolutePositionMs - currentAbsolutePositionMs;
  if (plan.status === 'planned') return 'prepare';
  const commitLeadMediaMs = Math.round(safeQueuedPlaybackMs * safePlaybackRate);
  if (plan.status === 'prepared' && mediaLeadMs <= commitLeadMediaMs) return 'commit';
  return null;
}

export function updateAudioCycleCandidateStatus<T>(
  plan: AudioCycleCandidatePlan<T>,
  status: AudioCycleCandidateStatus,
): AudioCycleCandidatePlan<T> {
  return { ...plan, status };
}

export type AudioCycleCommandMessage = {
  version: 1;
  type: 'audio-cycle-command';
  action: 'prepare' | 'commit' | 'cancel';
  candidate_id: number;
  playback_generation: number;
  base_audio_stream_revision?: number;
  target_absolute_position_ms?: number;
  audio?: Record<string, unknown>;
  audio_variants?: Record<string, unknown>[];
};

export type AudioCycleResultMessage = {
  version: 1;
  type: 'audio-cycle-result';
  action: 'prepare' | 'commit' | 'cancel';
  candidate_id: number;
  accepted: boolean;
  committed: boolean;
  reason: string | null;
  error_code?: string | null;
  snapshot?: unknown;
};

export function isAudioCycleCommandMessage(value: unknown): value is AudioCycleCommandMessage {
  if (!value || typeof value !== 'object') return false;
  const message = value as Partial<AudioCycleCommandMessage>;
  const validEnvelope = message.version === 1
    && message.type === 'audio-cycle-command'
    && ['prepare', 'commit', 'cancel'].includes(message.action ?? '')
    && Number.isSafeInteger(message.candidate_id)
    && Number(message.candidate_id) > 0
    && Number.isSafeInteger(message.playback_generation);
  if (!validEnvelope || message.action !== 'prepare') return validEnvelope;
  return Number.isSafeInteger(message.base_audio_stream_revision)
    && Number.isSafeInteger(message.target_absolute_position_ms)
    && Number(message.target_absolute_position_ms) >= 0
    && Boolean(message.audio)
    && typeof message.audio === 'object'
    && Array.isArray(message.audio_variants);
}

export function isAudioCycleResultMessage(value: unknown): value is AudioCycleResultMessage {
  if (!value || typeof value !== 'object') return false;
  const message = value as Partial<AudioCycleResultMessage>;
  return message.version === 1
    && message.type === 'audio-cycle-result'
    && ['prepare', 'commit', 'cancel'].includes(message.action ?? '')
    && Number.isSafeInteger(message.candidate_id)
    && typeof message.accepted === 'boolean'
    && typeof message.committed === 'boolean'
    && (message.reason === null || typeof message.reason === 'string')
    && (message.error_code == null || typeof message.error_code === 'string');
}

export function isExpectedAudioCycleSkipCode(code: string | null | undefined): boolean {
  return code === 'audio_mixer_candidate_silent' || code === 'audio_mixer_candidate_stale';
}
