const MIN_MEDIA_PLAYBACK_RATE = 0.25;
const MAX_MEDIA_PLAYBACK_RATE = 4;

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
