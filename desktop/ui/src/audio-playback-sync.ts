const MIN_MEDIA_PLAYBACK_RATE = 0.25;
const MAX_MEDIA_PLAYBACK_RATE = 4;
const SOURCE_AUDIO_SOFT_SYNC_THRESHOLD_MS = 20;
const SOURCE_AUDIO_HARD_SYNC_THRESHOLD_MS = 120;
const SOURCE_AUDIO_SYNC_RATE_DELTA = 0.02;

export type SourceAudioSyncDecision = Readonly<{
  hardRealign: boolean;
  playbackRate: number;
}>;

/**
 * FFmpeg 的音高链保持音频时长，因此音高不能改动画面时钟。
 * 只有显式 playback_speed 才同时驱动 HTML 视频和 PortAudio 音频。
 */
export function resolveSynchronizedVideoPlaybackRate(
  audioProcessingEnabled: boolean,
  playbackSpeed: number | null | undefined,
): number {
  if (!audioProcessingEnabled || !Number.isFinite(playbackSpeed) || Number(playbackSpeed) <= 0) {
    return 1;
  }
  return Math.min(MAX_MEDIA_PLAYBACK_RATE, Math.max(MIN_MEDIA_PLAYBACK_RATE, Number(playbackSpeed)));
}

export function resolveSourceAudioSync(
  basePlaybackRate: number,
  driftMs: number,
  crossedLoop: boolean,
  ended: boolean,
): SourceAudioSyncDecision {
  const rate = Number.isFinite(basePlaybackRate) && basePlaybackRate > 0
    ? Math.min(MAX_MEDIA_PLAYBACK_RATE, Math.max(MIN_MEDIA_PLAYBACK_RATE, basePlaybackRate))
    : 1;
  if (
    crossedLoop
    || ended
    || !Number.isFinite(driftMs)
    || Math.abs(driftMs) >= SOURCE_AUDIO_HARD_SYNC_THRESHOLD_MS
  ) {
    return { hardRealign: true, playbackRate: rate };
  }
  if (Math.abs(driftMs) <= SOURCE_AUDIO_SOFT_SYNC_THRESHOLD_MS) {
    return { hardRealign: false, playbackRate: rate };
  }
  return {
    hardRealign: false,
    playbackRate: Math.min(
      MAX_MEDIA_PLAYBACK_RATE,
      Math.max(
        MIN_MEDIA_PLAYBACK_RATE,
        rate * (driftMs > 0 ? 1 - SOURCE_AUDIO_SYNC_RATE_DELTA : 1 + SOURCE_AUDIO_SYNC_RATE_DELTA),
      ),
    ),
  };
}
