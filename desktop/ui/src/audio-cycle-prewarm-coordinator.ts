export const AUDIO_CYCLE_PREPARE_LEAD_MS = 4_000;
export const AUDIO_CYCLE_COMMIT_GRACE_MS = 500;

export type AudioCycleCandidateStatus = 'planned' | 'preparing' | 'prepared' | 'committing';

export type AudioCycleCandidatePlan<T> = {
  candidateId: number;
  sample: T;
  targetAtMs: number;
  prepareAtMs: number;
  status: AudioCycleCandidateStatus;
};

export type AudioCycleCoordinatorAction = 'prepare' | 'commit' | 'expire' | null;

export function createAudioCycleCandidatePlan<T>(
  candidateId: number,
  sample: T,
  committedAtMs: number,
  periodMs: number,
): AudioCycleCandidatePlan<T> {
  const safePeriodMs = Math.max(1, Math.round(periodMs));
  const targetAtMs = committedAtMs + safePeriodMs;
  return {
    candidateId,
    sample,
    targetAtMs,
    prepareAtMs: Math.max(committedAtMs, targetAtMs - AUDIO_CYCLE_PREPARE_LEAD_MS),
    status: 'planned',
  };
}

export function getAudioCycleCoordinatorAction<T>(
  plan: AudioCycleCandidatePlan<T> | null,
  nowMs: number,
): AudioCycleCoordinatorAction {
  if (!plan) return null;
  if (nowMs > plan.targetAtMs + AUDIO_CYCLE_COMMIT_GRACE_MS) return 'expire';
  if (plan.status === 'planned' && nowMs >= plan.prepareAtMs) return 'prepare';
  if (plan.status === 'prepared' && nowMs >= plan.targetAtMs) return 'commit';
  return null;
}

export function updateAudioCycleCandidateStatus<T>(
  plan: AudioCycleCandidatePlan<T>,
  status: AudioCycleCandidateStatus,
): AudioCycleCandidatePlan<T> {
  return { ...plan, status };
}

export function recoverAudioCycleCandidate<T>(
  plan: AudioCycleCandidatePlan<T>,
  operation: 'prepare' | 'commit',
  nowMs: number,
): AudioCycleCandidatePlan<T> | null {
  if (nowMs > plan.targetAtMs + AUDIO_CYCLE_COMMIT_GRACE_MS) return null;
  return {
    ...plan,
    status: operation === 'prepare' ? 'planned' : 'prepared',
  };
}

export type AudioCycleCommandMessage = {
  version: 1;
  type: 'audio-cycle-command';
  action: 'prepare' | 'commit' | 'cancel';
  candidate_id: number;
  playback_generation: number;
  base_audio_stream_revision?: number;
  target_at_ms?: number;
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
  snapshot?: unknown;
};

export function isAudioCycleCommandMessage(value: unknown): value is AudioCycleCommandMessage {
  if (!value || typeof value !== 'object') return false;
  const message = value as Partial<AudioCycleCommandMessage>;
  return message.version === 1
    && message.type === 'audio-cycle-command'
    && ['prepare', 'commit', 'cancel'].includes(message.action ?? '')
    && Number.isSafeInteger(message.candidate_id)
    && Number(message.candidate_id) > 0
    && Number.isSafeInteger(message.playback_generation);
}

export function isAudioCycleResultMessage(value: unknown): value is AudioCycleResultMessage {
  if (!value || typeof value !== 'object') return false;
  const message = value as Partial<AudioCycleResultMessage>;
  return message.version === 1
    && message.type === 'audio-cycle-result'
    && ['prepare', 'commit', 'cancel'].includes(message.action ?? '')
    && Number.isSafeInteger(message.candidate_id)
    && typeof message.accepted === 'boolean'
    && typeof message.committed === 'boolean';
}
