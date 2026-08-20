type RecoverableAudioOutputStatus = {
  available: boolean;
  preferred_portaudio: boolean;
  running: boolean;
  recovery_required: boolean;
  hardware_state: string;
};

type PortAudioSourceSnapshot = {
  audio_processing_enabled?: boolean;
  current_audio_source?: string | null;
  current_audio_reference?: string | null;
  current_video_reference?: string | null;
  source_media?: { source_path?: string | null } | null;
};

type AudioOutputSyncStatus = {
  preferred_portaudio: boolean;
  selected_backend: string;
  running: boolean;
  reason_code?: string | null;
  retryable?: boolean;
};

export type AudioOutputSyncDisposition = 'active' | 'retryable' | 'fallback';

export function isRetryableAudioOutputSyncCode(code: string | null | undefined): boolean {
  return code === 'audio_mixer_candidate_superseded'
    || code === 'audio_mixer_candidate_not_caught_up'
    || code === 'audio_mixer_recovery_in_progress'
    || code === 'audio_cycle_commit_in_progress';
}

export function isExpectedAudioOutputSyncCancellation(
  code: string | null | undefined,
  playbackState: string | null | undefined,
): boolean {
  return (playbackState === 'paused' || playbackState === 'stopped' || playbackState === 'ready')
    && (code === 'audio_mixer_candidate_stale' || code === 'audio_mixer_start_stale');
}

export function clearResolvedPortAudioSyncError(error: string | null): string | null {
  return error?.startsWith('PortAudio 音频源不可用，已回退 WebView')
    || error?.startsWith('PortAudio PCM 生产恢复失败，已回退 WebView')
    ? null
    : error;
}

export function classifyAudioOutputSync(
  status: AudioOutputSyncStatus,
): AudioOutputSyncDisposition {
  if (status.retryable || isRetryableAudioOutputSyncCode(status.reason_code)) {
    return 'retryable';
  }
  if (
    status.preferred_portaudio
    && status.selected_backend === 'portaudio'
    && status.running
  ) {
    return 'active';
  }
  return 'fallback';
}

export type PortAudioSourceRetryMode = 'recover' | 'sync' | null;

export function shouldKeepPortAudioCycleScheduling(
  status: RecoverableAudioOutputStatus | null | undefined,
): boolean {
  return Boolean(
    status?.available
    && status.preferred_portaudio
    && status.hardware_state === 'active',
  );
}

export function getPortAudioSourceRetryMode(
  status: RecoverableAudioOutputStatus,
  playbackState: string | null | undefined,
  alreadyAttempted: boolean,
): PortAudioSourceRetryMode {
  if (
    playbackState !== 'playing'
    || !status.available
    || !status.preferred_portaudio
    || status.running
    || status.hardware_state !== 'active'
    || alreadyAttempted
  ) {
    return null;
  }
  return status.recovery_required ? 'recover' : 'sync';
}

export function resolvePortAudioSourcePath(
  snapshot: PortAudioSourceSnapshot | null | undefined,
): string | null {
  if (
    snapshot?.current_audio_source === 'realtime_variant'
    && snapshot.current_audio_reference
  ) {
    return snapshot.current_audio_reference;
  }
  if (snapshot?.audio_processing_enabled) {
    return snapshot.source_media?.source_path ?? null;
  }
  return snapshot?.current_video_reference
    ?? snapshot?.source_media?.source_path
    ?? null;
}
