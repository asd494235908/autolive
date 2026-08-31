const PERMANENT_REALTIME_VIDEO_ERROR_CODES = new Set([
  'gpu83_parameters_unavailable',
  'realtime_video_sync_invalid',
  'realtime_video_sync_exhausted',
  'stale_original_video_renderer',
  'original_video_renderer_stale',
  'original_video_renderer_invalid',
  'original_video_playback_inactive',
  'original_video_renderer_not_allowed',
  'original_video_source_required',
  'original_video_position_invalid',
]);

const RECOVERABLE_REALTIME_VIDEO_ERROR_CODES = new Set([
  'stale_realtime_video_plan',
  'realtime_video_prepare_stale',
  'stale_realtime_video_backend_epoch',
]);

export type RealtimeVideoSyncErrorDisposition =
  | 'stale'
  | 'busy'
  | 'result_unknown'
  | 'transport'
  | 'permanent'
  | 'superseded';

function errorField(cause: unknown, field: 'code' | 'message'): string | null {
  if (!cause || typeof cause !== 'object' || Array.isArray(cause) || !(field in cause)) return null;
  const value = (cause as Record<string, unknown>)[field];
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

export function isPermanentRealtimeVideoError(cause: unknown): boolean {
  if (isRecoverableRealtimeVideoError(cause)) return false;
  const code = errorField(cause, 'code');
  if (code && PERMANENT_REALTIME_VIDEO_ERROR_CODES.has(code)) return true;
  const message = errorField(cause, 'message')
    ?? (cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : '');
  return message.includes('已过期')
    || message.includes('待提交计划')
    || message.includes('当前活动视频参数不能由 Vulkan GPU83 滤镜完整执行');
}

export function isRecoverableRealtimeVideoError(cause: unknown): boolean {
  const code = errorField(cause, 'code');
  if (code !== null && RECOVERABLE_REALTIME_VIDEO_ERROR_CODES.has(code)) return true;
  const message = errorField(cause, 'message')
    ?? (cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : '');
  return message.includes('实时画面计划绑定的播放代次已过期')
    || message.includes('实时画面提交绑定的播放代次已过期')
    || message.includes('实时画面准备请求的后端 epoch 已过期');
}

export function classifyRealtimeVideoSyncError(
  cause: unknown,
): RealtimeVideoSyncErrorDisposition {
  const code = errorField(cause, 'code');
  if (code === 'realtime_video_sync_superseded') return 'superseded';
  if (code === 'realtime_video_sync_busy') return 'busy';
  if (code === 'realtime_video_sync_result_unknown') return 'result_unknown';
  if (code === 'realtime_video_sync_transport_failed'
    || code === 'realtime_video_sync_failed') return 'transport';
  if (code === 'stale_realtime_video_sync'
    || code === 'realtime_video_sync_stale'
    || isRecoverableRealtimeVideoError(cause)) return 'stale';
  return isPermanentRealtimeVideoError(cause) ? 'permanent' : 'transport';
}
