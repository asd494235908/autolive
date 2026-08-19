import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open } from '@tauri-apps/plugin-dialog';
import { App as AntApp, Alert, Button, Card, Checkbox, ConfigProvider, Descriptions, Drawer, Input, InputNumber, Layout, Popconfirm, Progress, Select, Slider, Space, Switch, Tag, Typography } from 'antd';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { buildInterludeScheduleKey, chooseInterludeIndex, INTERLUDE_LIMITS, nextInterludeAtMs, resolvePlaybackAudioSource, shouldPauseInterlude } from './interlude-player';
import type { BaseAudioSource } from './interlude-player';
import {
  AUDIO_MIX_PICK_HARD_MAX,
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_MIX_PICK_MAX,
  DEFAULT_AUDIO_MIX_PICK_MIN,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  AUDIO_PARAM_CROSSFADE_SEC,
  buildAudioFxSignature,
  buildAudioVariantsFromCycle,
  buildRuntimePreviewParameters,
  getUnsupportedAudioPresetFields,
  UNMAPPED_AUDIO_PRESET_FIELD_LABELS,
  isRuntimeVariationDue,
  loadAudioMixSession,
  DEFAULT_AUDIO_PERIOD_MAX_MS,
  DEFAULT_AUDIO_PERIOD_MIN_MS,
  DEFAULT_VIDEO_PERIOD_MAX_MS,
  DEFAULT_VIDEO_PERIOD_MIN_MS,
  PERIOD_HARD_MAX_MS,
  PERIOD_HARD_MIN_MS,
  normalizeAudioMixPickMax,
  normalizeAudioMixPickMin,
  normalizeAudioPeriodRange,
  normalizeAudioVariationPeriod,
  normalizeVideoPeriodRange,
  loadAudioPeriodRange,
  loadVideoPeriodRange,
  sampleAudioCycle,
  samplePeriodMsInRange,
  sampleSubtleVideoParams,
  sanitizeMappedVideoSample,
  saveAudioMixSession,
  saveAudioPeriodRange,
  saveVideoPeriodRange,
} from './runtime-parameter-scheduler';
import type { AudioCycleSample, PeriodRangeMs, RuntimeBaseParameters, RuntimePreviewParameters, SubtleAudioSample } from './runtime-parameter-scheduler';
import { appendAudioCycleSnapshot } from './audioCycleSnapshot';
import { buildAudioCapabilityRows } from './audio-processing-capabilities';
import { resolveSynchronizedVideoPlaybackRate } from './audio-playback-sync';
import { shouldRestartPlayback } from './playback-loop';
import { buildFinalEffectWindowResizeKey } from './final-effect-window-size';
import { clampMediaTime, clampVolume, formatMediaTime, isPlaybackMediaControlMessage, isPlaybackMediaStateMessage } from './playback-control-message';
import type { PlaybackMediaControlMessage, PlaybackMediaStateMessage } from './playback-control-message';
import { scheduleAfterInitialPaint } from './startup-scheduler';
import { getDisplayErrorMessage } from './errorDisplay';
import {
  isCurrentRuntimeResourceAction,
  isRuntimeResourceBusy,
  resolvePendingRuntimeAction,
  runtimeResourceEnsureDecision,
  runtimeResourcePollComponent,
  shouldPollRuntimeResources,
} from './runtimeResources';
import type { RuntimeResourceComponent, RuntimeResourceStatus } from './runtimeResources';
import {
  getFixedSpeechTextError,
  isFixedSpeechCommandMessage,
  isFixedSpeechStatusMessage,
  selectLocalSpeechVoice,
} from './fixedSpeech';
import type {
  FixedSpeechCommandMessage,
  FixedSpeechStatus,
  FixedSpeechStatusMessage,
} from './fixedSpeech';
import {
  addFixedSpeechPreset,
  loadFixedSpeechPresets,
  removeFixedSpeechPreset,
  updateFixedSpeechPreset,
} from './fixedSpeechPresets';
import type { FixedSpeechPreset } from './fixedSpeechPresets';
import { getCspNonce } from './cspNonce';
import './desktop-layout.css';

const PLAYBACK_CHANNEL_NAME = 'autolive-playback-ui-v1';
const FIXED_SPEECH_ACK_TIMEOUT_MS = 3_000;
const RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500;
const PLAYBACK_SNAPSHOT_POLL_MS = 1_000;
const AUDIO_OUTPUT_STATUS_POLL_MS = 2_000;
const DIAGNOSTIC_PUBLISH_INTERVAL_MS = 50;
const DIAGNOSTIC_SAMPLE_COUNT = 128;
const REALTIME_AUDIO_SAFETY_LEAD_MS = 6_000;
const REALTIME_AUDIO_WORKER_TIMEOUT_MS = 5_000;
const PORTAUDIO_FORMAL_SOURCE_SYNC_READY = true;
const PORTAUDIO_SAMPLE_RATE_HZ = 44_100;
const PORTAUDIO_MIN_MEMORY_BUFFER_KIB = 128;
const PORTAUDIO_MAX_MEMORY_BUFFER_KIB = 2_048;
const PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB = 1_024;
const PORTAUDIO_DEFAULT_FRAMES_PER_BUFFER = 256;
const SUPPORTED_VIDEO_EXTENSIONS = [
  'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'ts', 'm2ts', 'flv', 'wmv', '3gp',
] as const;

type DiagnosticMessage = {
  version: 1;
  type: 'diagnostic';
  waveform: number[];
  spectrum: number[];
  sent_at_ms: number;
  error: string | null;
};

type RuntimeParameterMessage = {
  version: 1;
  type: 'runtime-parameters';
  payload: RuntimePreviewParameters | null;
  audio_processing_enabled: boolean;
  video_processing_enabled: boolean;
  playback_generation: number | null;
};

type PlaybackControlMessage = {
  version: 1;
  type: 'playback-control';
  action: 'pause' | 'resume' | 'stop';
};

type AudioOutputBackendMessage = {
  version: 1;
  type: 'audio-output-backend';
  preferred_portaudio: boolean;
  running: boolean;
  selected_backend: string;
  channels?: number;
  sample_rate_hz?: number;
};

type AudioOutputBackendClosedMessage = {
  version: 1;
  type: 'audio-output-backend-closed';
};

type PendingRuntimeAction = {
  component: RuntimeResourceComponent;
  resume: () => Promise<void>;
  token: number;
};

class RuntimeResourceConflictError extends Error {}

type PictureInPictureDocument = Document & {
  pictureInPictureEnabled?: boolean;
  pictureInPictureElement?: Element | null;
  exitPictureInPicture?: () => Promise<void>;
};

type PictureInPictureVideo = HTMLVideoElement & {
  requestPictureInPicture?: () => Promise<unknown>;
};

type MediaProbeResult = {
  canonical_path: string;
  source: {
    source_path: string;
    file_name: string;
    file_size_bytes: number;
    duration_ms: number | null;
    width: number | null;
    height: number | null;
    frame_rate_fps: number | null;
    audio_sample_rate_hz: number | null;
    audio_channel_count: number | null;
    mp4_sha256: string | null;
    mp4_hash_status: 'disabled' | 'pending' | 'ready' | 'failed';
  };
};

type PlaybackSnapshot = {
  playback_generation: number;
  playback_state: string;
  loop_index: number;
  current_position_ms: number;
  source_media: MediaProbeResult['source'] | null;
  current_video_source: string | null;
  current_video_reference: string | null;
  current_video_sha256: string | null;
  pending_video_reference: string | null;
  pending_video_sha256: string | null;
  video_processing_enabled: boolean;
  video_processing_status: string;
  audio_processing_enabled: boolean;
  realtime_audio_variant_enabled: boolean;
  current_audio_source: string | null;
  current_audio_reference: string | null;
  current_audio_start_at_ms: number;
  effective_audio_source?: BaseAudioSource | null;
  current_mp4_sha256: string | null;
  current_audio_sha256: string | null;
  audio_decision: string;
  worker_status: string;
  fallback_reason: string | null;
  pending_audio_candidate: boolean;
  pending_audio_reference: string | null;
  pending_audio_start_at_ms: number | null;
  pending_audio_duration_ms: number | null;
  audio_processing_parameters_version: string;
  audio_stream_variant_count?: number | null;
  audio_stream_revision: number;
  audio_processing_status: string;
  audio_processing_runtime: boolean;
  audio_processing_gain_db: number;
  interlude?: InterludeSnapshot | null;
};

type InterludeSnapshot = {
  enabled: boolean;
  directory: string | null;
  audio_files: string[];
  audio_count: number;
  status: string;
  error: string | null;
  interval_min_ms: number;
  interval_max_ms: number;
  volume_db: number;
  ducking_depth_db: number;
  ducking_attack_ms: number;
  ducking_release_ms: number;
};

type InterludeConfigDraft = {
  enabled: boolean;
  directory: string | null;
  intervalMinMs: number;
  intervalMaxMs: number;
  volumeDb: number;
  duckingDepthDb: number;
  duckingAttackMs: number;
  duckingReleaseMs: number;
};

type SpeechToSpeechWorkerCapabilities = {
  available: boolean;
  status: string;
  provider: string | null;
  model: string | null;
  reason: string | null;
};


type SpeechToSpeechStartResult = {
  accepted: boolean;
  snapshot: PlaybackSnapshot;
};

type FixedSpeechViewState = {
  operationId: string | null;
  status: 'idle' | FixedSpeechStatus;
  error: string | null;
};

type AudioOutputBackendStatus = {
  available: boolean;
  selected_backend: string;
  preferred_portaudio: boolean;
  running: boolean;
  reason: string | null;
  xrun_count: number;
  hardware_state: string;
  callback_status_flags: number;
  callback_status_flags_count: number;
  callback_underrun_count: number;
  producer_drop_count: number;
  output_latency_ms: number;
  actual_sample_rate_hz: number | null;
  callback_stalled_ms: number | null;
  ring_len_samples: number;
  ring_capacity_samples: number;
  device_index: number | null;
  memory_buffer_kib: number;
  frames_per_buffer: number;
  sample_rate_hz: number;
  channels: number;
  audio_task_count: number;
  current_audio_ffmpeg_pid: number | null;
  pending_audio_ffmpeg_pid: number | null;
};

type AudioOutputDevice = {
  id: string;
  name: string;
  host_api: string;
  max_output_channels: number;
  default_sample_rate_hz: number;
};

function getActualAudioOutputLabel(status: AudioOutputBackendStatus | null): 'PortAudio' | 'WebView' {
  return status?.running === true && status.selected_backend?.trim().toLowerCase() === 'portaudio'
    ? 'PortAudio'
    : 'WebView';
}

function getAudioRingBufferedLabel(status: AudioOutputBackendStatus | null): string {
  const sampleRateHz = status?.actual_sample_rate_hz ?? status?.sample_rate_hz ?? 0;
  if (!status || sampleRateHz <= 0 || status.channels <= 0) return '';
  const milliseconds = Math.round((status.ring_len_samples * 1_000) / (sampleRateHz * status.channels));
  return `（约 ${milliseconds}ms）`;
}

function getActualAudioStreamVariantCount(snapshot: PlaybackSnapshot | null): number | null {
  const count = snapshot?.audio_stream_variant_count;
  return typeof count === 'number' && Number.isSafeInteger(count) && count >= 0 ? count : null;
}

type MediaEngineCapabilities = {
  available: boolean;
  ffmpeg_version: string | null;
  ffprobe_version: string | null;
  reason: string | null;
};

type ResearchWorkerCapabilities = {
  available: boolean;
  executable: string | null;
  reason: string | null;
};

type ResearchStatus = {
  state: 'idle' | 'unavailable' | 'running' | 'ready' | 'failed' | 'cancelled';
  analysis_id: string | null;
  source_mp4_sha256: string | null;
  input_mp4_sha256: string | null;
  current_mp4_sha256: string | null;
  report_path: string | null;
  report_sha256: string | null;
  report_version: string | null;
  algorithm_version: string | null;
  random_seed: number | null;
  content_similarity_percent: number | null;
  media_robustness_score: number | null;
  invisible_mark_status: string | null;
  random_perturbation_applied: boolean | null;
  content_fingerprint: string | null;
  error: string | null;
};

type CacheCleanupResult = {
  removed_files: number;
  removed_bytes: number;
  remaining_bytes: number;
};

type ResearchParams = {
  audio: {
    natural_voice_mode: 'original' | 'natural_dynamic';
    random_change_period_ms: number;
    pitch_shift_semitones: number;
    spectral_perturbation_percent: number;
    environment_noise_percent: number;
    environment_noise_dbfs: number;
    phase_perturbation_percent: number;
    mfcc_shift_percent: number;
    loudness_adjustment_db: number;
    input_gain_db: number;
    output_gain_db: number;
    playback_speed: number;
    low_eq_db: number;
    mid_eq_db: number;
    high_eq_db: number;
    noise_reduction_percent: number;
    ambient_sound_mix_percent: number;
    fade_in_ms: number;
    fade_out_ms: number;
    dry_wet_percent: number;
    reverb_wet_percent: number;
    mfcc_dimensions: number;
    snr_variation_db: number;
    formant_shift_percent: number;
    vibrato_frequency_hz: number;
    vibrato_depth_percent: number;
    spectrum_blind_spot_percent: number;
    snr_target_db: number | null;
    current_formant_hz: number | null;
    filter_q: number;
    sample_rate_hz: number | null;
    output_bitrate_kbps: number;
    voice_library_id: string | null;
  };
  video: {
    brightness_percent: number;
    contrast_percent: number;
    saturation_percent: number;
    blur_radius_px: number;
    hue_rotation_degrees: number;
    sharpen_percent: number;
    noise_percent: number;
    detail_enhancement_percent: number;
    crop_edge_smoothing: number;
    frame_rate_jitter_percent: number;
    frame_rate_perturbation_frequency_hz: number;
    frame_rate_perturbation_amplitude_fps: number;
    pixel_scale_percent: number;
    pixel_jitter_px: number;
    dynamic_crop_percent: number;
    frame_inner_perturbation_percent: number;
    frame_inter_perturbation_percent: number;
    space_x_offset_px: number;
    space_y_offset_px: number;
    color_space_conversion_strength_percent: number;
  };
  research: {
    band_weights: Record<string, number>;
    target_frequency_hz: number | null;
    core_frequency_hz: number | null;
    wave_intensity: number;
    wave_level: number;
    wave_grain_count: number;
    dynamic_eq_threshold: number;
    channel_offset_percent: number;
    space_dimension: number;
    frequency_space_x_offset_px: number;
    frequency_space_y_offset_px: number;
    frame_perturbation_probability_percent: number;
    random_graphic_opacity_percent: number;
    random_graphic_size_px: number;
    abstract_face_count: number;
    abstract_face_size_percent: number;
    abstract_face_opacity_percent: number;
    overlay_offset_px: number;
    slice_length_ms: number;
    slice_min_length_ms: number;
    slice_trigger_interval_ms: number;
  };
};

const VISUAL_BAND_FREQUENCIES_HZ = [65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000] as const;

type ParameterValidationError = {
  field: string;
  code: string;
  unit: string;
  value: number | null;
  min: number | null;
  max: number | null;
  message: string;
};

const RUNTIME_PARAMETER_FIELDS: Array<keyof RuntimePreviewParameters> = [
  'audio_gain_db',
  'audio_low_eq_db',
  'audio_mid_eq_db',
  'audio_high_eq_db',
  'video_brightness_percent',
  'video_contrast_percent',
  'video_saturation_percent',
  'video_hue_rotation_degrees',
  'video_blur_radius_px',
  'video_pixel_scale_percent',
  'video_space_x_offset_px',
  'video_space_y_offset_px',
];

function isRuntimePreviewParameters(value: unknown): value is RuntimePreviewParameters {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return RUNTIME_PARAMETER_FIELDS.every((field) => typeof record[field] === 'number' && Number.isFinite(record[field]));
}

function isAudioOutputBackendMessage(value: unknown): value is AudioOutputBackendMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    record.version === 1
    && record.type === 'audio-output-backend'
    && typeof record.preferred_portaudio === 'boolean'
    && typeof record.running === 'boolean'
    && typeof record.selected_backend === 'string'
  );
}

function isAudioOutputBackendClosedMessage(value: unknown): value is AudioOutputBackendClosedMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return record.version === 1 && record.type === 'audio-output-backend-closed';
}

function isRuntimeParameterMessage(value: unknown): value is RuntimeParameterMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    record.version === 1 &&
    record.type === 'runtime-parameters' &&
    typeof record.audio_processing_enabled === 'boolean' &&
    typeof record.video_processing_enabled === 'boolean' &&
    (record.playback_generation === null || (typeof record.playback_generation === 'number' && Number.isSafeInteger(record.playback_generation))) &&
    (record.payload === null || isRuntimePreviewParameters(record.payload))
  );
}

function isPlaybackControlMessage(value: unknown): value is PlaybackControlMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return record.version === 1 && record.type === 'playback-control' && ['pause', 'resume', 'stop'].includes(record.action as string);
}

function isDiagnosticMessage(value: unknown): value is DiagnosticMessage {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  const validSamples = (samples: unknown): samples is number[] =>
    Array.isArray(samples) && samples.length <= 128 && samples.every((sample) => typeof sample === 'number' && Number.isFinite(sample));
  return (
    record.version === 1 &&
    record.type === 'diagnostic' &&
    typeof record.sent_at_ms === 'number' &&
    Number.isFinite(record.sent_at_ms) &&
    validSamples(record.waveform) &&
    validSamples(record.spectrum) &&
    (record.error === null || typeof record.error === 'string')
  );
}

function drawDiagnosticCanvas(canvas: HTMLCanvasElement | null, samples: number[], stroke: string, fill: string) {
  if (!canvas) return;
  const context = canvas.getContext('2d');
  if (!context) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.strokeStyle = stroke;
  context.fillStyle = fill;
  context.lineWidth = 2;
  context.beginPath();
  samples.forEach((sample, index) => {
    const x = samples.length <= 1 ? 0 : (index / (samples.length - 1)) * canvas.width;
    const y = canvas.height / 2 - sample * canvas.height * 0.45;
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  context.stroke();
  context.fillRect(0, canvas.height - 1, canvas.width, 1);
}

const isFinalEffectWindow =
  typeof window !== 'undefined' &&
  '__TAURI_INTERNALS__' in window &&
  getCurrentWindow().label === 'final-effect';

function toAssetUrl(path: string | null | undefined) {
  if (!path) return null;
  return convertFileSrc(path.replace(/^file:\/\//, ''));
}

type PlaybackDisplayState = 'loading' | 'no-source' | 'error' | 'disabled' | 'ready' | 'playing' | 'paused' | 'stopped' | 'unknown';

type ProcessingStatusKey = 'disabled' | 'configured' | 'processing' | 'ready' | 'runtime' | 'unavailable' | 'failed' | 'unsupported';
type MediaProcessingScope = 'video' | 'audio' | 'both';

function getProcessingStatusKey(status: string | null | undefined, enabled: boolean, configured = false): ProcessingStatusKey {
  if (!enabled) return 'disabled';
  if (status === 'unavailable' && configured) return 'configured';
  if (status === 'configured' || status === 'processing' || status === 'ready' || status === 'runtime' || status === 'unavailable' || status === 'failed') {
    return status;
  }
  return 'configured';
}

function getProcessingStatusColor(status: ProcessingStatusKey) {
  switch (status) {
    case 'ready':
    case 'runtime':
      return 'green';
    case 'processing':
      return 'processing';
    case 'failed':
      return 'red';
    case 'unavailable':
      return 'orange';
    case 'unsupported':
      return 'gold';
    case 'configured':
      return 'blue';
    default:
      return 'default';
  }
}

function getProcessingStatusLabel(status: ProcessingStatusKey) {
  switch (status) {
    case 'runtime':
      return '实时值';
    case 'ready':
      return '已生效';
    case 'configured':
      return '配置值';
    case 'processing':
      return '处理中';
    case 'unavailable':
      return '未接入';
    case 'unsupported':
      return '暂未支持';
    case 'failed':
      return '失败回退';
    default:
      return '未启用';
  }
}

function formatAudioPreviewValue(value: number | null | undefined, unit = '', digits = 3) {
  if (value === null || value === undefined || !Number.isFinite(value)) return '未设置';
  return `${value.toFixed(digits)}${unit ? ` ${unit}` : ''}`;
}

const AUDIO_PRESET_FIELD_DEFINITIONS: readonly {
  key: keyof SubtleAudioSample;
  label: string;
  unit: string;
  digits: number;
}[] = [
  { key: 'pitch_shift_semitones', label: '音高', unit: '半音', digits: 3 },
  { key: 'input_gain_db', label: '输入增益', unit: 'dB', digits: 3 },
  { key: 'output_gain_db', label: '输出增益', unit: 'dB', digits: 3 },
  { key: 'loudness_adjustment_db', label: '响度调整', unit: 'dB', digits: 3 },
  { key: 'low_eq_db', label: '低频 EQ', unit: 'dB', digits: 3 },
  { key: 'mid_eq_db', label: '中频 EQ', unit: 'dB', digits: 3 },
  { key: 'high_eq_db', label: '高频 EQ', unit: 'dB', digits: 3 },
  { key: 'filter_q', label: '滤波 Q', unit: '', digits: 3 },
  { key: 'phase_perturbation_percent', label: '相位扰动', unit: '%', digits: 3 },
  { key: 'vibrato_frequency_hz', label: '颤音频率', unit: 'Hz', digits: 3 },
  { key: 'vibrato_depth_percent', label: '颤音深度', unit: '%', digits: 3 },
  { key: 'reverb_wet_percent', label: '轻混响', unit: '%', digits: 3 },
  { key: 'noise_reduction_percent', label: '降噪', unit: '%', digits: 3 },
  { key: 'environment_noise_percent', label: '环境噪声', unit: '%', digits: 3 },
  { key: 'environment_noise_dbfs', label: '环境噪声电平', unit: 'dBFS', digits: 3 },
  { key: 'fade_in_ms', label: '淡入', unit: 'ms', digits: 3 },
  { key: 'fade_out_ms', label: '淡出', unit: 'ms', digits: 3 },
  { key: 'dry_wet_percent', label: '干湿比', unit: '%', digits: 3 },
  { key: 'ambient_sound_mix_percent', label: '环境声混合', unit: '%', digits: 3 },
  { key: 'spectral_perturbation_percent', label: '频谱微扰', unit: '%', digits: 3 },
];

function getPlaybackDisplayLabel(state: PlaybackDisplayState) {
  switch (state) {
    case 'loading':
      return '读取中';
    case 'no-source':
      return '未导入视频';
    case 'error':
      return '读取失败';
    case 'disabled':
      return '已禁用';
    case 'ready':
      return '待播放';
    case 'playing':
      return '播放中';
    case 'paused':
      return '已暂停';
    case 'stopped':
      return '已停止';
    default:
      return '未知';
  }
}

function getPlaybackDisplayColor(state: PlaybackDisplayState) {
  switch (state) {
    case 'playing':
    case 'ready':
      return 'green';
    case 'paused':
    case 'stopped':
      return 'gold';
    case 'disabled':
    case 'error':
      return 'red';
    case 'loading':
      return 'blue';
    default:
      return 'default';
  }
}

function countUnicodeCharacters(value: string) {
  return Array.from(value).length;
}

function getFixedSpeechPresetError(title: string, text: string) {
  const trimmedTitle = title.trim();
  if (!trimmedTitle) return '标题不能为空';
  if (countUnicodeCharacters(trimmedTitle) > 80) return '标题最多 80 个字符';
  return getFixedSpeechTextError(text);
}

function toGainValue(db: number) {
  return Math.pow(10, db / 20);
}

function getEffectiveAudioSource(snapshot: PlaybackSnapshot | null): BaseAudioSource {
  return resolvePlaybackAudioSource({
    effectiveAudioSource: snapshot?.effective_audio_source,
    currentAudioSource: snapshot?.current_audio_source,
    currentVideoSource: snapshot?.current_video_source,
  });
}

function buildInterludeDraft(interlude?: InterludeSnapshot | null): InterludeConfigDraft {
  return {
    enabled: interlude?.enabled ?? false,
    directory: interlude?.directory ?? null,
    intervalMinMs: interlude?.interval_min_ms ?? 8_000,
    intervalMaxMs: interlude?.interval_max_ms ?? 13_000,
    volumeDb: interlude?.volume_db ?? 0,
    duckingDepthDb: interlude?.ducking_depth_db ?? -12,
    duckingAttackMs: interlude?.ducking_attack_ms ?? 120,
    duckingReleaseMs: interlude?.ducking_release_ms ?? 240,
  };
}

function FinalEffectWindow() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const processedAudioARef = useRef<HTMLAudioElement | null>(null);
  const processedAudioBRef = useRef<HTMLAudioElement | null>(null);
  const interludeAudioRef = useRef<HTMLAudioElement | null>(null);
  const videoDryGainRef = useRef<GainNode | null>(null);
  const processedSlotGainARef = useRef<GainNode | null>(null);
  const processedSlotGainBRef = useRef<GainNode | null>(null);
  const processedActiveSlotRef = useRef<0 | 1>(0);
  const processedAudioGainTargetRef = useRef<'dry' | 'slot-a' | 'slot-b' | null>(null);
  const processedAudioPlayingRef = useRef(false);
  const processedAudioUrlRef = useRef<string | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const audioContextCleanupTimerRef = useRef<number | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  // 实时预览链：Gain + EQ + 轻混响 + 底噪；FFmpeg 仅「应用」固化，预览不当双重。
  const videoFxGainRef = useRef<GainNode | null>(null);
  const videoFxLowEqRef = useRef<BiquadFilterNode | null>(null);
  const videoFxMidEqRef = useRef<BiquadFilterNode | null>(null);
  const videoFxHighEqRef = useRef<BiquadFilterNode | null>(null);
  const videoFxReverbDelayRef = useRef<DelayNode | null>(null);
  const videoFxReverbFeedbackRef = useRef<GainNode | null>(null);
  const videoFxReverbWetRef = useRef<GainNode | null>(null);
  const videoFxNoiseGainRef = useRef<GainNode | null>(null);
  const videoFxNoiseSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const speakerMuteGainRef = useRef<GainNode | null>(null);
  const portAudioHardwareRef = useRef(false);
  const audioSourceSyncRef = useRef({ latestRequest: 0, pending: false, running: false });
  const runtimeAudioEnabledRef = useRef(false);
  const runtimeAudioParamsRef = useRef<RuntimePreviewParameters | null>(null);
  // ponytail: 参数切换 30ms 增益交叉淡化；duck/音量仍走同一出口但不强制淡化键
  const lastRealtimeFxKeyRef = useRef<string>('init');
  const interludeGainLevelRef = useRef(0);
  const duckGainLevelRef = useRef(1);
  const suppressMediaEventRef = useRef(false);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const userMutedRef = useRef(false);
  const userVolumeRef = useRef(1);
  const audioUrlRef = useRef<string | null>(null);
  const fixedSpeechActiveRef = useRef(false);
  const fixedSpeechOperationRef = useRef<{
    operationId: string;
    utterance: SpeechSynthesisUtterance | null;
    startTimer: number | null;
  } | null>(null);
  const interludeAudioUrlRef = useRef<string | null>(null);
  const audioDiagnosticsReadyRef = useRef(false);
  const realtimeAudioPlayingRef = useRef(false);
  const loopSourceKeyRef = useRef<string | null>(null);
  const loopGenerationRef = useRef<number | null>(null);
  const loopSequenceRef = useRef(0);
  const lastRestartTokenRef = useRef<string | number | null>(null);
  // ponytail: src 一切换 currentTime 先归零；用 ref 保住续播点
  const playbackPositionSecRef = useRef(0);
  const [processedAudioUrl, setProcessedAudioUrl] = useState<string | null>(null);
  const [processedAudioReady, setProcessedAudioReady] = useState(false);
  const interludeScheduleKeyRef = useRef<string | null>(null);
  const nextInterludeAtMsRef = useRef<number | null>(null);
  const lastInterludeIndexRef = useRef<number | null>(null);
  const interludeActiveRef = useRef(false);
  const interludePausedRef = useRef(false);
  const interludeStopTimerRef = useRef<number | null>(null);
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [fixedSpeechActive, setFixedSpeechActive] = useState(false);
  const [interludeAudioUrl, setInterludeAudioUrl] = useState<string | null>(null);
  const [runtimeParameters, setRuntimeParameters] = useState<RuntimePreviewParameters | null>(null);
  const [runtimeVideoProcessingEnabled, setRuntimeVideoProcessingEnabled] = useState(false);
  const nextSegmentStartRef = useRef<number | null>(null);
  const scheduleGenerationRef = useRef<number | null>(null);
  const scheduleLoopRef = useRef<number | null>(null);
  const snapshotRef = useRef<PlaybackSnapshot | null>(null);
  const finalEffectResizeKeyRef = useRef<string | null>(null);
  const workerAvailableRef = useRef(false);
  const [audioDiagnosticsReady, setAudioDiagnosticsReady] = useState(false);
  const [portAudioHardwareEnabled, setPortAudioHardwareEnabled] = useState(false);
  const [realtimeAudioPlaying, setRealtimeAudioPlaying] = useState(false);
  const [userMuted, setUserMuted] = useState(false);
  const [userVolume, setUserVolume] = useState(1);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [finalEffectResizeError, setFinalEffectResizeError] = useState<string | null>(null);
  const portAudioSourcePath = snapshot?.current_video_reference
    ?? snapshot?.source_media?.source_path
    ?? null;

  useEffect(() => {
    const html = document.documentElement;
    const body = document.body;
    const previousHtml = {
      margin: html.style.margin,
      width: html.style.width,
      height: html.style.height,
      overflow: html.style.overflow,
    };
    const previousBody = {
      margin: body.style.margin,
      width: body.style.width,
      height: body.style.height,
      overflow: body.style.overflow,
    };

    html.style.margin = '0';
    html.style.width = '100%';
    html.style.height = '100%';
    html.style.overflow = 'hidden';
    body.style.margin = '0';
    body.style.width = '100%';
    body.style.height = '100%';
    body.style.overflow = 'hidden';

    return () => {
      Object.assign(html.style, previousHtml);
      Object.assign(body.style, previousBody);
    };
  }, []);
  const snapshotSyncVersionRef = useRef(0);
  const snapshotPollInFlightRef = useRef(false);

  function applyPlayerSnapshot(nextSnapshot: PlaybackSnapshot) {
    snapshotSyncVersionRef.current += 1;
    setSnapshot(nextSnapshot);
  }

  function setRealtimeAudioPlaybackState(playing: boolean) {
    realtimeAudioPlayingRef.current = playing;
    setRealtimeAudioPlaying(playing);
    syncUserAudioSettings();
  }

  function applyRealtimeVideoFx(params: RuntimePreviewParameters | null, audioEnabled: boolean) {
    // 画面时钟独立于 Web Audio 图是否创建成功；音高保持时长，只有显式变速可改这里。
    const live = audioEnabled && params && !processedAudioPlayingRef.current;
    const video = videoRef.current;
    if (video) {
      video.playbackRate = resolveSynchronizedVideoPlaybackRate(
        Boolean(audioEnabled && params),
        params?.audio_playback_speed,
      );
    }
    const gain = videoFxGainRef.current;
    const low = videoFxLowEqRef.current;
    const mid = videoFxMidEqRef.current;
    const high = videoFxHighEqRef.current;
    const reverbWet = videoFxReverbWetRef.current;
    const reverbFeedback = videoFxReverbFeedbackRef.current;
    const noiseGain = videoFxNoiseGainRef.current;
    if (!gain || !low || !mid || !high || !reverbWet || !reverbFeedback || !noiseGain) return;
    // 真轨已挂上隐藏音轨时关实时 FX，避免双重；否则预览垫底。
    const totalDb = live
      ? (params.audio_input_gain_db ?? 0)
        + (params.audio_output_gain_db ?? 0)
        + (params.audio_loudness_adjustment_db ?? params.audio_gain_db ?? 0)
      : 0;
    const duck = interludeActiveRef.current ? duckGainLevelRef.current : 1;
    const user = clampVolume(userVolumeRef.current);
    const targetGain = (live ? 10 ** (totalDb / 20) : 1) * user * duck;
    const q = Math.max(0.3, Math.min(10, live ? (params.audio_filter_q ?? 1) : 1));
    const lowDb = live ? (params.audio_low_eq_db ?? 0) : 0;
    const midDb = live ? (params.audio_mid_eq_db ?? 0) : 0;
    const highDb = live ? (params.audio_high_eq_db ?? 0) : 0;
    const reverbRatio = live
      ? Math.max(0, Math.min(1, (params.audio_reverb_wet_percent ?? 0) / 100))
      : 0;
    const noiseRatio = live
      ? Math.max(0, Math.min(1, (params.audio_environment_noise_percent ?? 0) / 100))
      : 0;
    const noiseAmp = live
      ? Math.max(0.000_001, Math.min(1, 10 ** ((params.audio_environment_noise_dbfs ?? -40) / 20) * noiseRatio))
      : 0;
    const fxKey = buildAudioFxSignature(params, Boolean(live));
    const shouldCrossfade = fxKey !== lastRealtimeFxKeyRef.current;
    lastRealtimeFxKeyRef.current = fxKey;
    const ctx = audioContextRef.current;
    if (shouldCrossfade && ctx) {
      const now = ctx.currentTime;
      const end = now + AUDIO_PARAM_CROSSFADE_SEC;
      gain.gain.cancelScheduledValues(now);
      gain.gain.setValueAtTime(gain.gain.value, now);
      gain.gain.linearRampToValueAtTime(targetGain, end);
      for (const [node, value] of [
        [low.gain, lowDb],
        [mid.gain, midDb],
        [high.gain, highDb],
        [reverbWet.gain, reverbRatio],
        [reverbFeedback.gain, reverbRatio * 0.45],
        [noiseGain.gain, noiseAmp],
      ] as const) {
        node.cancelScheduledValues(now);
        node.setValueAtTime(node.value, now);
        node.linearRampToValueAtTime(value, end);
      }
    } else {
      gain.gain.value = targetGain;
      low.gain.value = lowDb;
      mid.gain.value = midDb;
      high.gain.value = highDb;
      reverbWet.gain.value = reverbRatio;
      reverbFeedback.gain.value = reverbRatio * 0.45;
      noiseGain.gain.value = noiseAmp;
    }
    low.Q.value = q;
    mid.Q.value = q;
    high.Q.value = q;
  }

  function scheduleProcessedAudioCrossfade(processedActive: boolean, activeSlot: 0 | 1) {
    const target = processedActive ? (activeSlot === 0 ? 'slot-a' : 'slot-b') : 'dry';
    if (processedAudioGainTargetRef.current === target) return;
    const context = audioContextRef.current;
    if (!context) return;
    processedAudioGainTargetRef.current = target;
    const now = context.currentTime;
    const end = now + AUDIO_PARAM_CROSSFADE_SEC;
    const gains: Array<[GainNode | null, number]> = [
      [videoDryGainRef.current, processedActive ? 0 : 1],
      [processedSlotGainARef.current, processedActive && activeSlot === 0 ? 1 : 0],
      [processedSlotGainBRef.current, processedActive && activeSlot === 1 ? 1 : 0],
    ];
    for (const [gain, value] of gains) {
      if (!gain) continue;
      gain.gain.cancelScheduledValues(now);
      gain.gain.setValueAtTime(gain.gain.value, now);
      gain.gain.linearRampToValueAtTime(value, end);
    }
  }

  function syncUserAudioSettings() {
    const video = videoRef.current;
    const audio = audioRef.current;
    const processedA = processedAudioARef.current;
    const processedB = processedAudioBRef.current;
    const interludeAudio = interludeAudioRef.current;
    const volume = userVolumeRef.current;
    const muted = userMutedRef.current;
    const hardwareOut = portAudioHardwareRef.current;
    const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
    const replacementAudioActive =
      fixedSpeechActiveRef.current ||
      (audioDiagnosticsReadyRef.current &&
        effectiveAudioSource === 'realtime_variant' &&
        realtimeAudioPlayingRef.current);
    const processedActive =
      !replacementAudioActive && processedAudioPlayingRef.current;
    const activeSlot = processedActiveSlotRef.current;
    const graphOwnsVideo = Boolean(videoFxGainRef.current);
    scheduleProcessedAudioCrossfade(processedActive, activeSlot);
    if (speakerMuteGainRef.current) {
      speakerMuteGainRef.current.gain.value = hardwareOut || muted ? 0 : 1;
    }
    if (processedActive) {
      if (videoFxReverbWetRef.current) videoFxReverbWetRef.current.gain.value = 0;
      if (videoFxReverbFeedbackRef.current) videoFxReverbFeedbackRef.current.gain.value = 0;
      if (videoFxNoiseGainRef.current) videoFxNoiseGainRef.current.gain.value = 0;
    }
    if (video) {
      video.volume = graphOwnsVideo
        ? 1
        : clampVolume(volume * (interludeActiveRef.current ? duckGainLevelRef.current : 1));
      video.muted = hardwareOut
        ? replacementAudioActive || processedActive
        : replacementAudioActive || processedActive || muted;
    }
    if (audio) {
      audio.volume = volume;
      audio.muted = hardwareOut ? fixedSpeechActiveRef.current : muted || fixedSpeechActiveRef.current;
    }
    for (const el of [processedA, processedB]) {
      if (!el) continue;
      el.volume = 1;
      el.muted = hardwareOut ? false : muted;
    }
    if (interludeAudio) {
      // 插话自身用电平；主轨 duck 已在 videoFxGain。PA 模式下元素仍出声进 tap。
      interludeAudio.volume = clampVolume(volume * interludeGainLevelRef.current);
      interludeAudio.muted = hardwareOut ? false : muted;
    }
    applyRealtimeVideoFx(
      runtimeAudioParamsRef.current,
      runtimeAudioEnabledRef.current && !replacementAudioActive && !processedActive && !(muted && !hardwareOut),
    );
  }

  function setPortAudioHardwareActive(requested: boolean) {
    const nextActive = requested && PORTAUDIO_FORMAL_SOURCE_SYNC_READY;
    if (portAudioHardwareRef.current === nextActive) {
      syncUserAudioSettings();
      return;
    }
    portAudioHardwareRef.current = nextActive;
    setPortAudioHardwareEnabled(nextActive);
    syncUserAudioSettings();
  }

  function clearInterludeStopTimer() {
    if (interludeStopTimerRef.current !== null) {
      window.clearTimeout(interludeStopTimerRef.current);
      interludeStopTimerRef.current = null;
    }
  }

  function clearInterludePlayback(options?: {
    releaseMs?: number;
    resetSchedule?: boolean;
    resetIndex?: boolean;
    clearSource?: boolean;
  }) {
    const releaseMs = options?.releaseMs ?? 0;
    const resetSchedule = options?.resetSchedule ?? true;
    const resetIndex = options?.resetIndex ?? false;
    const clearSource = options?.clearSource ?? true;
    clearInterludeStopTimer();
    interludeActiveRef.current = false;
    interludePausedRef.current = false;
    interludeGainLevelRef.current = 0;
    duckGainLevelRef.current = 1;
    syncUserAudioSettings();
    if (resetSchedule) nextInterludeAtMsRef.current = null;
    if (resetIndex) lastInterludeIndexRef.current = null;
    const finalize = () => {
      const interludeAudio = interludeAudioRef.current;
      if (interludeAudio) {
        interludeAudio.pause();
        interludeAudio.currentTime = 0;
        if (clearSource) {
          interludeAudio.removeAttribute('src');
          interludeAudio.load();
        }
      }
      if (clearSource) {
        interludeAudioUrlRef.current = null;
        setInterludeAudioUrl(null);
      }
      interludeStopTimerRef.current = null;
    };
    if (releaseMs <= 0) {
      finalize();
      return;
    }
    interludeStopTimerRef.current = window.setTimeout(finalize, releaseMs + 40);
  }

  function pauseInterludePlayback() {
    if (!interludeActiveRef.current) return;
    interludePausedRef.current = true;
    interludeAudioRef.current?.pause();
  }

  function resumeInterludePlayback() {
    if (snapshotRef.current?.playback_state !== 'playing') return;
    if (!interludeActiveRef.current || !interludePausedRef.current || !interludeAudioUrlRef.current) return;
    interludePausedRef.current = false;
    void interludeAudioRef.current?.play().catch(() => undefined);
  }

  function publishFixedSpeechStatus(
    operationId: string,
    status: FixedSpeechStatus,
    error: string | null = null,
  ) {
    try {
      playbackChannelRef.current?.postMessage({
        version: 1,
        type: 'fixed-speech-status',
        operation_id: operationId,
        status,
        error,
      } satisfies FixedSpeechStatusMessage);
    } catch {
      // 窗口关闭时通道可能已经失效，本地恢复仍必须继续。
    }
  }

  function finalizeFixedSpeech(
    operationId: string,
    status: Exclude<FixedSpeechStatus, 'starting' | 'playing'>,
    error: string | null = null,
  ) {
    const operation = fixedSpeechOperationRef.current;
    if (!operation || operation.operationId !== operationId) return;
    fixedSpeechOperationRef.current = null;
    if (operation.startTimer !== null) window.clearTimeout(operation.startTimer);
    window.speechSynthesis?.cancel();
    fixedSpeechActiveRef.current = false;
    setFixedSpeechActive(false);
    syncUserAudioSettings();
    resumeInterludePlayback();
    publishFixedSpeechStatus(operationId, status, error);
  }

  function cancelFixedSpeech(operationId: string) {
    finalizeFixedSpeech(operationId, 'cancelled');
  }

  async function waitForLocalSpeechVoice(operationId: string): Promise<SpeechSynthesisVoice | null> {
    const synthesis = window.speechSynthesis;
    const available = selectLocalSpeechVoice(synthesis.getVoices());
    if (available) return available;
    return new Promise((resolve) => {
      const finish = () => {
        synthesis.removeEventListener('voiceschanged', handleVoicesChanged);
        window.clearTimeout(timer);
        resolve(
          fixedSpeechOperationRef.current?.operationId === operationId
            ? selectLocalSpeechVoice(synthesis.getVoices())
            : null,
        );
      };
      const handleVoicesChanged = () => finish();
      const timer = window.setTimeout(finish, 1_500);
      synthesis.addEventListener('voiceschanged', handleVoicesChanged, { once: true });
    });
  }

  async function startFixedSpeech(message: Extract<FixedSpeechCommandMessage, { action: 'speak' }>) {
    const previousOperationId = fixedSpeechOperationRef.current?.operationId;
    if (previousOperationId) cancelFixedSpeech(previousOperationId);
    if (!('speechSynthesis' in window) || typeof SpeechSynthesisUtterance === 'undefined') {
      publishFixedSpeechStatus(message.operation_id, 'failed', '当前 Windows WebView 不支持系统语音');
      return;
    }

    fixedSpeechOperationRef.current = {
      operationId: message.operation_id,
      utterance: null,
      startTimer: null,
    };
    // 固定话术从等待本地 voice 的 starting 阶段就暂停插话，避免等待期间又触发随机插话。
    fixedSpeechActiveRef.current = true;
    setFixedSpeechActive(true);
    pauseInterludePlayback();
    syncUserAudioSettings();
    publishFixedSpeechStatus(message.operation_id, 'starting');
    const voice = await waitForLocalSpeechVoice(message.operation_id);
    const operation = fixedSpeechOperationRef.current;
    if (!operation || operation.operationId !== message.operation_id) return;
    if (!voice) {
      finalizeFixedSpeech(message.operation_id, 'failed', '未检测到本地系统语音，请在 Windows 中安装语音包');
      return;
    }

    const utterance = new SpeechSynthesisUtterance(message.text.trim());
    utterance.voice = voice;
    utterance.lang = voice.lang || 'zh-CN';
    // 用户静音只作用于视频/候选音轨，不能让用户主动发起的固定话术也变成静音。
    utterance.volume = 1;
    utterance.onstart = () => {
      const current = fixedSpeechOperationRef.current;
      if (!current || current.operationId !== message.operation_id) return;
      if (current.startTimer !== null) {
        window.clearTimeout(current.startTimer);
        current.startTimer = null;
      }
      publishFixedSpeechStatus(message.operation_id, 'playing');
    };
    utterance.onend = () => finalizeFixedSpeech(message.operation_id, 'completed');
    utterance.onerror = (event) => {
      const cancelled = event.error === 'canceled' || event.error === 'interrupted';
      finalizeFixedSpeech(
        message.operation_id,
        cancelled ? 'cancelled' : 'failed',
        cancelled ? null : `系统语音播放失败：${event.error}`,
      );
    };
    operation.utterance = utterance;
    operation.startTimer = window.setTimeout(() => {
      finalizeFixedSpeech(message.operation_id, 'failed', '系统语音启动超时，请检查 Windows 语音设置');
    }, FIXED_SPEECH_ACK_TIMEOUT_MS);
    window.speechSynthesis.speak(utterance);
  }

  function handleRealtimeAudioPlaying() {
    if (!audioDiagnosticsReadyRef.current || getEffectiveAudioSource(snapshotRef.current) !== 'realtime_variant') return;
    setRealtimeAudioPlaybackState(true);
  }

  function handleRealtimeAudioEnded() {
    setRealtimeAudioPlaybackState(false);
    const currentSnapshot = snapshotRef.current;
    if (
      currentSnapshot?.current_audio_source === 'realtime_variant' &&
      !currentSnapshot.pending_audio_candidate
    ) {
      void invoke<PlaybackSnapshot>('restore_original_audio')
        .then(applyPlayerSnapshot)
        .catch(() => undefined);
    }
  }

  function handleRealtimeAudioElementError(event: SyntheticEvent<HTMLAudioElement>) {
    setRealtimeAudioPlaybackState(false);
    const detail = event.currentTarget.error?.message;
    setPlaybackError(detail ? `实时音频候选播放失败：${detail}，已暂时使用源音轨。` : '实时音频候选播放失败，已暂时使用源音轨。');
    const currentSnapshot = snapshotRef.current;
    if (currentSnapshot?.current_audio_source !== 'realtime_variant' || currentSnapshot.pending_audio_candidate) return;
    void invoke<PlaybackSnapshot>('restore_original_audio')
      .then(applyPlayerSnapshot)
      .catch(() => undefined);
  }

  function handleInterludeEnded() {
    clearInterludeStopTimer();
    interludeActiveRef.current = false;
    interludePausedRef.current = false;
    interludeAudioUrlRef.current = null;
    setInterludeAudioUrl(null);
    interludeGainLevelRef.current = 0;
    duckGainLevelRef.current = 1;
    syncUserAudioSettings();
    nextInterludeAtMsRef.current = null;
  }

  function handleInterludeError(event: SyntheticEvent<HTMLAudioElement>) {
    if (!interludeActiveRef.current) return;
    const detail = event.currentTarget.error?.message;
    setPlaybackError(detail ? `插话音频播放失败：${detail}` : '插话音频播放失败，请检查文件格式和文件权限。');
    handleInterludeEnded();
  }

  function startInterludePlayback(interlude: InterludeSnapshot) {
    const count = interlude.audio_files.length;
    const nextIndex = chooseInterludeIndex(count, lastInterludeIndexRef.current);
    if (nextIndex === null) return;
    const selectedUrl = toAssetUrl(interlude.audio_files[nextIndex]);
    if (!selectedUrl) return;
    clearInterludeStopTimer();
    lastInterludeIndexRef.current = nextIndex;
    nextInterludeAtMsRef.current = null;
    interludeActiveRef.current = true;
    interludePausedRef.current = false;
    interludeGainLevelRef.current = toGainValue(interlude.volume_db);
    duckGainLevelRef.current = toGainValue(interlude.ducking_depth_db);
    interludeAudioUrlRef.current = selectedUrl;
    setInterludeAudioUrl(selectedUrl);
    syncUserAudioSettings();
  }

  function publishMediaState() {
    const channel = playbackChannelRef.current;
    const video = videoRef.current;
    if (!channel || !video) return;
    const duration = Number.isFinite(video.duration) && video.duration >= 0 ? video.duration : 0;
    const currentTime = clampMediaTime(video.currentTime, duration);
    void invoke<PlaybackSnapshot>('update_playback_position', {
      request: { position_ms: Math.max(0, Math.round(currentTime * 1000)) },
    }).catch(() => undefined);
    try {
      channel.postMessage({
        version: 1,
        type: 'playback-media-state',
        current_time: currentTime,
        duration,
        volume: clampVolume(video.volume),
        muted: userMutedRef.current,
        paused: video.paused,
      } satisfies PlaybackMediaStateMessage);
    } catch {
      // 播放器关闭时通道可能已失效，媒体播放不应因此失败。
    }
  }

  function applyPlaybackMediaControl(message: PlaybackMediaControlMessage) {
    const video = videoRef.current;
    if (!video) return;
    if (message.action === 'seek') {
      video.currentTime = clampMediaTime(message.current_time, video.duration);
      const audio = audioRef.current;
      if (audio) {
        audio.currentTime = Math.max(0, video.currentTime - (snapshotRef.current?.current_audio_start_at_ms ?? 0) / 1000);
      }
      publishMediaState();
      return;
    }
    if (message.action === 'set-volume') {
      userVolumeRef.current = clampVolume(message.volume);
      setUserVolume(userVolumeRef.current);
      syncUserAudioSettings();
      publishMediaState();
      return;
    }
    if (message.action === 'toggle-muted') {
      userMutedRef.current = !userMutedRef.current;
      setUserMuted(userMutedRef.current);
      syncUserAudioSettings();
      publishMediaState();
      return;
    }

  }

  useEffect(() => {
    if (typeof BroadcastChannel === 'undefined') return;
    let channel: BroadcastChannel;
    try {
      channel = new BroadcastChannel(PLAYBACK_CHANNEL_NAME);
    } catch {
      return;
    }
    playbackChannelRef.current = channel;
    const handleMessage = (event: MessageEvent<unknown>) => {
      if (isFixedSpeechCommandMessage(event.data)) {
        if (event.data.action === 'cancel') cancelFixedSpeech(event.data.operation_id);
        else void startFixedSpeech(event.data);
        return;
      }
      if (isPlaybackMediaControlMessage(event.data)) {
        void applyPlaybackMediaControl(event.data);
        return;
      }
      if (isPlaybackControlMessage(event.data)) {
        const video = videoRef.current;
        const audio = audioRef.current;
        if (event.data.action === 'stop') {
          video?.pause();
          if (video) video.currentTime = 0;
          audio?.pause();
          if (audio) audio.currentTime = 0;
          const operationId = fixedSpeechOperationRef.current?.operationId;
          if (operationId) cancelFixedSpeech(operationId);
        } else if (event.data.action === 'pause') {
          video?.pause();
          audio?.pause();
          window.speechSynthesis?.pause();
        } else if (video) {
          if (event.data.action === 'resume') resumeAudioDiagnostics();
          window.speechSynthesis?.resume();
          void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
          const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
          if (effectiveAudioSource === 'realtime_variant' && audioUrlRef.current && audioDiagnosticsReadyRef.current && audio) {
            void audio.play().catch(() => undefined);
          }
        }
        return;
      }
      if (isAudioOutputBackendMessage(event.data)) {
        setPortAudioHardwareActive(
          event.data.preferred_portaudio
            && event.data.running
            && event.data.selected_backend === 'portaudio'
            && PORTAUDIO_FORMAL_SOURCE_SYNC_READY,
        );
        return;
      }
      if (!isRuntimeParameterMessage(event.data)) return;
      const currentGeneration = snapshotRef.current?.playback_generation;
      if (
        currentGeneration !== undefined &&
        event.data.playback_generation !== null &&
        event.data.playback_generation !== currentGeneration
      ) {
        return;
      }
      setRuntimeParameters(event.data.payload);
      setRuntimeVideoProcessingEnabled(event.data.video_processing_enabled);
      runtimeAudioEnabledRef.current = event.data.audio_processing_enabled;
      runtimeAudioParamsRef.current = event.data.payload;
      void audioContextRef.current?.resume().catch(() => undefined);
      applyRealtimeVideoFx(event.data.payload, event.data.audio_processing_enabled);
      syncUserAudioSettings();
    };
    channel.addEventListener('message', handleMessage);
    const handlePageHide = () => {
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      setPortAudioHardwareActive(false);
      try {
        channel.postMessage({
          version: 1,
          type: 'audio-output-backend-closed',
        } satisfies AudioOutputBackendClosedMessage);
      } catch {
        // 通道可能已关
      }
    };
    window.addEventListener('pagehide', handlePageHide);
    return () => {
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      window.removeEventListener('pagehide', handlePageHide);
      channel.removeEventListener('message', handleMessage);
      channel.close();
      if (playbackChannelRef.current === channel) playbackChannelRef.current = null;
    };
  }, []);

  useEffect(() => {
    const channel = playbackChannelRef.current;
    const timer = window.setInterval(() => {
      const analyser = analyserRef.current;
      if (!channel || !analyser) return;
      const waveform = new Uint8Array(analyser.fftSize);
      const spectrum = new Uint8Array(analyser.frequencyBinCount);
      analyser.getByteTimeDomainData(waveform);
      analyser.getByteFrequencyData(spectrum);
      channel.postMessage({
        version: 1,
        type: 'diagnostic',
        waveform: Array.from(waveform.slice(0, DIAGNOSTIC_SAMPLE_COUNT), (sample) => (sample - 128) / 128),
        spectrum: Array.from(spectrum.slice(0, DIAGNOSTIC_SAMPLE_COUNT), (sample) => sample / 255),
        sent_at_ms: Date.now(),
        error: playbackError,
      } satisfies DiagnosticMessage);
    }, DIAGNOSTIC_PUBLISH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [playbackError]);

  function resumeAudioDiagnostics() {
    void audioContextRef.current?.resume().catch(() => undefined);
  }

  useEffect(() => {
    snapshotRef.current = snapshot;
  }, [snapshot]);

  useEffect(() => {
    userMutedRef.current = userMuted;
    syncUserAudioSettings();
  }, [userMuted]);

  useEffect(() => {
    userVolumeRef.current = userVolume;
    syncUserAudioSettings();
  }, [userVolume]);

  useEffect(() => {
    workerAvailableRef.current = Boolean(workerCapabilities?.available);
  }, [workerCapabilities?.available]);

  useEffect(() => {
    setPlaybackError(null);
  }, [sourceUrl]);

  useEffect(() => {
    const refreshSnapshot = () => {
      if (snapshotPollInFlightRef.current) return;
      snapshotPollInFlightRef.current = true;
      const version = snapshotSyncVersionRef.current;
      void invoke<PlaybackSnapshot>('get_snapshot').then((nextSnapshot) => {
        if (version === snapshotSyncVersionRef.current) applyPlayerSnapshot(nextSnapshot);
      }).catch(() => undefined).finally(() => {
        snapshotPollInFlightRef.current = false;
      });
    };
    refreshSnapshot();
    const timer = window.setInterval(refreshSnapshot, PLAYBACK_SNAPSHOT_POLL_MS);
    return () => window.clearInterval(timer);
  }, []);

  // 最终效果窗自行探测 PortAudio，避免只依赖 BroadcastChannel 时序。
  useEffect(() => {
    let cancelled = false;
    let realignBusy = false;
    let realignAttemptedForHz: number | null = null;
    let realignFailedForHz: number | null = null;
    let outputStatusPollInFlight = false;
    const refreshOutput = () => {
      if (outputStatusPollInFlight) return;
      outputStatusPollInFlight = true;
      void invoke<AudioOutputBackendStatus>('get_audio_output_backend_status')
        .then(async (status) => {
          if (cancelled) return;
          const targetRate = PORTAUDIO_SAMPLE_RATE_HZ;
          const hardware =
            PORTAUDIO_FORMAL_SOURCE_SYNC_READY
            && Boolean(status.preferred_portaudio)
            && Boolean(status.running)
            && status.selected_backend === 'portaudio';
          const needsRealign =
            hardware
            && Math.abs(targetRate - (status.sample_rate_hz || 0)) > 1
            && !realignBusy
            && realignAttemptedForHz !== targetRate
            && realignFailedForHz !== targetRate;
          // 同一目标 SR 只对齐一次；失败则退避，避免每 2s 重开流抖动。
          if (needsRealign) {
            realignBusy = true;
            realignAttemptedForHz = targetRate;
            try {
              const next = await invoke<AudioOutputBackendStatus>('set_audio_output_backend', {
                request: {
                  prefer_portaudio: true,
                  device_index: status.device_index,
                  memory_buffer_kib: status.memory_buffer_kib,
                  frames_per_buffer: status.frames_per_buffer,
                  sample_rate_hz: targetRate,
                  position_ms: (() => {
                    const video = videoRef.current;
                    return video && Number.isFinite(video.currentTime)
                      ? Math.max(0, Math.round(video.currentTime * 1000))
                      : Math.max(0, Math.round(snapshotRef.current?.current_position_ms ?? 0));
                  })(),
                },
              });
              if (cancelled) return;
              const aligned =
                PORTAUDIO_FORMAL_SOURCE_SYNC_READY
                && Boolean(next.preferred_portaudio)
                && Boolean(next.running)
                && next.selected_backend === 'portaudio';
              if (!aligned || Math.abs(targetRate - (next.sample_rate_hz || 0)) > 1) {
                realignFailedForHz = targetRate;
              }
              setPortAudioHardwareActive(aligned);
            } catch {
              realignFailedForHz = targetRate;
              setPortAudioHardwareActive(hardware);
            } finally {
              realignBusy = false;
            }
            return;
          }
          setPortAudioHardwareActive(hardware);
        })
        .catch(() => undefined)
        .finally(() => {
          outputStatusPollInFlight = false;
        });
    };
    refreshOutput();
    const timer = window.setInterval(refreshOutput, AUDIO_OUTPUT_STATUS_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  // 图只建一次：video → Gain/EQ → Analyser → speakerMute → destination。换 src 不拆图。
  useEffect(() => {
    let cancelled = false;
    let retryTimer: number | null = null;

    const ensureGraph = () => {
      if (cancelled) return;
      if (audioContextCleanupTimerRef.current !== null) {
        window.clearTimeout(audioContextCleanupTimerRef.current);
        audioContextCleanupTimerRef.current = null;
      }
      if (audioContextRef.current && videoFxGainRef.current) {
        void audioContextRef.current.resume().catch(() => undefined);
        audioDiagnosticsReadyRef.current = true;
        setAudioDiagnosticsReady(true);
        syncUserAudioSettings();
        return;
      }
      const video = videoRef.current;
      const audio = audioRef.current;
      const processedA = processedAudioARef.current;
      const processedB = processedAudioBRef.current;
      const interlude = interludeAudioRef.current;
      if (!video || !audio || !processedA || !processedB || !interlude) {
        retryTimer = window.setTimeout(ensureGraph, 50);
        return;
      }
      let context: AudioContext | null = null;
      try {
        // WebView 诊断图与 PortAudio 正式出口统一使用 44.1k。
        context = new AudioContext({ sampleRate: PORTAUDIO_SAMPLE_RATE_HZ });
        const analyser = context.createAnalyser();
        analyser.fftSize = 2_048;
        analyser.smoothingTimeConstant = 0.75;
        const gain = context.createGain();
        const low = context.createBiquadFilter();
        const mid = context.createBiquadFilter();
        const high = context.createBiquadFilter();
        const reverbDelay = context.createDelay(0.2);
        const reverbFeedback = context.createGain();
        const reverbWet = context.createGain();
        const noiseGain = context.createGain();
        low.type = 'peaking';
        mid.type = 'peaking';
        high.type = 'peaking';
        low.frequency.value = 200;
        mid.frequency.value = 1_000;
        high.frequency.value = 8_000;
        low.Q.value = 1;
        mid.Q.value = 1;
        high.Q.value = 1;
        reverbDelay.delayTime.value = 0.08;
        reverbFeedback.gain.value = 0;
        reverbWet.gain.value = 0;
        noiseGain.gain.value = 0;
        // ponytail: 1s 白噪循环；环境噪声只调 gain
        const noiseBuffer = context.createBuffer(1, Math.max(1, Math.floor(context.sampleRate)), context.sampleRate);
        const noiseData = noiseBuffer.getChannelData(0);
        for (let i = 0; i < noiseData.length; i += 1) noiseData[i] = Math.random() * 2 - 1;
        const noiseSource = context.createBufferSource();
        noiseSource.buffer = noiseBuffer;
        noiseSource.loop = true;
        // 画面锁源片；A/B 真轨槽预加载，就绪再切，加载期不断声。
        const dryGain = context.createGain();
        const slotGainA = context.createGain();
        const slotGainB = context.createGain();
        dryGain.gain.value = 1;
        slotGainA.gain.value = 0;
        slotGainB.gain.value = 0;
        context.createMediaElementSource(video).connect(dryGain);
        dryGain.connect(gain);
        gain.connect(low);
        low.connect(mid);
        mid.connect(high);
        high.connect(analyser);
        high.connect(reverbDelay);
        reverbDelay.connect(reverbFeedback);
        reverbFeedback.connect(reverbDelay);
        reverbDelay.connect(reverbWet);
        reverbWet.connect(analyser);
        noiseSource.connect(noiseGain);
        noiseGain.connect(analyser);
        noiseSource.start(0);
        context.createMediaElementSource(processedA).connect(slotGainA);
        context.createMediaElementSource(processedB).connect(slotGainB);
        slotGainA.connect(analyser);
        slotGainB.connect(analyser);
        context.createMediaElementSource(audio).connect(analyser);
        context.createMediaElementSource(interlude).connect(analyser);
        videoDryGainRef.current = dryGain;
        processedSlotGainARef.current = slotGainA;
        processedSlotGainBRef.current = slotGainB;
        const speakerMute = context.createGain();
        speakerMute.gain.value = 1;
        analyser.connect(speakerMute);
        speakerMute.connect(context.destination);
        audioContextRef.current = context;
        analyserRef.current = analyser;
        videoFxGainRef.current = gain;
        videoFxLowEqRef.current = low;
        videoFxMidEqRef.current = mid;
        videoFxHighEqRef.current = high;
        videoFxReverbDelayRef.current = reverbDelay;
        videoFxReverbFeedbackRef.current = reverbFeedback;
        videoFxReverbWetRef.current = reverbWet;
        videoFxNoiseGainRef.current = noiseGain;
        videoFxNoiseSourceRef.current = noiseSource;
        speakerMuteGainRef.current = speakerMute;
        audioDiagnosticsReadyRef.current = true;
        setAudioDiagnosticsReady(true);
        void context.resume().catch(() => undefined);
        syncUserAudioSettings();
      } catch (cause) {
        try {
          videoFxNoiseSourceRef.current?.stop();
        } catch {
          // ignore
        }
        void context?.close().catch(() => undefined);
        audioContextRef.current = null;
        analyserRef.current = null;
        videoFxGainRef.current = null;
        videoFxLowEqRef.current = null;
        videoFxMidEqRef.current = null;
        videoFxHighEqRef.current = null;
        videoFxReverbDelayRef.current = null;
        videoFxReverbFeedbackRef.current = null;
        videoFxReverbWetRef.current = null;
        videoFxNoiseGainRef.current = null;
        videoFxNoiseSourceRef.current = null;
        videoDryGainRef.current = null;
        processedSlotGainARef.current = null;
        processedSlotGainBRef.current = null;
        processedAudioGainTargetRef.current = null;
        speakerMuteGainRef.current = null;
        audioDiagnosticsReadyRef.current = false;
        setAudioDiagnosticsReady(false);
        setPlaybackError(cause instanceof Error ? `音频图初始化失败：${cause.message}` : '音频图初始化失败');
      }
    };

    ensureGraph();
    return () => {
      cancelled = true;
      if (retryTimer !== null) window.clearTimeout(retryTimer);
      if (audioContextCleanupTimerRef.current !== null) {
        window.clearTimeout(audioContextCleanupTimerRef.current);
      }
      // React StrictMode/HMR 可能只短暂清理后立即重新挂载；媒体元素不能绑定第二个
      // MediaElementSourceNode，因此给同一窗口的图一次短暂复用机会，再释放资源。
      audioContextCleanupTimerRef.current = window.setTimeout(() => {
        audioContextCleanupTimerRef.current = null;
        const context = audioContextRef.current;
        audioContextRef.current = null;
        analyserRef.current = null;
        try {
          videoFxNoiseSourceRef.current?.stop();
        } catch {
          // ignore
        }
        videoFxGainRef.current = null;
        videoFxLowEqRef.current = null;
        videoFxMidEqRef.current = null;
        videoFxHighEqRef.current = null;
        videoFxReverbDelayRef.current = null;
        videoFxReverbFeedbackRef.current = null;
        videoFxReverbWetRef.current = null;
        videoFxNoiseGainRef.current = null;
        videoFxNoiseSourceRef.current = null;
        videoDryGainRef.current = null;
        processedSlotGainARef.current = null;
        processedSlotGainBRef.current = null;
        processedAudioGainTargetRef.current = null;
        speakerMuteGainRef.current = null;
        portAudioHardwareRef.current = false;
        setPortAudioHardwareEnabled(false);
        audioDiagnosticsReadyRef.current = false;
        setAudioDiagnosticsReady(false);
        void context?.close().catch(() => undefined);
      }, 100);
    };
  }, []);

  // 换源：只 resume + 应用当前实时参数，不重建图。
  useEffect(() => {
    if (!sourceUrl) return;
    void audioContextRef.current?.resume().catch(() => undefined);
    syncUserAudioSettings();
  }, [sourceUrl]);

  function syncAudioOutputSourceLatest() {
    const sync = audioSourceSyncRef.current;
    sync.latestRequest += 1;
    sync.pending = true;
    if (sync.running) return;
    sync.running = true;
    void (async () => {
      let latestFailure: string | null = null;
      try {
        while (sync.pending) {
          sync.pending = false;
          const requestId = sync.latestRequest;
          try {
            const video = videoRef.current;
            const position_ms = video && Number.isFinite(video.currentTime)
              ? Math.max(0, Math.round(video.currentTime * 1000))
              : Math.max(0, Math.round(snapshotRef.current?.current_position_ms ?? 0));
            const status = await invoke('sync_audio_output_source', {
              request: { position_ms },
            }) as AudioOutputBackendStatus;
            if (requestId !== sync.latestRequest) continue;
            latestFailure =
              status.preferred_portaudio && status.selected_backend === 'portaudio' && status.running
                ? null
                : status.reason ?? '当前源音频不可用。';
          } catch (cause) {
            if (requestId === sync.latestRequest) {
              latestFailure = getDisplayErrorMessage(cause, '当前源音频不可用。');
            }
          }
        }
        if (latestFailure && sync.latestRequest === audioSourceSyncRef.current.latestRequest) {
          if (portAudioHardwareRef.current) setPortAudioHardwareActive(false);
          setPlaybackError(`PortAudio 源同步失败，已回退 WebView：${latestFailure}`);
        }
      } finally {
        sync.running = false;
        if (sync.pending) {
          // 最新源在本次 IPC 结束后再同步，避免并发调用。
          syncAudioOutputSourceLatest();
        }
      }
    })();
  }

  useEffect(() => {
    if (!portAudioHardwareEnabled || !portAudioSourcePath) return;
    syncAudioOutputSourceLatest();
    const sync = audioSourceSyncRef.current;
    return () => {
      sync.latestRequest += 1;
      sync.pending = false;
    };
  }, [
    portAudioHardwareEnabled,
    portAudioSourcePath,
    snapshot?.current_video_reference,
    snapshot?.source_media?.source_path,
    snapshot?.playback_generation,
    snapshot?.loop_index,
    snapshot?.audio_stream_revision,
  ]);

  useEffect(() => {
    void invoke<SpeechToSpeechWorkerCapabilities>('get_speech_to_speech_worker_capabilities')
      .then(setWorkerCapabilities)
      .catch(() => setWorkerCapabilities(null));
  }, []);

  useEffect(() => {
    // 普通声音已改为独立流式出口，视频缓存可以安全地只承担画面处理；
    // PortAudio 会从源视频取音频，WebView 回退则使用缓存中的源 AAC。
    const useProcessedVideo =
      snapshot?.current_video_source === 'processed'
      && Boolean(snapshot.current_video_reference);
    const path = useProcessedVideo
      ? snapshot.current_video_reference
      : (snapshot?.source_media?.source_path ?? null);
    const nextUrl = toAssetUrl(path);
    setSourceUrl((prev) => (prev === nextUrl ? prev : nextUrl));
  }, [
    snapshot?.audio_processing_enabled,
    snapshot?.current_video_reference,
    snapshot?.current_video_source,
    snapshot?.source_media?.source_path,
  ]);

  useEffect(() => {
    // 兼容旧快照：只有旧版仍标记为 ready 的处理音频才加载隐藏轨。
    // 新的 runtime 路径不保存处理后音频文件，不能把视频缓存误当成处理音轨。
    const audioLive = Boolean(snapshot?.audio_processing_enabled);
    const ref = snapshot?.audio_processing_status === 'ready'
      && snapshot?.current_video_source === 'processed'
      ? snapshot.current_video_reference
      : null;
    const next = audioLive && ref ? toAssetUrl(ref) : null;
    setProcessedAudioUrl((prev) => (prev === next ? prev : next));
    if (!next) {
      processedAudioPlayingRef.current = false;
      setProcessedAudioReady(false);
    }
  }, [
    snapshot?.audio_processing_enabled,
    snapshot?.current_video_reference,
    snapshot?.current_video_source,
    snapshot?.audio_processing_status,
  ]);

  useEffect(() => {
    const operationId = fixedSpeechOperationRef.current?.operationId;
    if (operationId) cancelFixedSpeech(operationId);
  }, [snapshot?.playback_generation, snapshot?.source_media?.source_path]);

  useEffect(() => {
    const source = snapshot?.source_media;
    const resizeKey = buildFinalEffectWindowResizeKey({
      width: source?.width,
      height: source?.height,
    });
    if (!resizeKey || resizeKey === finalEffectResizeKeyRef.current) return;

    let cancelled = false;
    let retryTimer: number | null = null;
    let attempts = 0;
    const resizeWindow = () => {
      attempts += 1;
      void invoke('resize_final_effect_window', {
        request: { width: source?.width, height: source?.height },
      })
        .then(() => {
          if (cancelled) return;
          finalEffectResizeKeyRef.current = resizeKey;
          setFinalEffectResizeError(null);
        })
        .catch((cause) => {
          if (cancelled) return;
          if (attempts < 3) {
            retryTimer = window.setTimeout(resizeWindow, 200);
            return;
          }
          setFinalEffectResizeError(cause instanceof Error ? cause.message : '最终效果窗口尺寸调整失败');
        });
    };
    resizeWindow();
    return () => {
      cancelled = true;
      if (retryTimer !== null) window.clearTimeout(retryTimer);
    };
  }, [
    snapshot?.source_media?.height,
    snapshot?.source_media?.width,
  ]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !sourceUrl) return;
    const resumeAt = playbackPositionSecRef.current;
    let cancelled = false;
    const restore = () => {
      if (cancelled) return;
      if (resumeAt > 0.05 && Number.isFinite(video.duration) && video.duration > 0) {
        const safeEnd = Math.max(0, video.duration - 0.2);
        video.currentTime = Math.min(resumeAt, safeEnd);
        playbackPositionSecRef.current = video.currentTime;
      }
      void audioContextRef.current?.resume().catch(() => undefined);
      userMutedRef.current = false;
      syncUserAudioSettings();
      if (snapshotRef.current?.playback_state === 'playing') {
        suppressMediaEventRef.current = true;
        void video.play().catch(() => {
          suppressMediaEventRef.current = false;
          setPlaybackError('处理结果已切换，但自动播放被拦截，请点击视频恢复声音。');
        });
      }
    };
    if (video.readyState >= 1) restore();
    else video.addEventListener('loadedmetadata', restore, { once: true });
    return () => {
      cancelled = true;
    };
  }, [sourceUrl]);

  function restoreDryAudioOutput() {
    processedAudioPlayingRef.current = false;
    setProcessedAudioReady(false);
    for (const el of [processedAudioARef.current, processedAudioBRef.current]) {
      el?.pause();
    }
    const video = videoRef.current;
    if (video && !portAudioHardwareRef.current && !userMutedRef.current) {
      video.muted = false;
    }
    syncUserAudioSettings();
  }

  useEffect(() => {
    const video = videoRef.current;
    const slotA = processedAudioARef.current;
    const slotB = processedAudioBRef.current;
    if (!slotA || !slotB) return;
    if (!processedAudioUrl) {
      processedAudioUrlRef.current = null;
      for (const el of [slotA, slotB]) {
        el.pause();
        el.removeAttribute('src');
        el.load();
      }
      restoreDryAudioOutput();
      return;
    }
    if (processedAudioUrlRef.current === processedAudioUrl) return;
    let cancelled = false;
    let cutDone = false;
    // 空闲槽预载；干声/旧槽一直播到新槽真正 playing，再切（根因：play() resolve ≠ 已出声）
    const nextSlot: 0 | 1 = processedAudioPlayingRef.current
      ? (processedActiveSlotRef.current === 0 ? 1 : 0)
      : 0;
    const standby = nextSlot === 0 ? slotA : slotB;
    const active = processedActiveSlotRef.current === 0 ? slotA : slotB;
    standby.pause();
    standby.src = processedAudioUrl;
    standby.load();
    const failStandby = () => {
      if (cancelled || cutDone) return;
      if (!processedAudioPlayingRef.current) restoreDryAudioOutput();
    };
    const cutOver = () => {
      if (cancelled || cutDone) return;
      cutDone = true;
      processedActiveSlotRef.current = nextSlot;
      processedAudioUrlRef.current = processedAudioUrl;
      processedAudioPlayingRef.current = true;
      setProcessedAudioReady(true);
      syncUserAudioSettings();
      if (active !== standby) {
        window.setTimeout(() => {
          if (
            !cancelled
            && processedAudioPlayingRef.current
            && processedActiveSlotRef.current === nextSlot
          ) {
            active.pause();
          }
        }, AUDIO_PARAM_CROSSFADE_SEC * 1000);
      }
    };
    const waitAudibleThenCut = () => {
      if (cancelled) return;
      const onPlaying = () => {
        standby.removeEventListener('playing', onPlaying);
        standby.removeEventListener('timeupdate', onPlaying);
        cutOver();
      };
      standby.addEventListener('playing', onPlaying);
      standby.addEventListener('timeupdate', onPlaying);
      // 保险：仍无事件则 400ms 后切，避免永远不切
      window.setTimeout(() => {
        if (!cancelled && !cutDone && !standby.paused) cutOver();
      }, 400);
    };
    const activate = () => {
      if (cancelled) return;
      const t = video && Number.isFinite(video.currentTime) ? video.currentTime : 0;
      const target = Number.isFinite(standby.duration) && standby.duration > 0
        ? Math.min(Math.max(0, t), Math.max(0, standby.duration - 0.05))
        : Math.max(0, t);
      const startPlay = () => {
        if (cancelled) return;
        void standby.play().then(waitAudibleThenCut).catch(failStandby);
      };
      if (Math.abs((standby.currentTime || 0) - target) > 0.05) {
        const onSeeked = () => {
          standby.removeEventListener('seeked', onSeeked);
          startPlay();
        };
        standby.addEventListener('seeked', onSeeked);
        try {
          standby.currentTime = target;
        } catch {
          startPlay();
        }
      } else {
        startPlay();
      }
    };
    if (standby.readyState >= 1) activate();
    else {
      standby.addEventListener('loadedmetadata', activate, { once: true });
      standby.addEventListener('error', failStandby, { once: true });
    }
    return () => {
      cancelled = true;
    };
  }, [processedAudioUrl]);

  // 真轨失效回干声；加载/seek 中的短暂停不算失效
  useEffect(() => {
    let badSince: number | null = null;
    const timer = window.setInterval(() => {
      if (!processedAudioPlayingRef.current) {
        badSince = null;
        return;
      }
      if (snapshotRef.current?.playback_state !== 'playing') {
        badSince = null;
        return;
      }
      const active = processedActiveSlotRef.current === 0
        ? processedAudioARef.current
        : processedAudioBRef.current;
      const broken = !active || Boolean(active.error) || (active.paused && !active.seeking);
      if (!broken) {
        badSince = null;
        return;
      }
      if (badSince === null) badSince = Date.now();
      if (Date.now() - badSince >= 1_200) restoreDryAudioOutput();
    }, 300);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (snapshot?.audio_processing_status !== 'failed') return;
    // 处理失败时强制回到源视频声音，避免旧 A/B 槽仍保持静音接管状态。
    restoreDryAudioOutput();
  }, [snapshot?.audio_processing_status, snapshot?.fallback_reason]);

  useEffect(() => {
    const video = videoRef.current;
    const audio = audioRef.current;
    const interlude = snapshot?.interlude ?? null;
    if (!video || !sourceUrl) return;
    if (snapshot?.playback_state === 'stopped') {
      video.pause();
      playbackPositionSecRef.current = 0;
      video.currentTime = 0;
      audio?.pause();
      if (audio) audio.currentTime = 0;
      for (const el of [processedAudioARef.current, processedAudioBRef.current]) {
        if (!el) continue;
        el.pause();
        el.currentTime = 0;
      }
      restoreDryAudioOutput();
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      clearInterludePlayback({ releaseMs: interlude?.ducking_release_ms ?? 0, resetSchedule: true, resetIndex: true });
      return;
    }
    if (snapshot?.playback_state === 'paused' || snapshot?.playback_state === 'ready') {
      // ready：导入后只展示首帧，等主页「播放」
      video.pause();
      audio?.pause();
      processedAudioARef.current?.pause();
      processedAudioBRef.current?.pause();
      window.speechSynthesis?.pause();
      pauseInterludePlayback();
      return;
    }
    if (snapshot?.playback_state === 'playing' && video.paused) {
      window.speechSynthesis?.resume();
      void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
      const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
      if (effectiveAudioSource === 'realtime_variant' && audioUrl && audioDiagnosticsReady && audio) {
        void audio.play().catch(() => undefined);
      }
      if (processedAudioPlayingRef.current) {
        const active = processedActiveSlotRef.current === 0
          ? processedAudioARef.current
          : processedAudioBRef.current;
        if (active) {
          active.currentTime = video.currentTime;
          void active.play().catch(() => undefined);
        }
      }
      resumeInterludePlayback();
    }
  }, [audioDiagnosticsReady, audioUrl, snapshot?.interlude, snapshot?.playback_state, snapshot?.effective_audio_source, snapshot?.current_audio_source, sourceUrl, processedAudioReady]);

  useEffect(() => {
    if (!snapshot || !sourceUrl) return;
    const reference = getEffectiveAudioSource(snapshot) === 'realtime_variant'
      ? snapshot.current_audio_reference
      : null;
    setAudioUrl(reference ? convertFileSrc(reference.replace(/^file:\/\//, '')) : null);
  }, [snapshot?.current_audio_reference, snapshot?.current_audio_source, snapshot?.effective_audio_source, sourceUrl]);

  useEffect(() => {
    const video = videoRef.current;
    const audio = audioRef.current;
    if (!video) return;
    audioUrlRef.current = audioUrl;
    audioDiagnosticsReadyRef.current = audioDiagnosticsReady;
    const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
    if (!audioUrl || !audio || effectiveAudioSource !== 'realtime_variant') {
      setRealtimeAudioPlaybackState(false);
      if (audio) {
        audio.pause();
        if (!audioUrl) {
          audio.removeAttribute('src');
          audio.load();
        }
      }
      syncUserAudioSettings();
      return;
    }
    if (!audioDiagnosticsReady) {
      setRealtimeAudioPlaybackState(false);
      audio.pause();
      syncUserAudioSettings();
      return;
    }
    setRealtimeAudioPlaybackState(false);
    audio.preload = 'auto';
    audio.src = audioUrl;
    audio.load();
    audio.currentTime = Math.max(0, video.currentTime - (snapshot?.current_audio_start_at_ms ?? 0) / 1000);
    syncUserAudioSettings();
    if (!video.paused) {
      void audio.play().catch((cause) => {
        setRealtimeAudioPlaybackState(false);
        setPlaybackError(`实时音频候选暂时无法播放：${getDisplayErrorMessage(cause, '已暂时使用源音轨。')}`);
      });
    }
  }, [audioDiagnosticsReady, audioUrl, snapshot?.current_audio_start_at_ms, snapshot?.current_audio_source, snapshot?.effective_audio_source]);

  useEffect(() => {
    const interludeAudio = interludeAudioRef.current;
    const currentSnapshot = snapshotRef.current;
    if (!interludeAudio) return;
    interludeAudioUrlRef.current = interludeAudioUrl;
    if (!interludeAudioUrl) {
      interludeAudio.pause();
      interludeAudio.currentTime = 0;
      interludeAudio.removeAttribute('src');
      interludeAudio.load();
      syncUserAudioSettings();
      return;
    }
    if (!audioDiagnosticsReady) {
      interludeAudio.pause();
      syncUserAudioSettings();
      return;
    }
    interludeAudio.src = interludeAudioUrl;
    interludeAudio.currentTime = 0;
    syncUserAudioSettings();
    if (
      currentSnapshot?.playback_state === 'playing' &&
      !shouldPauseInterlude({
        playbackState: currentSnapshot.playback_state,
        fixedSpeechActive: fixedSpeechActiveRef.current,
      })
    ) {
      void interludeAudio.play().catch(() => undefined);
    }
  }, [audioDiagnosticsReady, interludeAudioUrl]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const mediaEvents = [
      'loadedmetadata',
      'durationchange',
      'timeupdate',
      'play',
      'pause',
      'volumechange',
      'ended',
      'enterpictureinpicture',
      'leavepictureinpicture',
    ];
    mediaEvents.forEach((eventName) => video.addEventListener(eventName, publishMediaState));
    publishMediaState();
    return () => mediaEvents.forEach((eventName) => video.removeEventListener(eventName, publishMediaState));
  }, [sourceUrl]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !snapshot?.pending_audio_candidate) return;
    const commitIfDue = () => {
      void invoke<PlaybackSnapshot>('commit_audio_variant_candidate_if_due', {
        request: { position_ms: Math.max(0, Math.round(video.currentTime * 1000)) },
      }).then(applyPlayerSnapshot).catch(() => undefined);
    };
    commitIfDue();
    const timer = window.setInterval(commitIfDue, 100);
    return () => window.clearInterval(timer);
  }, [snapshot?.pending_audio_candidate]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const video = videoRef.current;
      const currentSnapshot = snapshotRef.current;
      const interlude = currentSnapshot?.interlude ?? null;
      if (!video || !currentSnapshot || !audioDiagnosticsReadyRef.current) return;

      const sourceKey =
        currentSnapshot.current_video_reference ??
        currentSnapshot.source_media?.source_path ??
        sourceUrl ??
        null;
      const scheduleKey = sourceKey === null
        ? null
        : buildInterludeScheduleKey(currentSnapshot.playback_generation, sourceKey);

      if (scheduleKey !== interludeScheduleKeyRef.current) {
        interludeScheduleKeyRef.current = scheduleKey;
        nextInterludeAtMsRef.current = null;
        lastInterludeIndexRef.current = null;
        clearInterludePlayback({ resetSchedule: true, resetIndex: true });
      }

      if (
        !interlude ||
        !interlude.enabled ||
        interlude.audio_files.length === 0 ||
        currentSnapshot.playback_state === 'stopped'
      ) {
        clearInterludePlayback({ resetSchedule: true });
        return;
      }

      if (shouldPauseInterlude({
        playbackState: currentSnapshot.playback_state,
        fixedSpeechActive: fixedSpeechActiveRef.current,
      })) {
        // 视频暂停和固定话术朗读都只暂停当前插话；清理会丢失正在播放的插话，
        // 导致朗读结束后随机插话播放器无法恢复。
        pauseInterludePlayback();
        return;
      }

      if (interludeActiveRef.current && interludePausedRef.current) {
        resumeInterludePlayback();
      }

      if (interludeActiveRef.current) return;

      const currentClockMs = performance.now();
      if (nextInterludeAtMsRef.current === null) {
        nextInterludeAtMsRef.current = nextInterludeAtMs(
          currentClockMs,
          lastInterludeIndexRef.current !== null,
          interlude.interval_min_ms,
          interlude.interval_max_ms,
        );
      }
      if (currentClockMs < nextInterludeAtMsRef.current) return;
      startInterludePlayback(interlude);
    }, 250);
    return () => window.clearInterval(timer);
  }, [sourceUrl]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const video = videoRef.current;
      const currentSnapshot = snapshotRef.current;
      const source = currentSnapshot?.source_media;
      if (
        !video ||
        !currentSnapshot ||
        !source?.source_path ||
        !source.duration_ms ||
        !source.audio_sample_rate_hz ||
        !source.audio_channel_count ||
        !currentSnapshot?.realtime_audio_variant_enabled ||
        currentSnapshot.playback_state !== 'playing' ||
        currentSnapshot.pending_audio_candidate ||
        currentSnapshot.worker_status === 'running' ||
        !workerAvailableRef.current
      ) {
        return;
      }

      const positionMs = Math.max(0, Math.round(video.currentTime * 1000));
      const safetyLeadMs = REALTIME_AUDIO_SAFETY_LEAD_MS;
      if (
        scheduleGenerationRef.current !== currentSnapshot.playback_generation ||
        scheduleLoopRef.current !== currentSnapshot.loop_index
      ) {
        scheduleGenerationRef.current = currentSnapshot.playback_generation;
        scheduleLoopRef.current = currentSnapshot.loop_index;
        nextSegmentStartRef.current = null;
      }
      const startAtMs = nextSegmentStartRef.current ?? positionMs + safetyLeadMs;
      if (nextSegmentStartRef.current !== null && positionMs + safetyLeadMs < startAtMs) return;
      const targetDurationMs = Math.min(5_000, source.duration_ms - startAtMs);
      if (targetDurationMs < 500 || startAtMs + targetDurationMs > source.duration_ms) return;

      const segmentId = `loop-${currentSnapshot.loop_index}-segment-${startAtMs}`;
      nextSegmentStartRef.current = startAtMs + targetDurationMs;
      void invoke<SpeechToSpeechStartResult>('start_speech_to_speech_worker', {
        request: {
          input: {
            track_id: `${segmentId}-input`,
            source_kind: 'local_file',
            audio_path_or_stream_ref: source.source_path,
            audio_sha256: null,
            start_at_ms: startAtMs,
            duration_ms: targetDurationMs,
            sample_rate_hz: source.audio_sample_rate_hz,
            channel_count: source.audio_channel_count,
          },
          context: {
            track_id: `${segmentId}-input`,
            source_kind: 'local_file',
            playback_generation: currentSnapshot.playback_generation,
            loop_index: currentSnapshot.loop_index,
            segment_id: segmentId,
            start_at_ms: startAtMs,
            sample_rate_hz: source.audio_sample_rate_hz,
            channel_count: source.audio_channel_count,
            audio_path_or_stream_ref: source.source_path,
            transcript_text: '',
            previous_variant_text: null,
            locked_fields: [],
            locked_field_values: [],
            target_duration_ms: targetDurationMs,
            max_chars: 160,
            language: 'zh-CN',
            rewrite_policy: 'keep_meaning_and_natural_delivery',
            timeout_ms: REALTIME_AUDIO_WORKER_TIMEOUT_MS,
          },
          max_sync_offset_ms: 120,
        },
      }).then((result) => applyPlayerSnapshot(result.snapshot)).catch(() => {
        nextSegmentStartRef.current = startAtMs;
      });
    }, 250);
    return () => window.clearInterval(timer);
  }, []);

  // 循环令牌跟当前画面 URL，不跟后台 processed 路径（声音-only 画面锁源片）
  const currentSourceKey = sourceUrl ?? snapshot?.source_media?.source_path ?? null;

  useEffect(() => {
    if (
      loopSourceKeyRef.current !== currentSourceKey ||
      loopGenerationRef.current !== (snapshot?.playback_generation ?? null)
    ) {
      loopSourceKeyRef.current = currentSourceKey;
      loopGenerationRef.current = snapshot?.playback_generation ?? null;
      loopSequenceRef.current = snapshot?.loop_index ?? 0;
      lastRestartTokenRef.current = null;
    } else if ((snapshot?.loop_index ?? 0) > loopSequenceRef.current) {
      loopSequenceRef.current = snapshot?.loop_index ?? loopSequenceRef.current;
    }

    if (snapshot?.playback_state === 'stopped') {
      lastRestartTokenRef.current = null;
    }
  }, [currentSourceKey, snapshot?.loop_index, snapshot?.playback_generation, snapshot?.playback_state]);

  function restartToNextLoop(video: HTMLVideoElement, restartToken: string) {
    const previousVideoReference = snapshotRef.current?.current_video_reference;
    lastRestartTokenRef.current = restartToken;
    loopSequenceRef.current += 1;
    playbackPositionSecRef.current = 0;
    // 声音-only：画面只 seek；真轨与画面同帧回 0
    video.currentTime = 0;
    if (audioRef.current) audioRef.current.currentTime = 0;
    const processed = processedAudioPlayingRef.current
      ? (processedActiveSlotRef.current === 0 ? processedAudioARef.current : processedAudioBRef.current)
      : null;
    if (processed) processed.currentTime = 0;
    suppressMediaEventRef.current = true;
    void video.play().then(() => {
      if (processed && processedAudioPlayingRef.current) {
        void processed.play().catch(() => undefined);
      }
    }).catch(() => {
      suppressMediaEventRef.current = false;
      setPlaybackError('视频已回到开头，但自动播放失败，请点击视频播放。');
    });
    const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
    if (effectiveAudioSource === 'realtime_variant' && audioRef.current) {
      setRealtimeAudioPlaybackState(false);
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
    }

    void invoke<PlaybackSnapshot>('complete_playback_loop')
      .then((nextSnapshot) => {
        if (nextSnapshot.current_video_reference === previousVideoReference) {
          applyPlayerSnapshot(nextSnapshot);
        }
      })
      .catch((cause) => {
        setPlaybackError(cause instanceof Error ? cause.message : '播放轮次同步失败，已保持本地循环。');
      });
  }

  function restartAtBoundary(event: SyntheticEvent<HTMLVideoElement>) {
    const video = event.currentTarget;
    const currentSnapshot = snapshotRef.current;
    if (!currentSnapshot || currentSnapshot.playback_state !== 'playing' || !currentSourceKey) return;
    const currentTime = Number.isFinite(video.currentTime) ? video.currentTime : 0;
    if (currentTime > 0) playbackPositionSecRef.current = currentTime;
    // ponytail: 不在 timeupdate 里 seek 真轨，频繁 seek 会卡顿；只在循环边界对齐
    const duration = Number.isFinite(video.duration) ? video.duration : 0;
    const restartToken = `${currentSourceKey}:${currentSnapshot.playback_generation}:${loopSequenceRef.current}`;
    if (
      !shouldRestartPlayback({
        restartToken,
        lastRestartToken: lastRestartTokenRef.current,
        ended: event.type === 'ended',
        currentTime,
        duration,
      })
    ) {
      return;
    }
    restartToNextLoop(video, restartToken);
  }

  const runtimeVideoStyle = runtimeVideoProcessingEnabled && runtimeParameters
    ? {
        filter: `brightness(${Math.max(0, 1 + runtimeParameters.video_brightness_percent / 100)}) contrast(${runtimeParameters.video_contrast_percent / 100}) saturate(${runtimeParameters.video_saturation_percent / 100}) hue-rotate(${runtimeParameters.video_hue_rotation_degrees}deg) blur(${runtimeParameters.video_blur_radius_px}px)`,
        transform: `translate(${runtimeParameters.video_space_x_offset_px}px, ${runtimeParameters.video_space_y_offset_px}px) scale(${runtimeParameters.video_pixel_scale_percent / 100})`,
        transformOrigin: 'center',
      }
    : {};

  const finalEffectNotice = playbackError
    ? { type: 'error' as const, message: playbackError }
    : !sourceUrl
      ? { type: 'info' as const, message: '尚未导入视频，请先回到主页导入一个受支持的视频文件。' }
      : snapshot?.playback_state === 'disabled'
        ? { type: 'warning' as const, message: '当前播放已禁用，请返回主页检查源素材或后台状态。' }
        : null;

  return (
    <Layout style={{ width: '100%', height: '100vh', minHeight: 0, overflow: 'hidden', background: '#000' }}>
      <Layout.Content style={{ width: '100%', height: '100%', minHeight: 0, padding: 0, overflow: 'hidden' }}>
        <div style={{ position: 'relative', width: '100%', height: '100%', overflow: 'hidden', background: '#000' }}>
          {finalEffectNotice ? (
            <div style={{ position: 'absolute', top: 8, left: 8, right: 8, zIndex: 1 }}>
              <Alert type={finalEffectNotice.type} showIcon message={finalEffectNotice.message} />
            </div>
          ) : null}
          {finalEffectResizeError ? (
            <div style={{ position: 'absolute', top: finalEffectNotice ? 66 : 8, left: 8, right: 8, zIndex: 1 }}>
              <Alert type="warning" showIcon message={finalEffectResizeError} />
            </div>
          ) : null}
          {sourceUrl ? (
            <>
              <video
                ref={videoRef}
                crossOrigin="anonymous"
                src={sourceUrl}
                playsInline
                preload="metadata"
                muted={
                  fixedSpeechActive ||
                  (audioDiagnosticsReady &&
                    getEffectiveAudioSource(snapshot) === 'realtime_variant' &&
                    realtimeAudioPlaying)
                    ? true
                    : userMuted
                }
                onClick={resumeAudioDiagnostics}
                onPlay={() => {
                  resumeAudioDiagnostics();
                  if (suppressMediaEventRef.current) {
                    suppressMediaEventRef.current = false;
                    return;
                  }
                  if (snapshot?.playback_state === 'paused') {
                    void invoke<PlaybackSnapshot>('resume_playback').then(applyPlayerSnapshot).catch(() => undefined);
                  } else if (snapshot?.playback_state === 'ready' || snapshot?.playback_state === 'stopped') {
                    // 导入后 ready：挡住 WebView 误触发的 play，等主页「播放」
                    suppressMediaEventRef.current = true;
                    videoRef.current?.pause();
                  }
                }}
                onPause={() => {
                  if (suppressMediaEventRef.current) {
                    suppressMediaEventRef.current = false;
                    return;
                  }
                  if (snapshot?.playback_state === 'playing') {
                    void invoke<PlaybackSnapshot>('pause_playback').then(applyPlayerSnapshot).catch(() => undefined);
                  }
                }}
                onTimeUpdate={restartAtBoundary}
                onEnded={restartAtBoundary}
                style={{
                  width: '100%',
                  height: '100%',
                  maxWidth: 'none',
                  maxHeight: 'none',
                  objectFit: 'contain',
                  display: 'block',
                  background: '#000',
                  ...runtimeVideoStyle,
                }}
                onError={() => setPlaybackError('独立播放器加载视频失败，请回到主页重新导入。')}
              />
              <audio
                ref={audioRef}
                crossOrigin="anonymous"
                src={audioUrl ?? undefined}
                preload="auto"
                muted={userMuted || fixedSpeechActive}
                onPlaying={handleRealtimeAudioPlaying}
                onEnded={handleRealtimeAudioEnded}
                onError={handleRealtimeAudioElementError}
                hidden
              />
              <audio
                ref={processedAudioARef}
                crossOrigin="anonymous"
                preload="auto"
                onError={restoreDryAudioOutput}
                hidden
              />
              <audio
                ref={processedAudioBRef}
                crossOrigin="anonymous"
                preload="auto"
                onError={restoreDryAudioOutput}
                hidden
              />
              <audio
                ref={interludeAudioRef}
                crossOrigin="anonymous"
                src={interludeAudioUrl ?? undefined}
                preload="auto"
                muted={userMuted}
                onEnded={handleInterludeEnded}
                onError={handleInterludeError}
                hidden
              />
            </>
          ) : (
            <div style={{ display: 'grid', placeItems: 'center', width: '100%', height: '100%', padding: 24, boxSizing: 'border-box' }}>
              <Alert type="info" showIcon message="请从主窗口导入视频后开始播放。" />
            </div>
          )}
        </div>
      </Layout.Content>
    </Layout>
  );
}

function DesktopApp() {
  const [probe, setProbe] = useState<MediaProbeResult | null>(null);
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [snapshotLoading, setSnapshotLoading] = useState(true);
  const [snapshotFetchError, setSnapshotFetchError] = useState<string | null>(null);
  const [playerWindowBusy, setPlayerWindowBusy] = useState(false);
  const [importVideoBusy, setImportVideoBusy] = useState(false);
  const [runtimeResourceStatus, setRuntimeResourceStatus] = useState<RuntimeResourceStatus | null>(null);
  const [playbackActionBusy, setPlaybackActionBusy] = useState<'pause' | 'resume' | 'stop' | null>(null);
  const [videoProcessingEnabled, setVideoProcessingEnabled] = useState(false);
  const [audioProcessingEnabled, setAudioProcessingEnabled] = useState(false);
  const [realtimeAudioVariantEnabled, setRealtimeAudioVariantEnabled] = useState(false);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [fixedSpeechPresets, setFixedSpeechPresets] = useState<FixedSpeechPreset[]>(() => loadFixedSpeechPresets());
  const [selectedFixedSpeechPresetId, setSelectedFixedSpeechPresetId] = useState<string | null>(null);
  const [fixedSpeechPresetTitle, setFixedSpeechPresetTitle] = useState('');
  const [fixedSpeechText, setFixedSpeechText] = useState('');
  const [fixedSpeechFormError, setFixedSpeechFormError] = useState<string | null>(null);
  const [fixedSpeechState, setFixedSpeechState] = useState<FixedSpeechViewState>({
    operationId: null,
    status: 'idle',
    error: null,
  });
  const [interludeDraft, setInterludeDraft] = useState<InterludeConfigDraft>(() => buildInterludeDraft());
  const [interludeDirty, setInterludeDirty] = useState(false);
  const [interludeSaving, setInterludeSaving] = useState(false);
  const [mediaEngineCapabilities, setMediaEngineCapabilities] = useState<MediaEngineCapabilities | null>(null);
  const [audioOutputBackend, setAudioOutputBackend] = useState<AudioOutputBackendStatus | null>(null);
  const [audioOutputDevices, setAudioOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [audioOutputBusy, setAudioOutputBusy] = useState(false);
  const [audioOutputDeviceId, setAudioOutputDeviceId] = useState<string | null>(null);
  const [audioOutputMemoryKib, setAudioOutputMemoryKib] = useState(PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB);
  const [audioOutputMemoryKibInput, setAudioOutputMemoryKibInput] = useState<number | null>(
    PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB,
  );
  const [audioOutputHostApiFilter, setAudioOutputHostApiFilter] = useState<string>('all');

  function publishAudioOutputBackend(status: AudioOutputBackendStatus) {
    setAudioOutputBackend(status);
    if (typeof status.device_index === 'number') {
      setAudioOutputDeviceId(String(status.device_index));
    }
    if (typeof status.memory_buffer_kib === 'number') {
      setAudioOutputMemoryKib(status.memory_buffer_kib);
      setAudioOutputMemoryKibInput(status.memory_buffer_kib);
    }
    playbackChannelRef.current?.postMessage({
      version: 1,
      type: 'audio-output-backend',
      preferred_portaudio: Boolean(status.preferred_portaudio),
      running: Boolean(status.running),
      selected_backend: status.selected_backend,
      channels: status.channels,
      sample_rate_hz: status.sample_rate_hz,
    } satisfies AudioOutputBackendMessage);
  }

  async function applyAudioOutputBackend(
    preferPortaudio: boolean,
    deviceId = audioOutputDeviceId,
    memoryBufferKib = audioOutputMemoryKib,
  ) {
    setAudioOutputBusy(true);
    const position_ms = Number.isFinite(mediaState?.current_time)
      ? Math.max(0, Math.round((mediaState?.current_time ?? 0) * 1000))
      : Math.max(0, Math.round(snapshot?.current_position_ms ?? 0));
    try {
      const sampleRateHz = PORTAUDIO_SAMPLE_RATE_HZ;
      const status = await invoke<AudioOutputBackendStatus>('set_audio_output_backend', {
        request: {
          prefer_portaudio: preferPortaudio,
          device_index: deviceId !== null && deviceId !== '' ? Number(deviceId) : null,
          memory_buffer_kib: memoryBufferKib,
          sample_rate_hz: sampleRateHz,
          position_ms,
        },
      });
      publishAudioOutputBackend(status);
      if (status.preferred_portaudio && status.selected_backend !== 'portaudio') {
        setError(status.reason ?? 'PortAudio 启动失败，已回退 WebView');
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '切换音频出口失败');
    } finally {
      setAudioOutputBusy(false);
    }
  }
  const [researchWorkerCapabilities, setResearchWorkerCapabilities] = useState<ResearchWorkerCapabilities | null>(null);
  const [researchStatus, setResearchStatus] = useState<ResearchStatus | null>(null);
  const [researchParams, setResearchParams] = useState<ResearchParams | null>(null);
  const [researchValidation, setResearchValidation] = useState<ParameterValidationError[]>([]);
  const [researchValidationStatus, setResearchValidationStatus] = useState<'idle' | 'valid' | 'invalid'>('idle');
  const [cacheCleanup, setCacheCleanup] = useState<CacheCleanupResult | null>(null);
  const [cacheCleanupError, setCacheCleanupError] = useState<string | null>(null);
  const [mediaProcessingBusy, setMediaProcessingBusy] = useState<MediaProcessingScope | null>(null);
  const [researchActionBusy, setResearchActionBusy] = useState(false);
  const [researchCancelBusy, setResearchCancelBusy] = useState(false);
  const [cacheCleanupBusy, setCacheCleanupBusy] = useState(false);
  const snapshotRequestRef = useRef(0);
  const snapshotPollInFlightRef = useRef(false);
  const importVideoInFlightRef = useRef(false);
  const pendingRuntimeActionRef = useRef<PendingRuntimeAction | null>(null);
  const runtimeResourceActionTokenRef = useRef(0);
  const runtimeResourcePollGenerationRef = useRef(0);
  const runtimeResourceMountedRef = useRef(true);
  const runtimeResourceBusyRef = useRef(false);
  const fixedSpeechOperationIdRef = useRef<string | null>(null);
  const fixedSpeechAckTimerRef = useRef<number | null>(null);
  const playbackActionRequestRef = useRef(0);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const pictureInPictureVideoRef = useRef<HTMLVideoElement | null>(null);
  const runtimeMessageRef = useRef<RuntimeParameterMessage | null>(null);
  const runtimeSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const audioSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const pendingAudioApplyRef = useRef<{
    params: ResearchParams;
    cycle: AudioCycleSample | null;
  } | null>(null);
  const mediaApplyInFlightRef = useRef(false);
  const researchParamsRef = useRef<ResearchParams | null>(null);
  const researchParamsMutationVersionRef = useRef(0);
  const audioProcessingEnabledRef = useRef(false);
  const videoProcessingEnabledRef = useRef(false);
  const snapshotRefHome = useRef<PlaybackSnapshot | null>(null);
  const schedulePeriodRenderRef = useRef<(params: ResearchParams) => void>(() => undefined);
  const waveformCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const spectrumCanvasRef = useRef<HTMLCanvasElement | null>(null);

  const refreshRuntimeResourceCapabilities = useCallback((
    components: readonly RuntimeResourceComponent[],
    expectedActionToken: number,
  ) => {
    if (components.includes('media')) {
      void invoke<MediaEngineCapabilities>('get_media_engine_capabilities')
        .then((capabilities) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setMediaEngineCapabilities(capabilities);
          }
        })
        .catch(() => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setMediaEngineCapabilities(null);
          }
        });
    }
  }, []);

  const applyRuntimeResourceStatus = useCallback(async (
    nextStatus: RuntimeResourceStatus,
    expectedActionToken?: number,
  ) => {
    if (!runtimeResourceMountedRef.current) return;
    if (!isCurrentRuntimeResourceAction(runtimeResourceActionTokenRef.current, expectedActionToken)) {
      return;
    }
    setRuntimeResourceStatus(nextStatus);
    runtimeResourceBusyRef.current = isRuntimeResourceBusy(nextStatus);
    const capabilityToken = runtimeResourceActionTokenRef.current;
    if (
      nextStatus.state === 'ready'
      && nextStatus.component === 'media'
    ) {
      refreshRuntimeResourceCapabilities(['media'], capabilityToken);
    }
    const pending = pendingRuntimeActionRef.current;
    const resolution = resolvePendingRuntimeAction(
      pending,
      nextStatus,
      runtimeResourceActionTokenRef.current,
      expectedActionToken,
    );
    pendingRuntimeActionRef.current = resolution.pending;
    if (!pending || !resolution.shouldResume) return;
    try {
      await pending.resume();
    } catch (cause) {
      if (runtimeResourceMountedRef.current) {
        setError(getDisplayErrorMessage(cause, '媒体能力就绪后的操作恢复失败'));
      }
    }
  }, [refreshRuntimeResourceCapabilities]);

  const ensureRuntimeResources = useCallback(async (
    component: RuntimeResourceComponent,
    resume: () => Promise<void>,
  ) => {
    const token = runtimeResourceActionTokenRef.current + 1;
    runtimeResourceActionTokenRef.current = token;
    pendingRuntimeActionRef.current = { component, resume, token };
    try {
      const currentStatus = await invoke<RuntimeResourceStatus>('get_runtime_resource_status', { component });
      if (runtimeResourceActionTokenRef.current !== token) return;
      await applyRuntimeResourceStatus(currentStatus, token);
      const currentDecision = runtimeResourceEnsureDecision(component, currentStatus);
      if (currentDecision === 'resume' || currentDecision === 'wait') return;
      if (currentDecision === 'conflict') {
        pendingRuntimeActionRef.current = null;
        throw new RuntimeResourceConflictError('另一项媒体能力操作正在执行，请稍后再试');
      }
      const installingStatus = await invoke<RuntimeResourceStatus>('install_runtime_resources', { component });
      await applyRuntimeResourceStatus(installingStatus, token);
      const installingDecision = runtimeResourceEnsureDecision(component, installingStatus);
      if (installingDecision === 'conflict') {
        pendingRuntimeActionRef.current = null;
        throw new RuntimeResourceConflictError('另一项媒体能力操作正在执行，请稍后再试');
      }
      if (installingDecision === 'install') {
        pendingRuntimeActionRef.current = null;
        throw new Error(installingStatus.error || '媒体能力准备未启动，请重试');
      }
    } catch (cause) {
      if (runtimeResourceActionTokenRef.current !== token || !runtimeResourceMountedRef.current) return;
      if (cause instanceof RuntimeResourceConflictError) throw cause;
      setRuntimeResourceStatus({
        state: 'failed',
        component,
        current_file: null,
        downloaded_bytes: 0,
        total_bytes: 0,
        bytes_per_second: 0,
        installed_bytes: 0,
        resource_root: '',
        error: getDisplayErrorMessage(cause, '媒体能力准备失败'),
      });
      throw cause;
    }
  }, [applyRuntimeResourceStatus]);

  useEffect(() => {
    runtimeResourceMountedRef.current = true;
    const actionToken = runtimeResourceActionTokenRef.current;
    void invoke<RuntimeResourceStatus>('get_runtime_resource_status', { component: 'media' })
      .then((status) => {
        return applyRuntimeResourceStatus(status, actionToken);
      })
      .catch(() => undefined);
    return () => {
      runtimeResourceMountedRef.current = false;
      runtimeResourceActionTokenRef.current += 1;
      runtimeResourcePollGenerationRef.current += 1;
      pendingRuntimeActionRef.current = null;
    };
  }, [applyRuntimeResourceStatus]);

  useEffect(() => {
    if (!runtimeResourceStatus || !shouldPollRuntimeResources(runtimeResourceStatus)) return;
    const component = runtimeResourcePollComponent(runtimeResourceStatus);
    if (!component) return;
    const pollGeneration = runtimeResourcePollGenerationRef.current + 1;
    runtimeResourcePollGenerationRef.current = pollGeneration;
    const timer = window.setTimeout(() => {
      const pending = pendingRuntimeActionRef.current;
      void invoke<RuntimeResourceStatus>('get_runtime_resource_status', { component })
        .then((status) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourcePollGenerationRef.current === pollGeneration
          ) {
            return applyRuntimeResourceStatus(status, pending?.token);
          }
          return undefined;
        })
        .catch((cause) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourcePollGenerationRef.current === pollGeneration
          ) {
            setRuntimeResourceStatus((current) => current ? { ...current } : current);
          }
        });
    }, RUNTIME_RESOURCE_POLL_INTERVAL_MS);
    return () => {
      window.clearTimeout(timer);
      if (runtimeResourcePollGenerationRef.current === pollGeneration) {
        runtimeResourcePollGenerationRef.current += 1;
      }
    };
  }, [applyRuntimeResourceStatus, runtimeResourceStatus]);

  const [runtimeCycle, setRuntimeCycle] = useState(0);
  const [runtimeLastChangeMs, setRuntimeLastChangeMs] = useState<number | null>(null);
  const [audioVariationCycle, setAudioVariationCycle] = useState(0);
  const [audioLastChangeMs, setAudioLastChangeMs] = useState<number | null>(null);
  // 每个预设=完整 20 参数值；默认勾选全部；mix 默认关（单轨）；会话可持久化
  const [audioValuePresetIds, setAudioValuePresetIds] = useState<string[]>(() => {
    const saved = loadAudioMixSession();
    return saved?.selectedPresetIds?.length
      ? saved.selectedPresetIds
      : [...DEFAULT_AUDIO_VALUE_PRESET_IDS];
  });
  const [audioMixEnabled, setAudioMixEnabled] = useState(() => Boolean(loadAudioMixSession()?.mixEnabled));
  const [audioMixPickMin, setAudioMixPickMin] = useState(() => {
    const saved = loadAudioMixSession();
    return normalizeAudioMixPickMin(saved?.pickMin ?? DEFAULT_AUDIO_MIX_PICK_MIN, saved?.pickMax ?? DEFAULT_AUDIO_MIX_PICK_MAX);
  });
  const [audioMixPickMax, setAudioMixPickMax] = useState(() =>
    normalizeAudioMixPickMax(loadAudioMixSession()?.pickMax ?? DEFAULT_AUDIO_MIX_PICK_MAX),
  );
  const [audioActivePresetIds, setAudioActivePresetIds] = useState<string[]>([]);
  const [audioCycleSeed, setAudioCycleSeed] = useState<number | null>(null);
  const [audioCycleWeights, setAudioCycleWeights] = useState<number[]>([]);
  const [audioPresetDrawerOpen, setAudioPresetDrawerOpen] = useState(false);
  const audioCycleSampleRef = useRef<AudioCycleSample | null>(null);
  const lastAudioCycleSnapshotSignatureRef = useRef<string | null>(null);
  const audioValuePresetIdsRef = useRef(audioValuePresetIds);
  const audioMixEnabledRef = useRef(audioMixEnabled);
  const audioMixPickMinRef = useRef(audioMixPickMin);
  const audioMixPickMaxRef = useRef(audioMixPickMax);
  useEffect(() => {
    audioValuePresetIdsRef.current = audioValuePresetIds;
  }, [audioValuePresetIds]);
  useEffect(() => {
    audioMixEnabledRef.current = audioMixEnabled;
  }, [audioMixEnabled]);
  useEffect(() => {
    audioMixPickMinRef.current = audioMixPickMin;
  }, [audioMixPickMin]);
  useEffect(() => {
    audioMixPickMaxRef.current = audioMixPickMax;
  }, [audioMixPickMax]);
  useEffect(() => {
    saveAudioMixSession({
      selectedPresetIds: audioValuePresetIds,
      mixEnabled: audioMixEnabled,
      pickMin: audioMixPickMin,
      pickMax: audioMixPickMax,
    });
  }, [audioValuePresetIds, audioMixEnabled, audioMixPickMin, audioMixPickMax]);

  useEffect(() => {
    researchParamsRef.current = researchParams;
  }, [researchParams]);
  useEffect(() => {
    audioProcessingEnabledRef.current = audioProcessingEnabled;
  }, [audioProcessingEnabled]);
  useEffect(() => {
    videoProcessingEnabledRef.current = videoProcessingEnabled;
  }, [videoProcessingEnabled]);
  useEffect(() => {
    snapshotRefHome.current = snapshot;
  }, [snapshot]);

  function commitAudioCycleSample(
    sample: AudioCycleSample,
    options: {
      scheduleRender?: boolean;
      applyAudioToResearchParams?: boolean;
      baseParams?: ResearchParams;
      randomChangePeriodMs?: number;
    } = {},
  ) {
    const {
      scheduleRender = true,
      applyAudioToResearchParams = true,
      baseParams,
      randomChangePeriodMs,
    } = options;
    audioCycleSampleRef.current = sample;
    setAudioActivePresetIds(sample.presetIds);
    setAudioCycleSeed(sample.seed);
    setAudioCycleWeights(sample.weights);
    const snapshotSignature = JSON.stringify({
      seed: sample.seed,
      presetIds: sample.presetIds,
      weights: sample.weights,
      values: sample.values,
    });
    if (lastAudioCycleSnapshotSignatureRef.current !== snapshotSignature) {
      // 可复现：落盘 seed + preset ids + weights + 参数快照 + 提交时间。
      appendAudioCycleSnapshot({
        at: new Date().toISOString(),
        seed: sample.seed,
        presetIds: sample.presetIds,
        weights: sample.weights,
        values: { ...sample.values },
      });
      lastAudioCycleSnapshotSignatureRef.current = snapshotSignature;
    }
    if (!applyAudioToResearchParams) return sample;
    setResearchParams((current) => {
      const source = baseParams ?? current;
      if (!source) return source;
      const next = {
        ...source,
        audio: {
          ...source.audio,
          ...sample.values,
          ...(typeof randomChangePeriodMs === 'number'
            ? { random_change_period_ms: randomChangePeriodMs }
            : {}),
        },
      };
      // 首轮/周期都提交流式配置；PortAudio 候选预热后再切轨，失败沿用旧轨。
      if (scheduleRender) {
        queueMicrotask(() => schedulePeriodRenderRef.current(next));
      }
      return next;
    });
    return sample;
  }

  function sampleAndCommitAudioCycle(options: {
    scheduleRender?: boolean;
    applyAudioToResearchParams?: boolean;
    baseParams?: ResearchParams;
    randomChangePeriodMs?: number;
  } = {}) {
    return commitAudioCycleSample(
      sampleAudioCycle(audioValuePresetIdsRef.current, {
        mixEnabled: audioMixEnabledRef.current,
        pickMin: audioMixPickMinRef.current,
        pickMax: audioMixPickMaxRef.current,
        previousPresetIds: audioCycleSampleRef.current?.presetIds,
      }),
      options,
    );
  }

  function applyAudioCycleSample(scheduleRender = true) {
    sampleAndCommitAudioCycle({ scheduleRender });
  }
  const [runtimeNowMs, setRuntimeNowMs] = useState(() => Date.now());
  const [runtimePreview, setRuntimePreview] = useState<RuntimePreviewParameters | null>(null);
  const [runtimeChannelError, setRuntimeChannelError] = useState<string | null>(null);
  const [diagnosticMessage, setDiagnosticMessage] = useState<DiagnosticMessage | null>(null);
  const [mediaState, setMediaState] = useState<PlaybackMediaStateMessage | null>(null);
  const [pictureInPictureActive, setPictureInPictureActive] = useState(false);
  const pictureInPictureSourceUrl = toAssetUrl(
    snapshot?.current_video_reference ?? snapshot?.source_media?.source_path ?? probe?.source.source_path,
  );
  const runtimeResourceBusy = runtimeResourceStatus !== null && isRuntimeResourceBusy(runtimeResourceStatus);

  useLayoutEffect(() => {
    runtimeResourceBusyRef.current = runtimeResourceBusy;
  }, [runtimeResourceBusy]);

  useEffect(() => {
    if (typeof BroadcastChannel === 'undefined') {
      setRuntimeChannelError('当前桌面运行时不支持跨窗口预览通信，视频播放不受影响。');
      return;
    }
    let channel: BroadcastChannel;
    try {
      channel = new BroadcastChannel(PLAYBACK_CHANNEL_NAME);
    } catch {
      setRuntimeChannelError('预览通信通道创建失败，视频播放不受影响。');
      return;
    }
    playbackChannelRef.current = channel;
    const handleMessage = (event: MessageEvent<unknown>) => {
      if (isPlaybackMediaStateMessage(event.data)) {
        setMediaState(event.data);
        return;
      }
      if (isFixedSpeechStatusMessage(event.data)) {
        if (event.data.operation_id !== fixedSpeechOperationIdRef.current) return;
        if (fixedSpeechAckTimerRef.current !== null) {
          window.clearTimeout(fixedSpeechAckTimerRef.current);
          fixedSpeechAckTimerRef.current = null;
        }
        if (['completed', 'failed', 'cancelled'].includes(event.data.status)) {
          fixedSpeechOperationIdRef.current = null;
        }
        setFixedSpeechState({
          operationId: event.data.operation_id,
          status: event.data.status,
          error: event.data.error,
        });
        return;
      }
      if (isAudioOutputBackendClosedMessage(event.data)) {
        setAudioOutputBackend((current) =>
          current
            ? {
                ...current,
                preferred_portaudio: false,
                running: false,
                selected_backend: 'webview',
                hardware_state: 'not_created',
                callback_status_flags: 0,
                callback_status_flags_count: 0,
                callback_underrun_count: 0,
                producer_drop_count: 0,
                output_latency_ms: 0,
                actual_sample_rate_hz: null,
                callback_stalled_ms: null,
                ring_len_samples: 0,
                ring_capacity_samples: 0,
                audio_task_count: 0,
                current_audio_ffmpeg_pid: null,
                pending_audio_ffmpeg_pid: null,
              }
            : {
                available: false,
                selected_backend: 'webview',
                hardware_state: 'not_created',
                callback_status_flags: 0,
                callback_status_flags_count: 0,
                callback_underrun_count: 0,
                producer_drop_count: 0,
                output_latency_ms: 0,
                actual_sample_rate_hz: null,
                callback_stalled_ms: null,
                ring_len_samples: 0,
                ring_capacity_samples: 0,
                preferred_portaudio: false,
                running: false,
                reason: '最终效果窗已关闭',
                xrun_count: 0,
                device_index: null,
                memory_buffer_kib: PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB,
                frames_per_buffer: PORTAUDIO_DEFAULT_FRAMES_PER_BUFFER,
                sample_rate_hz: PORTAUDIO_SAMPLE_RATE_HZ,
                channels: 2,
                audio_task_count: 0,
                current_audio_ffmpeg_pid: null,
                pending_audio_ffmpeg_pid: null,
              },
        );
        return;
      }
      if (!isDiagnosticMessage(event.data)) return;
      setDiagnosticMessage(event.data);
    };
    channel.addEventListener('message', handleMessage);
    return () => {
      const operationId = fixedSpeechOperationIdRef.current;
      if (operationId) {
        channel.postMessage({
          version: 1,
          type: 'fixed-speech-command',
          action: 'cancel',
          operation_id: operationId,
        } satisfies FixedSpeechCommandMessage);
      }
      if (fixedSpeechAckTimerRef.current !== null) {
        window.clearTimeout(fixedSpeechAckTimerRef.current);
        fixedSpeechAckTimerRef.current = null;
      }
      channel.removeEventListener('message', handleMessage);
      channel.close();
      if (playbackChannelRef.current === channel) playbackChannelRef.current = null;
    };
  }, []);

  function syncPictureInPictureVideo() {
    const video = pictureInPictureVideoRef.current;
    if (!video || !mediaState) return;
    const duration = Number.isFinite(video.duration) && video.duration >= 0 ? video.duration : mediaState.duration;
    video.currentTime = clampMediaTime(mediaState.current_time, duration);
    video.volume = mediaState.volume;
    video.muted = true;
  }

  useEffect(() => {
    const video = pictureInPictureVideoRef.current;
    if (!video) return;
    const handleEnter = () => setPictureInPictureActive(true);
    const handleLeave = () => {
      setPictureInPictureActive(false);
      video.pause();
    };
    video.addEventListener('enterpictureinpicture', handleEnter);
    video.addEventListener('leavepictureinpicture', handleLeave);
    return () => {
      video.removeEventListener('enterpictureinpicture', handleEnter);
      video.removeEventListener('leavepictureinpicture', handleLeave);
    };
  }, [pictureInPictureSourceUrl]);

  useEffect(() => {
    const video = pictureInPictureVideoRef.current;
    if (!video) return;
    if (!pictureInPictureSourceUrl) {
      video.removeAttribute('src');
      video.load();
      setPictureInPictureActive(false);
      return;
    }
    if (video.src !== pictureInPictureSourceUrl) {
      video.src = pictureInPictureSourceUrl;
      video.load();
    }
    syncPictureInPictureVideo();
  }, [pictureInPictureSourceUrl, mediaState?.current_time, mediaState?.duration, mediaState?.volume]);

  useEffect(() => {
    drawDiagnosticCanvas(waveformCanvasRef.current, diagnosticMessage?.waveform ?? [], '#1677ff', 'rgba(22, 119, 255, 0.12)');
    drawDiagnosticCanvas(spectrumCanvasRef.current, diagnosticMessage?.spectrum ?? [], '#1677ff', 'rgba(22, 119, 255, 0.12)');
  }, [diagnosticMessage]);

  useEffect(() => {
    const timer = window.setInterval(() => setRuntimeNowMs(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, []);

  // 关最终效果窗会清 PortAudio preferred；主窗轮询对齐开关状态。
  useEffect(() => {
    let cancelled = false;
    let outputStatusPollInFlight = false;
    const refreshOutputStatus = () => {
      if (outputStatusPollInFlight) return;
      outputStatusPollInFlight = true;
      void invoke<AudioOutputBackendStatus>('get_audio_output_backend_status')
        .then((status) => {
          if (cancelled) return;
          setAudioOutputBackend((current) => {
            if (
              current
              && current.preferred_portaudio === status.preferred_portaudio
              && current.running === status.running
              && current.selected_backend === status.selected_backend
              && current.xrun_count === status.xrun_count
              && current.hardware_state === status.hardware_state
              && current.callback_status_flags === status.callback_status_flags
              && current.callback_status_flags_count === status.callback_status_flags_count
              && current.callback_underrun_count === status.callback_underrun_count
              && current.producer_drop_count === status.producer_drop_count
              && current.output_latency_ms === status.output_latency_ms
              && current.actual_sample_rate_hz === status.actual_sample_rate_hz
              && current.callback_stalled_ms === status.callback_stalled_ms
              && current.audio_task_count === status.audio_task_count
              && current.current_audio_ffmpeg_pid === status.current_audio_ffmpeg_pid
              && current.pending_audio_ffmpeg_pid === status.pending_audio_ffmpeg_pid
            ) {
              return current;
            }
            return status;
          });
          if (!status.preferred_portaudio) {
            playbackChannelRef.current?.postMessage({
              version: 1,
              type: 'audio-output-backend',
              preferred_portaudio: false,
              running: false,
              selected_backend: status.selected_backend,
              channels: status.channels,
              sample_rate_hz: status.sample_rate_hz,
            } satisfies AudioOutputBackendMessage);
          }
        })
        .catch(() => undefined)
        .finally(() => {
          outputStatusPollInFlight = false;
        });
    };
    refreshOutputStatus();
    const timer = window.setInterval(refreshOutputStatus, AUDIO_OUTPUT_STATUS_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const mediaWasProcessingRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const refreshSnapshot = async () => {
      if (snapshotPollInFlightRef.current) return;
      snapshotPollInFlightRef.current = true;
      const requestId = ++snapshotRequestRef.current;
      setSnapshotLoading(true);
      try {
        const nextSnapshot = await invoke<PlaybackSnapshot>('get_snapshot');
        if (cancelled || requestId !== snapshotRequestRef.current) return;
        setSnapshot(nextSnapshot);
        setSnapshotFetchError(null);
      } catch (cause) {
        if (cancelled || requestId !== snapshotRequestRef.current) return;
        setSnapshotFetchError(cause instanceof Error ? cause.message : '读取播放状态失败');
      } finally {
        snapshotPollInFlightRef.current = false;
        if (!cancelled && requestId === snapshotRequestRef.current) {
          setSnapshotLoading(false);
        }
      }
    };
    void refreshSnapshot();
    const timer = window.setInterval(() => {
      void refreshSnapshot();
    }, PLAYBACK_SNAPSHOT_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (!snapshot) return;
    setVideoProcessingEnabled(snapshot.video_processing_enabled);
    setAudioProcessingEnabled(snapshot.audio_processing_enabled);
    setRealtimeAudioVariantEnabled(snapshot.realtime_audio_variant_enabled);
  }, [snapshot?.audio_processing_enabled, snapshot?.realtime_audio_variant_enabled, snapshot?.video_processing_enabled]);

  useEffect(() => {
    if (!interludeDirty) {
      setInterludeDraft(buildInterludeDraft(snapshot?.interlude));
    }
  }, [
    interludeDirty,
    snapshot?.interlude?.directory,
    snapshot?.interlude?.ducking_attack_ms,
    snapshot?.interlude?.ducking_depth_db,
    snapshot?.interlude?.ducking_release_ms,
    snapshot?.interlude?.enabled,
    snapshot?.interlude?.interval_max_ms,
    snapshot?.interlude?.interval_min_ms,
    snapshot?.interlude?.volume_db,
  ]);

  useEffect(() => {
    let cancelled = false;
    let researchStatusTimer: number | undefined;
    const initialMutationVersion = researchParamsMutationVersionRef.current;
    const cancelIdleWork = scheduleAfterInitialPaint(() => {
      if (cancelled) return;
      const capabilityToken = runtimeResourceActionTokenRef.current;
      if (
        !runtimeResourceBusyRef.current
        && pendingRuntimeActionRef.current === null
      ) {
        refreshRuntimeResourceCapabilities(['media'], capabilityToken);
      }
      void invoke<ResearchWorkerCapabilities>('get_research_worker_capabilities')
        .then((capabilities) => {
          if (!cancelled) setResearchWorkerCapabilities(capabilities);
        })
        .catch(() => {
          if (!cancelled) setResearchWorkerCapabilities(null);
        });
      void invoke<AudioOutputDevice[]>('list_audio_output_devices')
        .then((devices) => {
          if (!cancelled) {
            setAudioOutputDevices(
              devices.filter(
                (device) => typeof device.host_api === 'string'
                  && device.host_api.trim().toLowerCase() !== 'asio',
              ),
            );
          }
        })
        .catch(() => {
          if (!cancelled) setAudioOutputDevices([]);
        });
      void invoke<ResearchStatus>('get_research_status')
        .then((status) => {
          if (!cancelled) setResearchStatus(status);
        })
        .catch(() => {
          if (!cancelled) setResearchStatus(null);
        });
      void invoke<ResearchParams>('get_default_local_research_params')
        .then((params) => {
          if (!cancelled && researchParamsMutationVersionRef.current === initialMutationVersion) {
            // 声音处理参数默认从预设抽样；采样率/码率等结构字段保留默认。
            sampleAndCommitAudioCycle({
              scheduleRender: false,
              baseParams: params,
              randomChangePeriodMs: samplePeriodMsInRange(audioPeriodRangeRef.current),
            });
          }
        })
        .catch(() => {
          if (!cancelled) setResearchParams(null);
        });
      void invoke<SpeechToSpeechWorkerCapabilities>('get_speech_to_speech_worker_capabilities')
        .then((capabilities) => {
          if (!cancelled) setWorkerCapabilities(capabilities);
        })
        .catch(() => {
          if (!cancelled) setWorkerCapabilities(null);
        });
      researchStatusTimer = window.setInterval(() => {
        void invoke<ResearchStatus>('get_research_status')
          .then((status) => {
            if (!cancelled) setResearchStatus(status);
          })
          .catch(() => undefined);
      }, 500);
    });
    return () => {
      cancelled = true;
      cancelIdleWork();
      if (researchStatusTimer !== undefined) window.clearInterval(researchStatusTimer);
    };
  }, [refreshRuntimeResourceCapabilities]);

  useEffect(() => {
    if (selectedFixedSpeechPresetId && !fixedSpeechPresets.some((preset) => preset.id === selectedFixedSpeechPresetId)) {
      setSelectedFixedSpeechPresetId(null);
    }
  }, [fixedSpeechPresets, selectedFixedSpeechPresetId]);

  const runtimeBaseParameters = useMemo<RuntimeBaseParameters | null>(() => {
    if (!researchParams) return null;
    return {
      audio_gain_db:
        researchParams.audio.input_gain_db +
        researchParams.audio.output_gain_db +
        researchParams.audio.loudness_adjustment_db,
      audio_low_eq_db: researchParams.audio.low_eq_db,
      audio_mid_eq_db: researchParams.audio.mid_eq_db,
      audio_high_eq_db: researchParams.audio.high_eq_db,
      audio_input_gain_db: researchParams.audio.input_gain_db,
      audio_output_gain_db: researchParams.audio.output_gain_db,
      audio_loudness_adjustment_db: researchParams.audio.loudness_adjustment_db,
      audio_pitch_shift_semitones: researchParams.audio.pitch_shift_semitones,
      audio_playback_speed: researchParams.audio.playback_speed,
      audio_fade_in_ms: researchParams.audio.fade_in_ms,
      audio_fade_out_ms: researchParams.audio.fade_out_ms,
      audio_reverb_wet_percent: researchParams.audio.reverb_wet_percent,
      audio_noise_reduction_percent: researchParams.audio.noise_reduction_percent,
      audio_phase_perturbation_percent: researchParams.audio.phase_perturbation_percent,
      audio_vibrato_frequency_hz: researchParams.audio.vibrato_frequency_hz,
      audio_vibrato_depth_percent: researchParams.audio.vibrato_depth_percent,
      audio_environment_noise_percent: researchParams.audio.environment_noise_percent,
      audio_environment_noise_dbfs: researchParams.audio.environment_noise_dbfs,
      audio_filter_q: researchParams.audio.filter_q,
      audio_sample_rate_hz: researchParams.audio.sample_rate_hz ?? 0,
      audio_output_bitrate_kbps: researchParams.audio.output_bitrate_kbps,
      video_brightness_percent: researchParams.video.brightness_percent,
      video_contrast_percent: researchParams.video.contrast_percent,
      video_saturation_percent: researchParams.video.saturation_percent,
      video_hue_rotation_degrees: researchParams.video.hue_rotation_degrees,
      video_blur_radius_px: researchParams.video.blur_radius_px,
      video_pixel_scale_percent: researchParams.video.pixel_scale_percent,
      video_space_x_offset_px: researchParams.video.space_x_offset_px,
      video_space_y_offset_px: researchParams.video.space_y_offset_px,
    };
  }, [
    researchParams?.audio.input_gain_db,
    researchParams?.audio.loudness_adjustment_db,
    researchParams?.audio.pitch_shift_semitones,
    researchParams?.audio.playback_speed,
    researchParams?.audio.fade_in_ms,
    researchParams?.audio.fade_out_ms,
    researchParams?.audio.reverb_wet_percent,
    researchParams?.audio.noise_reduction_percent,
    researchParams?.audio.phase_perturbation_percent,
    researchParams?.audio.vibrato_frequency_hz,
    researchParams?.audio.vibrato_depth_percent,
    researchParams?.audio.environment_noise_percent,
    researchParams?.audio.environment_noise_dbfs,
    researchParams?.audio.filter_q,
    researchParams?.audio.sample_rate_hz,
    researchParams?.audio.output_bitrate_kbps,
    researchParams?.audio.low_eq_db,
    researchParams?.audio.mid_eq_db,
    researchParams?.audio.high_eq_db,
    researchParams?.audio.output_gain_db,
    researchParams?.video.brightness_percent,
    researchParams?.video.contrast_percent,
    researchParams?.video.saturation_percent,
    researchParams?.video.hue_rotation_degrees,
    researchParams?.video.blur_radius_px,
    researchParams?.video.pixel_scale_percent,
    researchParams?.video.space_x_offset_px,
    researchParams?.video.space_y_offset_px,
  ]);
  const [audioPeriodRange, setAudioPeriodRange] = useState<PeriodRangeMs>(() => loadAudioPeriodRange());
  const [videoPeriodRange, setVideoPeriodRange] = useState<PeriodRangeMs>(() => loadVideoPeriodRange());
  const audioPeriodRangeRef = useRef(audioPeriodRange);
  const videoPeriodRangeRef = useRef(videoPeriodRange);
  const nextAudioPeriodMsRef = useRef(samplePeriodMsInRange(audioPeriodRange));
  const nextVideoPeriodMsRef = useRef(samplePeriodMsInRange(videoPeriodRange));
  const [audioPeriodMs, setAudioPeriodMs] = useState(() => nextAudioPeriodMsRef.current);
  const [videoPeriodMs, setVideoPeriodMs] = useState(() => nextVideoPeriodMsRef.current);
  const playbackActive = snapshot?.playback_state?.toLowerCase() === 'playing';
  // 暂停/停止时冻结音视频周期变化，避免画面停了参数还在跳。
  const runtimeActive = playbackActive && videoProcessingEnabled;
  const audioPeriodActive = playbackActive && audioProcessingEnabled && Boolean(researchParams);

  useEffect(() => {
    audioPeriodRangeRef.current = audioPeriodRange;
    saveAudioPeriodRange(audioPeriodRange);
  }, [audioPeriodRange]);
  useEffect(() => {
    videoPeriodRangeRef.current = videoPeriodRange;
    saveVideoPeriodRange(videoPeriodRange);
  }, [videoPeriodRange]);

  useEffect(() => {
    if (!runtimeActive || !runtimeBaseParameters) {
      setRuntimePreview(null);
      if (!runtimeActive) {
        runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
        setRuntimeCycle(0);
        setRuntimeLastChangeMs(null);
      }
      return;
    }

    const initialNowMs = Date.now();
    const scheduler = runtimeSchedulerRef.current;
    if (scheduler.cycle === 0 || scheduler.lastChangeMs === null) {
      runtimeSchedulerRef.current = { cycle: 1, lastChangeMs: initialNowMs };
      setRuntimeCycle(1);
      setRuntimeLastChangeMs(initialNowMs);
      setResearchParams((current) => {
        if (!current) return current;
        // 只微扰已映射视频字段；research 实验参数不进 FFmpeg
        return {
          ...current,
          video: { ...current.video, ...sampleSubtleVideoParams() },
        };
      });
    }
    setRuntimePreview(buildRuntimePreviewParameters(runtimeBaseParameters, runtimeSchedulerRef.current.cycle));

    const timer = window.setInterval(() => {
      const nowMs = Date.now();
      const current = runtimeSchedulerRef.current;
      if (current.lastChangeMs === null || !isRuntimeVariationDue(nowMs, current.lastChangeMs, nextVideoPeriodMsRef.current)) return;
      current.cycle += 1;
      current.lastChangeMs = nowMs;
      const nextPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
      nextVideoPeriodMsRef.current = nextPeriod;
      setVideoPeriodMs(nextPeriod);
      setRuntimeCycle(current.cycle);
      setRuntimeLastChangeMs(nowMs);
      // ponytail: 周期到只重采样已映射视频字段
      setResearchParams((params) => {
        if (!params) return params;
        return {
          ...params,
          video: { ...params.video, ...sampleSubtleVideoParams() },
        };
      });
    }, 100);
    return () => window.clearInterval(timer);
  }, [runtimeActive, runtimeBaseParameters, snapshot?.playback_state]);

  // 声音开启：立刻采样；周期到再提交新的 FFmpeg 流式配置，候选未就绪沿用当前出口。
  useEffect(() => {
    if (!audioPeriodActive) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      pendingAudioApplyRef.current = null;
      setAudioVariationCycle(0);
      setAudioLastChangeMs(null);
      return;
    }

    if (audioSchedulerRef.current.cycle === 0 || audioSchedulerRef.current.lastChangeMs === null) {
      const nowMs = Date.now();
      const firstPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      nextAudioPeriodMsRef.current = firstPeriod;
      setAudioPeriodMs(firstPeriod);
      audioSchedulerRef.current = { cycle: 1, lastChangeMs: nowMs };
      setAudioVariationCycle(1);
      setAudioLastChangeMs(nowMs);
      applyAudioCycleSample();
    }

    let cancelled = false;
    const timer = window.setInterval(() => {
      if (cancelled) return;
      const nowMs = Date.now();
      const scheduler = audioSchedulerRef.current;
      if (scheduler.lastChangeMs === null) {
        scheduler.lastChangeMs = nowMs;
        setAudioLastChangeMs(nowMs);
        return;
      }
      if (!isRuntimeVariationDue(nowMs, scheduler.lastChangeMs, nextAudioPeriodMsRef.current)) return;
      scheduler.cycle += 1;
      scheduler.lastChangeMs = nowMs;
      const nextPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      nextAudioPeriodMsRef.current = nextPeriod;
      setAudioPeriodMs(nextPeriod);
      setAudioVariationCycle(scheduler.cycle);
      setAudioLastChangeMs(nowMs);
      applyAudioCycleSample(true);
    }, 100);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [audioPeriodActive]);

  // 仅播放中才推送实时参数；暂停时冻结播放窗效果。
  useEffect(() => {
    const liveAudio = playbackActive && audioProcessingEnabled;
    const liveVideo = runtimeActive;
    const audioPayload = runtimeBaseParameters && (liveAudio || liveVideo)
      ? {
          ...runtimeBaseParameters,
          ...(liveVideo && runtimePreview
            ? {
                video_brightness_percent: runtimePreview.video_brightness_percent,
                video_contrast_percent: runtimePreview.video_contrast_percent,
                video_saturation_percent: runtimePreview.video_saturation_percent,
                video_hue_rotation_degrees: runtimePreview.video_hue_rotation_degrees,
                video_blur_radius_px: runtimePreview.video_blur_radius_px,
                video_pixel_scale_percent: runtimePreview.video_pixel_scale_percent,
                video_space_x_offset_px: runtimePreview.video_space_x_offset_px,
                video_space_y_offset_px: runtimePreview.video_space_y_offset_px,
              }
            : {}),
        }
      : null;
    runtimeMessageRef.current = {
      version: 1,
      type: 'runtime-parameters',
      payload: audioPayload,
      audio_processing_enabled: liveAudio,
      video_processing_enabled: liveVideo,
      playback_generation: snapshot?.playback_generation ?? null,
    };
    playbackChannelRef.current?.postMessage(runtimeMessageRef.current);
  }, [
    audioProcessingEnabled,
    playbackActive,
    runtimeActive,
    runtimeBaseParameters,
    runtimePreview,
    snapshot?.playback_generation,
    snapshot?.playback_state,
    videoProcessingEnabled,
  ]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const message = runtimeMessageRef.current;
      if (message) playbackChannelRef.current?.postMessage(message);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

  function updateResearchParam(section: 'audio' | 'video' | 'research', field: string, value: number | null) {
    if (value === null || !researchParams) return;
    researchParamsMutationVersionRef.current += 1;
    setResearchParams({
      ...researchParams,
      [section]: { ...researchParams[section], [field]: value },
    });
    setResearchValidationStatus('idle');
  }

  async function validateResearchParams() {
    if (!researchParams) return;
    try {
      const result = await invoke<{ valid: boolean; errors: ParameterValidationError[] }>(
        'validate_local_research_params',
        { request: researchParams },
      );
      setResearchValidation(result.errors);
      setResearchValidationStatus(result.valid ? 'valid' : 'invalid');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '研究参数校验失败');
    }
  }

  async function resetResearchParams() {
    const mutationVersion = ++researchParamsMutationVersionRef.current;
    try {
      const defaults = await invoke<ResearchParams>('get_default_local_research_params');
      if (mutationVersion !== researchParamsMutationVersionRef.current) return;
      sampleAndCommitAudioCycle({ scheduleRender: false, baseParams: defaults });
      setResearchValidation([]);
      setResearchValidationStatus('idle');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '恢复研究参数默认值失败');
    }
  }

  function rerollSubtleAudioParams() {
    if (!researchParams) return;
    const nowMs = Date.now();
    audioSchedulerRef.current = {
      cycle: Math.max(1, audioSchedulerRef.current.cycle + 1),
      lastChangeMs: nowMs,
    };
    setAudioVariationCycle(audioSchedulerRef.current.cycle);
    setAudioLastChangeMs(nowMs);
    applyAudioCycleSample(true);
    setResearchValidationStatus('idle');
  }

  const currentSource = snapshot?.source_media ?? probe?.source ?? null;
  const normalizedPlaybackState = snapshot?.playback_state?.toLowerCase() ?? '';
  const playbackDisplayState: PlaybackDisplayState =
    snapshotFetchError
      ? 'error'
      : snapshotLoading && !snapshot
        ? 'loading'
        : !currentSource
          ? 'no-source'
          : normalizedPlaybackState === 'disabled'
            ? 'disabled'
            : normalizedPlaybackState === 'ready' ||
                normalizedPlaybackState === 'playing' ||
                normalizedPlaybackState === 'paused' ||
                normalizedPlaybackState === 'stopped'
              ? (normalizedPlaybackState as PlaybackDisplayState)
              : 'unknown';
  const playbackNotice =
    playbackDisplayState === 'error'
      ? snapshotFetchError ?? '播放状态读取失败'
      : playbackDisplayState === 'loading'
        ? '正在读取播放状态…'
        : playbackDisplayState === 'no-source'
          ? '尚未导入视频，请先导入一个受支持的视频文件。'
          : playbackDisplayState === 'disabled'
            ? '当前播放已禁用，请返回主页检查源素材或后台状态。'
            : playbackDisplayState === 'playing'
              ? '当前视频正在循环播放。'
              : playbackDisplayState === 'paused'
                ? '当前视频已暂停。'
                : playbackDisplayState === 'stopped'
                  ? '当前视频已停止。'
                  : '播放状态已就绪。';
  const playbackNoticeType =
    playbackDisplayState === 'error'
      ? 'error'
      : playbackDisplayState === 'loading' || playbackDisplayState === 'no-source'
        ? 'info'
        : playbackDisplayState === 'disabled'
          ? 'warning'
          : playbackDisplayState === 'playing'
            ? 'success'
            : 'info';
  const canPause = playbackDisplayState === 'playing';
  const canResume = playbackDisplayState === 'paused';
  // ponytail: 有源即可反复点播放（再开窗+开播）
  const canStartPlayback = Boolean(currentSource);
  const canStop = ['ready', 'playing', 'paused'].includes(playbackDisplayState);
  const fixedSpeechTextCount = countUnicodeCharacters(fixedSpeechText);
  const trimmedFixedSpeechText = fixedSpeechText.trim();
  const trimmedFixedSpeechPresetTitle = fixedSpeechPresetTitle.trim();
  const selectedFixedSpeechPreset =
    selectedFixedSpeechPresetId === null
      ? null
      : fixedSpeechPresets.find((preset) => preset.id === selectedFixedSpeechPresetId) ?? null;
  const fixedSpeechBusy = fixedSpeechState.status === 'starting' || fixedSpeechState.status === 'playing';
  const fixedSpeechPlayDisabledReason =
    !currentSource
      ? '请先导入视频'
      : playbackDisplayState !== 'playing'
        ? '请先开始播放视频'
        : runtimeChannelError
          ? '最终效果窗口通信不可用'
          : fixedSpeechBusy
            ? '固定话术正在朗读'
            : getFixedSpeechTextError(fixedSpeechText);
  const fixedSpeechSaveDisabledReason =
    selectedFixedSpeechPreset
      ? getFixedSpeechPresetError(fixedSpeechPresetTitle, fixedSpeechText)
      : fixedSpeechPresets.length >= 10
        ? '最多保存 10 条预制文本'
        : getFixedSpeechPresetError(fixedSpeechPresetTitle, fixedSpeechText);
  const fixedSpeechStatusLabel = {
    idle: '未开始',
    starting: '准备中',
    playing: '朗读中',
    completed: '已完成',
    failed: '失败',
    cancelled: '已取消',
  }[fixedSpeechState.status];
  const fixedSpeechNotice =
    fixedSpeechState.status === 'playing'
      ? { type: 'success' as const, message: '系统语音正在朗读；最终效果原声已临时静音。' }
      : fixedSpeechState.status === 'starting'
        ? { type: 'info' as const, message: '正在调用本机系统语音。' }
        : fixedSpeechState.status === 'failed'
          ? { type: 'error' as const, message: fixedSpeechState.error ?? '系统语音朗读失败。' }
          : fixedSpeechState.status === 'cancelled'
            ? { type: 'warning' as const, message: '系统语音朗读已取消，原声已恢复。' }
            : fixedSpeechState.status === 'completed'
              ? { type: 'success' as const, message: '朗读完成，原声已恢复。' }
              : { type: 'info' as const, message: '输入 1–500 字后，由本机系统语音直接朗读，不下载模型、不识别人声。' };
  function applyFixedSpeechPresetSelection(presetId: string | null) {
    setSelectedFixedSpeechPresetId(presetId);
    if (!presetId) return;
    const preset = fixedSpeechPresets.find((item) => item.id === presetId);
    if (!preset) return;
    setFixedSpeechPresetTitle(preset.title);
    setFixedSpeechText(preset.text);
    setFixedSpeechFormError(null);
  }

  function playCurrentFixedSpeechText() {
    const validationError = getFixedSpeechTextError(fixedSpeechText);
    if (validationError) {
      setFixedSpeechFormError(validationError);
      return;
    }
    const channel = playbackChannelRef.current;
    if (!channel) {
      setFixedSpeechFormError('最终效果窗口通信尚未建立，请重新打开播放器');
      return;
    }
    const operationId = typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : `fixed-speech-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    fixedSpeechOperationIdRef.current = operationId;
    setFixedSpeechFormError(null);
    setError(null);
    setFixedSpeechState({ operationId, status: 'starting', error: null });
    try {
      channel.postMessage({
        version: 1,
        type: 'fixed-speech-command',
        action: 'speak',
        operation_id: operationId,
        text: trimmedFixedSpeechText,
      } satisfies FixedSpeechCommandMessage);
    } catch {
      fixedSpeechOperationIdRef.current = null;
      setFixedSpeechState({ operationId, status: 'failed', error: '发送朗读指令失败，请重新打开播放器' });
      return;
    }
    if (fixedSpeechAckTimerRef.current !== null) window.clearTimeout(fixedSpeechAckTimerRef.current);
    fixedSpeechAckTimerRef.current = window.setTimeout(() => {
      if (fixedSpeechOperationIdRef.current !== operationId) return;
      fixedSpeechOperationIdRef.current = null;
      fixedSpeechAckTimerRef.current = null;
      setFixedSpeechState({ operationId, status: 'failed', error: '最终效果窗口未响应，请重新打开播放器' });
    }, FIXED_SPEECH_ACK_TIMEOUT_MS);
  }

  function cancelCurrentFixedSpeech() {
    const operationId = fixedSpeechOperationIdRef.current;
    if (!operationId) return;
    try {
      playbackChannelRef.current?.postMessage({
        version: 1,
        type: 'fixed-speech-command',
        action: 'cancel',
        operation_id: operationId,
      } satisfies FixedSpeechCommandMessage);
    } catch {
      // 即使通道已关闭，也要立即收口主窗口状态。
    }
    if (fixedSpeechAckTimerRef.current !== null) window.clearTimeout(fixedSpeechAckTimerRef.current);
    fixedSpeechAckTimerRef.current = null;
    fixedSpeechOperationIdRef.current = null;
    setFixedSpeechState({ operationId, status: 'cancelled', error: null });
  }

  function saveFixedSpeechPresetFromForm() {
    const validationError = getFixedSpeechPresetError(fixedSpeechPresetTitle, fixedSpeechText);
    if (validationError) {
      setFixedSpeechFormError(validationError);
      return;
    }
    try {
      const nextPresets = selectedFixedSpeechPreset
        ? updateFixedSpeechPreset(window.localStorage, fixedSpeechPresets, {
            id: selectedFixedSpeechPreset.id,
            title: trimmedFixedSpeechPresetTitle,
            text: trimmedFixedSpeechText,
          })
        : addFixedSpeechPreset(window.localStorage, fixedSpeechPresets, {
            title: trimmedFixedSpeechPresetTitle,
            text: trimmedFixedSpeechText,
          });
      setFixedSpeechPresets(nextPresets);
      const activePreset =
        selectedFixedSpeechPreset
          ? nextPresets.find((preset) => preset.id === selectedFixedSpeechPreset.id) ?? null
          : nextPresets[nextPresets.length - 1] ?? null;
      setSelectedFixedSpeechPresetId(activePreset?.id ?? null);
      setFixedSpeechPresetTitle(activePreset?.title ?? trimmedFixedSpeechPresetTitle);
      setFixedSpeechText(activePreset?.text ?? trimmedFixedSpeechText);
      setFixedSpeechFormError(null);
    } catch (cause) {
      setFixedSpeechFormError(getDisplayErrorMessage(cause, '添加或更新文案失败'));
    }
  }

  function deleteSelectedFixedSpeechPreset() {
    if (!selectedFixedSpeechPreset) return;
    try {
      const nextPresets = removeFixedSpeechPreset(window.localStorage, fixedSpeechPresets, selectedFixedSpeechPreset.id);
      setFixedSpeechPresets(nextPresets);
      setSelectedFixedSpeechPresetId(null);
      setFixedSpeechPresetTitle('');
      setFixedSpeechText('');
      setFixedSpeechFormError(null);
    } catch (cause) {
      setFixedSpeechFormError(getDisplayErrorMessage(cause, '删除预制文本失败'));
    }
  }

  async function openFinalEffectWindowFromHome(
    sourceOverride?: MediaProbeResult['source'] | null,
  ): Promise<boolean> {
    setPlayerWindowBusy(true);
    setError(null);
    try {
      const source = sourceOverride ?? snapshot?.source_media ?? probe?.source ?? null;
      const request =
        typeof source?.width === 'number'
        && typeof source?.height === 'number'
        && Number.isSafeInteger(source.width)
        && Number.isSafeInteger(source.height)
        && source.width > 0
        && source.height > 0
          ? { width: source.width, height: source.height }
          : null;
      await invoke('open_final_effect_window', request ? { request } : {});
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '打开独立播放器失败');
      return false;
    } finally {
      setPlayerWindowBusy(false);
    }
  }

  function postPlaybackMediaControl(message: PlaybackMediaControlMessage) {
    const channel = playbackChannelRef.current;
    if (!channel) {
      setError('播放器尚未连接，当前媒体控制不可用。');
      return;
    }
    try {
      channel.postMessage(message);
    } catch {
      setError('播放器控制通道暂不可用，请重新打开播放器。');
    }
  }

  async function togglePictureInPicture() {
    const video = pictureInPictureVideoRef.current as PictureInPictureVideo | null;
    const pipDocument = document as PictureInPictureDocument;
    if (!video) {
      setError('播放器尚未准备好，画中画暂不可用。');
      return;
    }
    try {
      if (pipDocument.pictureInPictureElement === video) {
        if (typeof pipDocument.exitPictureInPicture !== 'function') {
          setError('当前桌面运行时无法退出画中画。');
          return;
        }
        await pipDocument.exitPictureInPicture();
        video.pause();
        return;
      }
      if (pipDocument.pictureInPictureEnabled !== true || typeof video.requestPictureInPicture !== 'function') {
        setError('当前桌面运行时不支持画中画。');
        return;
      }
      syncPictureInPictureVideo();
      const pictureInPictureRequest = video.requestPictureInPicture();
      if (!mediaState?.paused) void video.play().catch(() => undefined);
      await pictureInPictureRequest;
    } catch {
      video.pause();
      setPictureInPictureActive(false);
      setError('画中画操作失败，视频播放不受影响。');
    }
  }

  async function runPlaybackAction(
    action: 'pause' | 'resume' | 'stop',
    command: 'pause_playback' | 'resume_playback' | 'stop_playback' | 'start_playback',
  ) {
    const requestId = ++playbackActionRequestRef.current;
    setPlaybackActionBusy(action);
    setError(null);
    try {
      const nextSnapshot = await invoke<PlaybackSnapshot>(command);
      if (requestId !== playbackActionRequestRef.current) return;
      setSnapshot(nextSnapshot);
      playbackChannelRef.current?.postMessage({ version: 1, type: 'playback-control', action } satisfies PlaybackControlMessage);
    } catch (cause) {
      if (requestId !== playbackActionRequestRef.current) return;
      setError(cause instanceof Error ? cause.message : '更新播放状态失败');
    } finally {
      if (requestId === playbackActionRequestRef.current) setPlaybackActionBusy(null);
    }
  }

  async function startPlaybackFromHome() {
    // ponytail: 点播放才开窗+开播；导入只探测
    const opened = await openFinalEffectWindowFromHome();
    if (!opened) return;
    await runPlaybackAction('resume', 'start_playback');
  }

  async function updateProcessingSwitches(next: {
    video_processing_enabled: boolean;
    audio_processing_enabled: boolean;
    realtime_audio_variant_enabled: boolean;
  }) {
    researchParamsMutationVersionRef.current += 1;
    setVideoProcessingEnabled(next.video_processing_enabled);
    setAudioProcessingEnabled(next.audio_processing_enabled);
    setRealtimeAudioVariantEnabled(next.realtime_audio_variant_enabled);
    // 打开声音/视频：立刻抽样；不自动 FFmpeg。
    if (researchParams && (next.audio_processing_enabled || next.video_processing_enabled)) {
      const nowMs = Date.now();
      const audioSample = next.audio_processing_enabled
        ? sampleAndCommitAudioCycle({
            scheduleRender: false,
            applyAudioToResearchParams: false,
          })
        : null;
      setResearchParams((current) => {
        const base = current ?? researchParams;
        return {
          ...base,
          audio: audioSample
            ? { ...base.audio, ...audioSample.values }
            : base.audio,
          video: next.video_processing_enabled
            ? { ...base.video, ...sampleSubtleVideoParams() }
            : base.video,
          research: base.research,
        };
      });
      if (next.audio_processing_enabled) {
        audioSchedulerRef.current = { cycle: 1, lastChangeMs: nowMs };
        setAudioVariationCycle(1);
        setAudioLastChangeMs(nowMs);
        pendingAudioApplyRef.current = null;
      }
      if (next.video_processing_enabled) {
        runtimeSchedulerRef.current = { cycle: 1, lastChangeMs: nowMs };
        setRuntimeCycle(1);
        setRuntimeLastChangeMs(nowMs);
      }
    }
    if (!next.audio_processing_enabled) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      pendingAudioApplyRef.current = null;
      setAudioVariationCycle(0);
      setAudioLastChangeMs(null);
    }
    if (!next.video_processing_enabled) {
      runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setRuntimeCycle(0);
      setRuntimeLastChangeMs(null);
    }
    try {
      const nextSnapshot = await invoke<PlaybackSnapshot>('set_processing_switches', {
        request: next,
      });
      setSnapshot(nextSnapshot);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '更新处理开关失败');
    }
  }

  function schedulePeriodMediaRender(params: ResearchParams) {
    if (!audioProcessingEnabledRef.current) return;
    if (!snapshotRefHome.current?.source_media) return;
    const scope: MediaProcessingScope = videoProcessingEnabledRef.current ? 'both' : 'audio';
    void applyMediaProcessing(scope, params);
  }
  schedulePeriodRenderRef.current = schedulePeriodMediaRender;

  async function applyMediaProcessing(
    scope: MediaProcessingScope,
    paramsOverride?: ResearchParams,
  ) {
    const rawParams = paramsOverride ?? researchParams;
    const params = rawParams
      ? {
          ...rawParams,
          // 未映射声音字段保留原值；下面在 UI 边界显式报错，Rust 仍会再次校验。
          audio: { ...rawParams.audio },
          video: { ...rawParams.video, ...sanitizeMappedVideoSample(rawParams.video as never) },
          // research 实验参数当前 Worker 未映射；必须与 Rust Default 一致，否则范围校验/未映射检查会拒渲染
          research: {
            ...rawParams.research,
            band_weights: Object.fromEntries(
              Object.keys(rawParams.research.band_weights ?? {}).map((key) => [key, 1]),
            ),
            target_frequency_hz: null,
            core_frequency_hz: null,
            wave_intensity: 0,
            wave_level: 0,
            wave_grain_count: 20,
            dynamic_eq_threshold: 10,
            channel_offset_percent: 0,
            space_dimension: 2,
            frequency_space_x_offset_px: 0,
            frequency_space_y_offset_px: 0,
            frame_perturbation_probability_percent: 0,
            random_graphic_opacity_percent: 0,
            random_graphic_size_px: 4,
            abstract_face_count: 0,
            abstract_face_size_percent: 2,
            abstract_face_opacity_percent: 0,
            overlay_offset_px: 0,
            slice_length_ms: 5_000,
            slice_min_length_ms: 10_000,
            slice_trigger_interval_ms: 15_000,
          },
        }
      : null;
    const scopeEnabled = scope === 'video'
      ? videoProcessingEnabled && !audioProcessingEnabled
      : scope === 'audio'
        ? audioProcessingEnabled && !videoProcessingEnabled
        : videoProcessingEnabled && audioProcessingEnabled;
    if (!params || !snapshot?.source_media || !scopeEnabled) return;
    const unsupportedAudioFields = getUnsupportedAudioPresetFields(params.audio);
    if (unsupportedAudioFields.length > 0) {
      setError(
        `声音参数暂未支持：${unsupportedAudioFields
          .map((field) => UNMAPPED_AUDIO_PRESET_FIELD_LABELS[field])
          .join('、')}。请恢复为默认值后再应用。`,
      );
      return;
    }
    // 已有 FFmpeg 在跑 / IPC 在途：只排队最新参数，绝不中断当前任务。
    if (
      mediaApplyInFlightRef.current
      || snapshot.audio_processing_status === 'processing'
      || snapshot.video_processing_status === 'processing'
    ) {
      pendingAudioApplyRef.current = {
        params,
        cycle: audioCycleSampleRef.current,
      };
      return;
    }
    mediaApplyInFlightRef.current = true;
    setError(null);
    setMediaProcessingBusy(scope);
    try {
      await ensureRuntimeResources('media', async () => {
        setMediaProcessingBusy(scope);
        try {
          // 不在这里 stop worker：start 直接带上最新 params。
          const cycle = audioCycleSampleRef.current;
          const audio_variants =
            audioMixEnabledRef.current && cycle && cycle.variants.length > 1
              ? buildAudioVariantsFromCycle(params.audio, cycle)
              : undefined;
          const nextSnapshot = await invoke<PlaybackSnapshot>('start_media_processing', {
            request: { params, audio_variants },
          });
          setSnapshot(nextSnapshot);
          if (
            nextSnapshot.fallback_reason &&
            (nextSnapshot.audio_processing_status === 'failed' || nextSnapshot.video_processing_status === 'failed')
          ) {
            setError(nextSnapshot.fallback_reason);
          }
        } finally {
          setMediaProcessingBusy(null);
        }
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '启动本地媒体处理失败'));
    } finally {
      mediaApplyInFlightRef.current = false;
      setMediaProcessingBusy(null);
    }
  }

  async function flushPendingAudioApply() {
    const pending = pendingAudioApplyRef.current;
    if (!pending || !snapshot?.source_media || !audioProcessingEnabled) return;
    if (mediaApplyInFlightRef.current || mediaProcessingBusy !== null) return;
    if (snapshot.audio_processing_status === 'processing' || snapshot.video_processing_status === 'processing') return;
    pendingAudioApplyRef.current = null;
    if (pending.cycle) {
      commitAudioCycleSample(pending.cycle, { applyAudioToResearchParams: false });
    }
    const scope: MediaProcessingScope = videoProcessingEnabled ? 'both' : 'audio';
    await applyMediaProcessing(scope, pending.params);
  }

  // FFmpeg 结束后稍等再续跑排队任务，避免 Tag 一直停在 processing、听感被连续重渲打断。
  useEffect(() => {
    const processing =
      snapshot?.audio_processing_status === 'processing'
      || snapshot?.video_processing_status === 'processing';
    if (mediaWasProcessingRef.current && !processing) {
      const timer = window.setTimeout(() => {
        void flushPendingAudioApply();
      }, 2_500);
      mediaWasProcessingRef.current = false;
      return () => window.clearTimeout(timer);
    }
    mediaWasProcessingRef.current = Boolean(processing);
  }, [snapshot?.audio_processing_status, snapshot?.video_processing_status]);

  async function startResearchAnalysis(generateOutputMp4: boolean) {
    if (!researchParams || !snapshot?.source_media) return;
    setError(null);
    setResearchActionBusy(true);
    try {
      const nextStatus = await invoke<ResearchStatus>('start_research_analysis', {
        request: { params: researchParams, generate_output_mp4: generateOutputMp4 },
      });
      setResearchStatus(nextStatus);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '启动研究分析失败');
    } finally {
      setResearchActionBusy(false);
    }
  }

  async function cancelResearchAnalysis() {
    setResearchCancelBusy(true);
    try {
      setResearchStatus(await invoke<ResearchStatus>('cancel_research_analysis'));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '取消研究分析失败');
    } finally {
      setResearchCancelBusy(false);
    }
  }

  async function cleanupLocalCaches() {
    if (cacheCleanupBusy) return;
    setCacheCleanupError(null);
    setCacheCleanup(null);
    setError(null);
    setCacheCleanupBusy(true);
    try {
      setCacheCleanup(await invoke<CacheCleanupResult>('cleanup_local_caches_command'));
    } catch (cause) {
      const message = getDisplayErrorMessage(cause, '删除已生成缓存失败');
      setCacheCleanupError(message);
      setError(message);
    } finally {
      setCacheCleanupBusy(false);
    }
  }

  async function importVideo() {
    if (importVideoInFlightRef.current) return;
    importVideoInFlightRef.current = true;
    setImportVideoBusy(true);
    try {
      const selection = await open({
        multiple: false,
        filters: [{ name: '视频文件', extensions: [...SUPPORTED_VIDEO_EXTENSIONS] }],
      });
      if (typeof selection !== 'string') return;
      const selected = selection;
      setFixedSpeechFormError(null);
      setError(null);
      await ensureRuntimeResources('media', async () => {
        importVideoInFlightRef.current = true;
        setImportVideoBusy(true);
        try {
          const result = await invoke<MediaProbeResult>('probe_local_video', { request: { path: selected } });
          setProbe(result);
          const nextSnapshot = await invoke<PlaybackSnapshot>('get_snapshot');
          setSnapshot(nextSnapshot);
          setSnapshotLoading(false);
          setSnapshotFetchError(null);
        } catch (cause) {
          setError(getDisplayErrorMessage(cause, '导入视频失败'));
        } finally {
          importVideoInFlightRef.current = false;
          setImportVideoBusy(false);
        }
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '导入视频失败'));
    } finally {
      importVideoInFlightRef.current = false;
      setImportVideoBusy(false);
    }
  }

  function updateInterludeDraft(patch: Partial<InterludeConfigDraft>) {
    setInterludeDirty(true);
    setInterludeDraft((current) => ({ ...current, ...patch }));
  }

  async function chooseInterludeDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== 'string') return;
    updateInterludeDraft({ directory: selected });
  }

  async function saveInterludeConfig() {
    setInterludeSaving(true);
    setError(null);
    try {
      const nextSnapshot = await invoke<PlaybackSnapshot>('set_interlude_config', {
        request: {
          enabled: interludeDraft.enabled,
          directory: interludeDraft.directory,
          interval_min_ms: interludeDraft.intervalMinMs,
          interval_max_ms: interludeDraft.intervalMaxMs,
          volume_db: interludeDraft.volumeDb,
          ducking_depth_db: interludeDraft.duckingDepthDb,
          ducking_attack_ms: interludeDraft.duckingAttackMs,
          ducking_release_ms: interludeDraft.duckingReleaseMs,
        },
      });
      setSnapshot(nextSnapshot);
      setInterludeDraft(buildInterludeDraft(nextSnapshot.interlude));
      setInterludeDirty(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '保存插话配置失败');
    } finally {
      setInterludeSaving(false);
    }
  }

  const runtimeRemainingMs = runtimeLastChangeMs === null
    ? null
    : Math.max(0, videoPeriodMs - (runtimeNowMs - runtimeLastChangeMs));
  const audioRemainingMs = audioLastChangeMs === null
    ? null
    : Math.max(0, audioPeriodMs - (runtimeNowMs - audioLastChangeMs));
  const diagnosticFresh = diagnosticMessage !== null && Date.now() - diagnosticMessage.sent_at_ms < 1_500;
  const interludePlaybackNotice = interludeDraft.enabled
    ? playbackDisplayState !== 'playing'
      ? '当前视频已暂停或未开始，插话不会播放，请点击“继续”后再试听。'
      : !diagnosticFresh
        ? '最终效果播放器尚未连接，插话不会播放，请点击“打开/聚焦播放器”。'
        : null
    : null;
  const mediaDuration = mediaState?.duration ?? 0;
  const mediaCurrentTime = clampMediaTime(mediaState?.current_time ?? 0, mediaDuration);
  const pictureInPictureDocument = document as PictureInPictureDocument;
  const pictureInPictureSupported =
    Boolean(pictureInPictureSourceUrl) &&
    pictureInPictureDocument.pictureInPictureEnabled === true &&
    typeof (pictureInPictureVideoRef.current as PictureInPictureVideo | null)?.requestPictureInPicture === 'function';
  const videoProcessingStatus = getProcessingStatusKey(
    snapshot?.video_processing_status,
    videoProcessingEnabled,
    Boolean(videoProcessingEnabled && snapshot?.source_media && mediaEngineCapabilities?.available),
  );
  const audioProcessingStatus = getProcessingStatusKey(
    snapshot?.audio_processing_status,
    audioProcessingEnabled,
    Boolean(audioProcessingEnabled && snapshot?.source_media && mediaEngineCapabilities?.available),
  );
  const audioPreviewStatus = !audioProcessingEnabled
    ? '已关闭'
    : audioProcessingStatus === 'processing'
      ? '后台 FFmpeg 渲染中'
      : audioProcessingStatus === 'ready' || audioProcessingStatus === 'runtime'
        ? '后台缓存已就绪'
        : '实时预览中';
  const actualAudioOutputLabel = getActualAudioOutputLabel(audioOutputBackend);
  const actualAudioStreamVariantCount = getActualAudioStreamVariantCount(snapshot);
  const actualAudioMixLabel = actualAudioStreamVariantCount === null
    ? '未上报（兼容旧快照）'
    : actualAudioOutputLabel === 'PortAudio'
      ? `${actualAudioStreamVariantCount} 条支路`
      : `未进入正式输出（配置 ${actualAudioStreamVariantCount} 条）`;
  const portAudioFallback = Boolean(audioOutputBackend?.preferred_portaudio)
    && actualAudioOutputLabel === 'WebView';
  const activeAudioPresets = useMemo(
    () => audioActivePresetIds
      .map((id) => AUDIO_VALUE_PRESETS.find((preset) => preset.id === id))
      .filter((preset): preset is (typeof AUDIO_VALUE_PRESETS)[number] => Boolean(preset)),
    [audioActivePresetIds],
  );
  const audioCapabilityRows = useMemo(
    () => buildAudioCapabilityRows(
      researchParams?.audio ?? null,
      audioProcessingEnabled,
      audioProcessingStatus === 'ready' || audioProcessingStatus === 'runtime',
    ),
    [audioProcessingEnabled, audioProcessingStatus, researchParams?.audio],
  );

    return (
    <Layout className="desktop-page">
      <Layout.Content className="desktop-page-content">
        <video
          ref={pictureInPictureVideoRef}
          src={pictureInPictureSourceUrl ?? undefined}
          muted
          playsInline
          preload="auto"
          aria-hidden="true"
          onLoadedMetadata={syncPictureInPictureVideo}
          style={{ position: 'fixed', width: 1, height: 1, opacity: 0, pointerEvents: 'none' }}
        />
        <div className="desktop-workspace">
          <section className="desktop-column desktop-column-source" aria-label="视频素材与状态">
            <Typography.Title level={2}>autoLive 桌面端</Typography.Title>
            <Space wrap>
              <Button
                type="primary"
                size="large"
                loading={importVideoBusy}
                disabled={importVideoBusy || runtimeResourceBusy}
                onClick={() => void importVideo()}
              >
                导入视频
              </Button>
              <Button
                type="primary"
                size="large"
                onClick={() => void startPlaybackFromHome()}
                disabled={!canStartPlayback}
                loading={playbackActionBusy === 'resume' || playerWindowBusy}
              >
                播放
              </Button>
              <Button size="large" onClick={() => void openFinalEffectWindowFromHome()} loading={playerWindowBusy}>
                打开/聚焦播放器
              </Button>
            </Space>
            {error ? <Alert type="error" showIcon message={error} /> : null}
            <Card title="播放状态与控制">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space wrap>
                  <Tag color={getPlaybackDisplayColor(playbackDisplayState)}>{getPlaybackDisplayLabel(playbackDisplayState)}</Tag>
                  <Tag color={snapshot?.playback_state === 'playing' ? 'green' : 'default'}>
                    循环次数：{snapshot?.loop_index ?? 0}
                  </Tag>
                  <Tag>{currentSource ? currentSource.file_name : '尚未导入源素材'}</Tag>
                </Space>
                <Alert type={playbackNoticeType as 'info' | 'success' | 'warning' | 'error'} showIcon message={playbackNotice} />
                <Descriptions column={2} size="small">
                  <Descriptions.Item label="播放状态">{snapshot?.playback_state ?? '未获取'}</Descriptions.Item>
                  <Descriptions.Item label="循环次数">{snapshot?.loop_index ?? 0}</Descriptions.Item>
                  <Descriptions.Item label="当前视频">{snapshot?.current_video_reference ?? '—'}</Descriptions.Item>
                  <Descriptions.Item label="当前音轨">{snapshot ? getEffectiveAudioSource(snapshot) : '—'}</Descriptions.Item>
                  <Descriptions.Item label="视频处理">{snapshot?.video_processing_enabled ? '开启' : '关闭'}</Descriptions.Item>
                  <Descriptions.Item label="视频处理状态">{videoProcessingStatus}</Descriptions.Item>
                  <Descriptions.Item label="声音处理">{snapshot?.audio_processing_enabled ? '开启' : '关闭'}</Descriptions.Item>
                  <Descriptions.Item label="声音处理状态">{audioProcessingStatus}</Descriptions.Item>
                </Descriptions>
                <Space wrap>
                  <Button
                    onClick={() => void runPlaybackAction('pause', 'pause_playback')}
                    disabled={!canPause}
                    loading={playbackActionBusy === 'pause'}
                  >
                    暂停
                  </Button>
                  <Button
                    onClick={() => void runPlaybackAction('resume', 'resume_playback')}
                    disabled={!canResume}
                    loading={playbackActionBusy === 'resume'}
                  >
                    继续
                  </Button>
                  <Button
                    danger
                    onClick={() => void runPlaybackAction('stop', 'stop_playback')}
                    disabled={!canStop}
                    loading={playbackActionBusy === 'stop'}
                  >
                    停止
                  </Button>
                </Space>
                <Space direction="vertical" size="small" style={{ width: '100%' }}>
                  <Typography.Text>
                    播放进度：{formatMediaTime(mediaCurrentTime)} / {formatMediaTime(mediaDuration)}
                  </Typography.Text>
                  <Slider
                    aria-label="播放进度"
                    min={0}
                    max={Math.max(mediaDuration, 1)}
                    value={mediaCurrentTime}
                    onChange={(value) => {
                      if (typeof value !== 'number') return;
                      postPlaybackMediaControl({
                        version: 1,
                        type: 'playback-media-control',
                        action: 'seek',
                        current_time: value,
                      });
                    }}
                    disabled={!mediaState || mediaDuration <= 0}
                  />
                  <Space wrap>
                    <Button
                      onClick={() => postPlaybackMediaControl({ version: 1, type: 'playback-media-control', action: 'toggle-muted' })}
                      disabled={!mediaState}
                    >
                      {mediaState?.muted ? '取消静音' : '静音'}
                    </Button>
                    <Typography.Text>音量</Typography.Text>
                    <Slider
                      aria-label="音量"
                      min={0}
                      max={1}
                      step={0.01}
                      value={mediaState?.volume ?? 1}
                      onChange={(value) => {
                        if (typeof value !== 'number') return;
                        postPlaybackMediaControl({
                          version: 1,
                          type: 'playback-media-control',
                          action: 'set-volume',
                          volume: value,
                        });
                      }}
                      disabled={!mediaState}
                      style={{ width: 180 }}
                    />
                    <Button
                      onClick={() => void togglePictureInPicture()}
                      disabled={!mediaState || !pictureInPictureSupported}
                    >
                      {pictureInPictureActive ? '退出画中画' : '画中画'}
                    </Button>
                  </Space>
                </Space>
              </Space>
            </Card>
            {probe || snapshot?.source_media ? (
              <Card title="当前源素材">
                <Descriptions column={1} size="small">
                  <Descriptions.Item label="文件">
                    {snapshot?.source_media?.file_name ?? probe?.source.file_name}
                  </Descriptions.Item>
                  <Descriptions.Item label="格式">
                    {currentSource?.file_name.split('.').pop()?.toUpperCase() ?? 'VIDEO'}
                  </Descriptions.Item>
                  <Descriptions.Item label="大小">
                    {((currentSource?.file_size_bytes ?? 0) / 1024 / 1024).toFixed(1)} MB
                  </Descriptions.Item>
                  <Descriptions.Item label="时长">
                    {snapshot?.source_media?.duration_ms ?? probe?.source.duration_ms ?? '-'} ms
                  </Descriptions.Item>
                </Descriptions>
              </Card>
            ) : null}
          </section>

          <section className="desktop-column desktop-column-audio" aria-label="音频设置">
            <Card title="声音设置">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space align="center" wrap>
                  <Switch
                    aria-label="声音处理"
                    checked={audioProcessingEnabled}
                    onChange={(checked) =>
                      void updateProcessingSwitches({
                        video_processing_enabled: videoProcessingEnabled,
                        audio_processing_enabled: checked,
                        realtime_audio_variant_enabled: realtimeAudioVariantEnabled,
                      })
                    }
                  />
                  <Typography.Text>声音处理（总开关）</Typography.Text>
                  <Tag color={getProcessingStatusColor(audioProcessingStatus)}>
                    {audioProcessingStatus}
                  </Tag>
                </Space>
                <Typography.Text type="secondary">
                  总开关：开启后才做周期随机、实时预览和 FFmpeg 固化。关闭=保持源音轨，多轨合并也无效。
                </Typography.Text>
                <Space align="center">
                  <Switch
                    aria-label="实时话术幻化"
                    checked={realtimeAudioVariantEnabled}
                    onChange={(checked) =>
                      void updateProcessingSwitches({
                        video_processing_enabled: videoProcessingEnabled,
                        audio_processing_enabled: audioProcessingEnabled,
                        realtime_audio_variant_enabled: checked,
                      })
                    }
                  />
                  <Typography.Text>实时话术幻化</Typography.Text>
                </Space>
                <Alert
                  type={workerCapabilities?.available ? 'success' : 'warning'}
                  showIcon
                  message={
                    workerCapabilities?.available
                      ? `本地话术 Worker：${workerCapabilities.provider}/${workerCapabilities.model}`
                    : `本地话术 Worker 不可用：${workerCapabilities?.reason ?? '未完成能力探测'}`
                  }
                />
                <Button
                  onClick={() => void applyMediaProcessing('audio')}
                  loading={mediaProcessingBusy === 'audio'}
                  disabled={
                    !snapshot?.source_media ||
                    !researchParams ||
                    !audioProcessingEnabled ||
                    videoProcessingEnabled ||
                    mediaProcessingBusy !== null ||
                    audioProcessingStatus === 'processing' ||
                    runtimeResourceBusy
                  }
                  title={videoProcessingEnabled ? '视频和声音同时开启时，请使用“音视频同时应用”' : undefined}
                >
                  应用音频参数
                </Button>
                <Space wrap>
                  <Button size="small" onClick={rerollSubtleAudioParams} disabled={!researchParams}>
                    重新随机
                  </Button>
                  <InputNumber
                    aria-label="声音周期最小秒"
                    addonBefore="声音最小"
                    addonAfter="秒"
                    min={PERIOD_HARD_MIN_MS / 1000}
                    max={PERIOD_HARD_MAX_MS / 1000}
                    step={1}
                    value={audioPeriodRange.minMs / 1000}
                    onChange={(value) => {
                      if (typeof value !== 'number' || !Number.isFinite(value)) return;
                      setAudioPeriodRange((current) =>
                        normalizeAudioPeriodRange(Math.round(value * 1000), current.maxMs),
                      );
                    }}
                  />
                  <InputNumber
                    aria-label="声音周期最大秒"
                    addonBefore="声音最大"
                    addonAfter="秒"
                    min={PERIOD_HARD_MIN_MS / 1000}
                    max={PERIOD_HARD_MAX_MS / 1000}
                    step={1}
                    value={audioPeriodRange.maxMs / 1000}
                    onChange={(value) => {
                      if (typeof value !== 'number' || !Number.isFinite(value)) return;
                      setAudioPeriodRange((current) =>
                        normalizeAudioPeriodRange(current.minMs, Math.round(value * 1000)),
                      );
                    }}
                  />
                </Space>
                <Typography.Text type="secondary">
                  每个预设=一条处理支路。关多轨=每周期 1 套；开多轨=1～N 套后台混成单轨。声音开着：画面锁源片；真声 A/B 双缓冲（加载中不断声，就绪再切）；未好前实时预览垫底。听感验收请显式勾选「22. 明显变调与空间（手动）」；它不进入默认随机池。
                </Typography.Text>
                <Space wrap>
                  <Tag color={actualAudioOutputLabel === 'PortAudio' ? 'green' : 'blue'}>
                    实际输出：{actualAudioOutputLabel}
                  </Tag>
                  <Tag color={actualAudioStreamVariantCount === null ? 'default' : 'processing'}>
                    实际混音：{actualAudioMixLabel}
                  </Tag>
                </Space>
                {portAudioFallback ? (
                  <Alert
                    type="warning"
                    showIcon
                    message="PortAudio 回退到 WebView；多轨当前未进入正式输出。"
                  />
                ) : null}
                <Space align="center" wrap>
                  <Switch
                    aria-label="PortAudio 硬件出口"
                    checked={Boolean(audioOutputBackend?.preferred_portaudio)}
                    loading={audioOutputBusy}
                    disabled={!audioOutputBackend?.available && !audioOutputBackend?.preferred_portaudio}
                    onChange={(checked) => {
                      void applyAudioOutputBackend(checked);
                    }}
                  />
                  <Typography.Text>PortAudio 硬件出口</Typography.Text>
                  <Tag color={audioOutputBackend?.running ? 'green' : audioOutputBackend?.available ? 'processing' : 'default'}>
                    {audioOutputBackend?.selected_backend ?? 'webview'}
                    {audioOutputBackend?.running ? ' · 运行中' : ''}
                    {typeof audioOutputBackend?.xrun_count === 'number' && audioOutputBackend.xrun_count > 0
                      ? ` · xrun ${audioOutputBackend.xrun_count}`
                      : ''}
                  </Tag>
                  {audioOutputBackend ? (
                    <Typography.Text type="secondary">
                      硬件 {audioOutputBackend.hardware_state} · 回调欠载 {audioOutputBackend.callback_underrun_count}
                      {' · '}写入丢弃 {audioOutputBackend.producer_drop_count}
                      {' · '}输出延迟 {audioOutputBackend.output_latency_ms}ms
                      {' · '}实际采样率 {audioOutputBackend.actual_sample_rate_hz ?? audioOutputBackend.sample_rate_hz}Hz
                      {' · '}音频任务 {audioOutputBackend.audio_task_count}/2
                      {audioOutputBackend.current_audio_ffmpeg_pid !== null
                        ? ` · 当前 FFmpeg PID ${audioOutputBackend.current_audio_ffmpeg_pid}`
                        : ''}
                      {audioOutputBackend.pending_audio_ffmpeg_pid !== null
                        ? ` · 候选 FFmpeg PID ${audioOutputBackend.pending_audio_ffmpeg_pid}`
                        : ''}
                      {' · '}环缓 {audioOutputBackend.ring_len_samples}/{audioOutputBackend.ring_capacity_samples}
                      {getAudioRingBufferedLabel(audioOutputBackend)}
                    </Typography.Text>
                  ) : null}
                  <Button
                    size="small"
                    loading={audioOutputBusy}
                    disabled={!audioOutputBackend?.preferred_portaudio || audioOutputBusy}
                    onClick={() => {
                      setAudioOutputBusy(true);
                      void invoke<AudioOutputBackendStatus>('play_portaudio_test_tone', {
                        request: { frequency_hz: 440, duration_ms: 400, amplitude: 0.12 },
                      })
                        .then((status) => publishAudioOutputBackend(status))
                        .catch((cause) => {
                          setError(cause instanceof Error ? cause.message : 'PortAudio 测试音失败');
                        })
                        .finally(() => setAudioOutputBusy(false));
                    }}
                  >
                    测试音
                  </Button>
                </Space>
                {audioOutputBackend?.preferred_portaudio ? (
                  <Space wrap>
                    <Select
                      aria-label="PortAudio Host API"
                      style={{ minWidth: 140 }}
                      value={audioOutputHostApiFilter}
                      options={[
                        { value:'all', label:'全部 Host API' },
                        { value:'wasapi', label:'WASAPI' },
                        { value:'mme', label:'MME' },
                        { value:'dsound', label:'DirectSound' },
                        { value:'wdmks', label:'WDMKS' },
                      ]}
                      onChange={(value) => setAudioOutputHostApiFilter(String(value))}
                    />
                    <Select
                      aria-label="PortAudio 输出设备"
                      style={{ minWidth: 220 }}
                      placeholder="默认输出设备"
                      allowClear
                      value={audioOutputDeviceId ?? undefined}
                      options={audioOutputDevices
                        .filter(
                          (device) => typeof device.host_api === 'string'
                            && device.host_api.trim().toLowerCase() !== 'asio',
                        )
                        .filter((device) =>
                          audioOutputHostApiFilter === 'all'
                            ? true
                            : device.host_api === audioOutputHostApiFilter,
                        )
                        .map((device) => ({
                          value: device.id,
                          label: `${device.name} · ${device.host_api}`,
                        }))}
                      onChange={(value) => {
                        const nextId = value === undefined ? null : String(value);
                        setAudioOutputDeviceId(nextId);
                        void applyAudioOutputBackend(true, nextId, audioOutputMemoryKib);
                      }}
                    />
                    <InputNumber
                      aria-label="PortAudio 内存缓冲区大小"
                      style={{ width: 140 }}
                      min={PORTAUDIO_MIN_MEMORY_BUFFER_KIB}
                      max={PORTAUDIO_MAX_MEMORY_BUFFER_KIB}
                      step={1}
                      precision={0}
                      value={audioOutputMemoryKibInput}
                      onChange={(value) => {
                        setAudioOutputMemoryKibInput(value);
                      }}
                      onBlur={() => {
                        const value = audioOutputMemoryKibInput;
                        if (
                          typeof value !== 'number'
                          || !Number.isInteger(value)
                          || value < PORTAUDIO_MIN_MEMORY_BUFFER_KIB
                          || value > PORTAUDIO_MAX_MEMORY_BUFFER_KIB
                        ) {
                          setAudioOutputMemoryKibInput(audioOutputMemoryKib);
                          return;
                        }
                        setAudioOutputMemoryKib(value);
                        void applyAudioOutputBackend(true, audioOutputDeviceId, value);
                      }}
                    />
                    <Typography.Text type="secondary">内存缓冲 128–2048 KiB，默认 1024 KiB</Typography.Text>
                  </Space>
                ) : null}
                <Typography.Text type="secondary">
                  参考 Audacity：PortAudio I/O + 多轨混单轨；非完整 DAW。当前仅显示已启用的 Host API。
                </Typography.Text>
                {audioOutputBackend?.reason ? (
                  <Typography.Text type="secondary">{audioOutputBackend.reason}</Typography.Text>
                ) : null}
                <Space align="center" wrap>
                  <Switch
                    aria-label="多轨合并"
                    checked={audioMixEnabled}
                    disabled={!audioProcessingEnabled}
                    onChange={(checked) => setAudioMixEnabled(checked)}
                  />
                  <Typography.Text>多轨合并（声音处理的子模式）</Typography.Text>
                  <Tag color={audioProcessingEnabled && audioMixEnabled ? 'processing' : 'default'}>
                    {!audioProcessingEnabled
                      ? '需先开声音处理'
                      : audioMixEnabled
                        ? `每周期 ${audioMixPickMin}-${audioMixPickMax} 轨`
                        : '单轨'}
                  </Tag>
                </Space>
                {audioProcessingEnabled && audioMixEnabled ? (
                  <Space wrap>
                    <InputNumber
                      aria-label="最少随机轨数"
                      addonBefore="最少轨数"
                      min={1}
                      max={AUDIO_MIX_PICK_HARD_MAX}
                      value={audioMixPickMin}
                      onChange={(value) => {
                        if (typeof value !== 'number' || !Number.isFinite(value)) return;
                        const nextMax = normalizeAudioMixPickMax(audioMixPickMax);
                        const nextMin = normalizeAudioMixPickMin(value, nextMax);
                        setAudioMixPickMin(nextMin);
                        setAudioMixPickMax(Math.max(nextMin, nextMax));
                      }}
                    />
                    <InputNumber
                      aria-label="最多随机轨数"
                      addonBefore="最多轨数"
                      min={1}
                      max={AUDIO_MIX_PICK_HARD_MAX}
                      value={audioMixPickMax}
                      onChange={(value) => {
                        if (typeof value !== 'number' || !Number.isFinite(value)) return;
                        const nextMax = normalizeAudioMixPickMax(value);
                        const nextMin = normalizeAudioMixPickMin(audioMixPickMin, nextMax);
                        setAudioMixPickMax(nextMax);
                        setAudioMixPickMin(nextMin);
                      }}
                    />
                  </Space>
                ) : null}
                <Space wrap>
                  <Button
                    size="small"
                    disabled={!audioProcessingEnabled}
                    onClick={() => setAudioValuePresetIds([...DEFAULT_AUDIO_VALUE_PRESET_IDS])}
                  >
                    全选
                  </Button>
                  <Button
                    size="small"
                    disabled={!audioProcessingEnabled}
                    onClick={() => setAudioValuePresetIds([])}
                  >
                    全不选
                  </Button>
                  <Tag>已勾选 {audioValuePresetIds.length}/{AUDIO_VALUE_PRESETS.filter((preset) => preset.id !== 'p21').length}</Tag>
                  {audioProcessingEnabled && audioActivePresetIds.length > 0 ? (
                    <Button
                      type="link"
                      size="small"
                      aria-label="查看当前声音预设参数"
                      onClick={() => setAudioPresetDrawerOpen(true)}
                    >
                      本周期：{activeAudioPresets.length > 0
                        ? activeAudioPresets.map((preset) => preset.label).join('、')
                        : audioActivePresetIds.join('、')}
                    </Button>
                  ) : null}
                  {audioProcessingEnabled && audioCycleSeed !== null ? <Tag>seed {audioCycleSeed}</Tag> : null}
                  {audioProcessingEnabled && audioCycleWeights.length > 1 ? (
                    <Tag>weights {audioCycleWeights.map((w) => w.toFixed(2)).join('/')}</Tag>
                  ) : null}
                </Space>
                <Checkbox.Group
                  aria-label="声音参数值预设"
                  value={audioValuePresetIds}
                  disabled={!audioProcessingEnabled}
                  onChange={(values) => setAudioValuePresetIds(values.map(String))}
                  style={{ width: '100%' }}
                >
                  <Space wrap>
                    {AUDIO_VALUE_PRESETS.filter((preset) => preset.id !== 'p21').map((preset) => (
                      <Checkbox key={preset.id} value={preset.id}>
                        {preset.label}
                      </Checkbox>
                    ))}
                  </Space>
                </Checkbox.Group>
                {snapshot?.fallback_reason && (audioProcessingStatus === 'failed' || videoProcessingStatus === 'failed') ? (
                  <Alert type="error" showIcon message={snapshot.fallback_reason} />
                ) : null}
              </Space>
            </Card>
            <Drawer
              title={`当前声音预设${audioVariationCycle > 0 ? `（第 ${audioVariationCycle} 轮）` : ''}`}
              placement="right"
              width={520}
              open={audioPresetDrawerOpen}
              onClose={() => setAudioPresetDrawerOpen(false)}
            >
              {activeAudioPresets.length === 0 ? (
                <Typography.Text type="secondary">当前没有可展示的声音预设。</Typography.Text>
              ) : (
                <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                  <Typography.Text type="secondary">
                    以下是本周期实际参与处理的预设值；多轨时每张卡片对应一条处理支路。
                  </Typography.Text>
                  {activeAudioPresets.map((preset) => (
                    <Card key={preset.id} size="small" title={preset.label} extra={<Tag>{preset.id}</Tag>}>
                      <Descriptions column={1} size="small" bordered>
                        {AUDIO_PRESET_FIELD_DEFINITIONS.map((field) => (
                          <Descriptions.Item key={field.key} label={field.label}>
                            {formatAudioPreviewValue(preset.values[field.key], field.unit, field.digits)}
                          </Descriptions.Item>
                        ))}
                      </Descriptions>
                    </Card>
                  ))}
                </Space>
              )}
            </Drawer>
            <Card title="音频实时参数与生效状态">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space wrap>
                  <Tag color={audioProcessingEnabled ? 'green' : 'default'}>
                    声音处理：{audioPreviewStatus}
                  </Tag>
                  <Tag color="blue">
                    配置字段：{audioCapabilityRows.length} 项
                  </Tag>
                  <Tag color={audioMixEnabled ? 'processing' : 'default'}>
                    {audioMixEnabled
                      ? `多轨：本周期 ${audioActivePresetIds.length || 0} 套（后台混）`
                      : '单轨'}
                  </Tag>
                  <Tag color={audioPeriodActive ? 'processing' : 'default'}>
                    随机轮次：{audioPeriodActive ? `第 ${audioVariationCycle} 轮` : '未启动'}
                  </Tag>
                </Space>
                <div>
                  <Typography.Text type="secondary">
                    声音区间 {(audioPeriodRange.minMs / 1000).toFixed(0)}–{(audioPeriodRange.maxMs / 1000).toFixed(0)} s · 本轮 {(audioPeriodMs / 1000).toFixed(1)} s
                    {!audioPeriodActive || audioRemainingMs === null
                      ? ' · 未启动'
                      : ` · 剩余 ${(audioRemainingMs / 1000).toFixed(1)} s`}
                  </Typography.Text>
                  <Progress
                    percent={
                      !audioPeriodActive || audioRemainingMs === null || audioPeriodMs <= 0
                        ? 0
                        : Math.min(100, Math.round(((audioPeriodMs - audioRemainingMs) / audioPeriodMs) * 100))
                    }
                    status={audioPeriodActive ? 'active' : 'normal'}
                    showInfo={false}
                    size="small"
                  />
                </div>
                <Typography.Text type="secondary">
                  「配置字段」= 本周期参数表行数（固定约 22），不是轨道数。多轨时列表仍显示第一套配置；混音结果在后台缓存，听感当前是源视频+实时预览。
                </Typography.Text>
                <Descriptions column={2} size="small" bordered>
                  {audioCapabilityRows.map((row) => (
                    <Descriptions.Item key={row.key} label={row.label}>
                      <Space size={4} wrap>
                        <Typography.Text>
                          {row.value === null ? '自动/未设置' : formatAudioPreviewValue(row.value, row.unit, row.unit === 'x' ? 3 : 3)}
                        </Typography.Text>
                        <Tag color={getProcessingStatusColor(row.status)}>{getProcessingStatusLabel(row.status)}</Tag>
                      </Space>
                      <Typography.Text type="secondary" style={{ display: 'block' }}>
                        {row.description}
                      </Typography.Text>
                    </Descriptions.Item>
                  ))}
                </Descriptions>
              </Space>
            </Card>
            <Card title="随机插话播放器">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space align="center">
                  <Switch
                    aria-label="启用随机插话"
                    checked={interludeDraft.enabled}
                    onChange={(checked) => updateInterludeDraft({ enabled: checked })}
                  />
                  <Typography.Text>启用随机插话</Typography.Text>
                </Space>
                <Typography.Text type="secondary">
                  首段立即随机播放；每段结束后等待设置的随机间隔，再播放下一段。
                </Typography.Text>
                {interludePlaybackNotice ? <Alert type="warning" showIcon message={interludePlaybackNotice} /> : null}
                <Space.Compact style={{ width: '100%' }}>
                  <Input
                    readOnly
                    value={interludeDraft.directory ?? ''}
                    placeholder="请选择音频文件夹"
                  />
                  <Button onClick={() => void chooseInterludeDirectory()}>选择音频文件夹</Button>
                </Space.Compact>
                <Space wrap>
                  <InputNumber
                    aria-label="插话最小间隔"
                    addonBefore="最小间隔"
                    addonAfter="ms"
                    min={INTERLUDE_LIMITS.intervalMinMs.min}
                    max={INTERLUDE_LIMITS.intervalMinMs.max}
                    step={500}
                    value={interludeDraft.intervalMinMs}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMinMs: value })}
                  />
                  <InputNumber
                    aria-label="插话最大间隔"
                    addonBefore="最大间隔"
                    addonAfter="ms"
                    min={INTERLUDE_LIMITS.intervalMinMs.min}
                    max={INTERLUDE_LIMITS.intervalMinMs.max}
                    step={500}
                    value={interludeDraft.intervalMaxMs}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMaxMs: value })}
                  />
                  <InputNumber
                    aria-label="插话音量"
                    addonBefore="插话音量"
                    addonAfter="dB"
                    min={INTERLUDE_LIMITS.volumeDb.min}
                    max={INTERLUDE_LIMITS.volumeDb.max}
                    step={0.5}
                    value={interludeDraft.volumeDb}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ volumeDb: value })}
                  />
                  <InputNumber
                    aria-label="原声压低"
                    addonBefore="原声压低"
                    addonAfter="dB"
                    min={INTERLUDE_LIMITS.duckingDepthDb.min}
                    max={INTERLUDE_LIMITS.duckingDepthDb.max}
                    step={0.5}
                    value={interludeDraft.duckingDepthDb}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingDepthDb: value })}
                  />
                  <InputNumber
                    aria-label="插话淡入时长"
                    addonBefore="Attack"
                    addonAfter="ms"
                    min={INTERLUDE_LIMITS.duckingAttackMs.min}
                    max={INTERLUDE_LIMITS.duckingAttackMs.max}
                    step={5}
                    value={interludeDraft.duckingAttackMs}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingAttackMs: value })}
                  />
                  <InputNumber
                    aria-label="插话淡出时长"
                    addonBefore="Release"
                    addonAfter="ms"
                    min={INTERLUDE_LIMITS.duckingReleaseMs.min}
                    max={INTERLUDE_LIMITS.duckingReleaseMs.max}
                    step={10}
                    value={interludeDraft.duckingReleaseMs}
                    onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingReleaseMs: value })}
                  />
                </Space>
                <Space wrap>
                  <Tag color={snapshot?.interlude?.enabled ? 'green' : 'default'}>
                    开关：{snapshot?.interlude?.enabled ? '开启' : '关闭'}
                  </Tag>
                  <Tag>文件数：{snapshot?.interlude?.audio_count ?? 0}</Tag>
                  <Tag>状态：{snapshot?.interlude?.status ?? 'idle'}</Tag>
                </Space>
                {snapshot?.interlude?.error ? <Alert type="error" showIcon message={snapshot.interlude.error} /> : null}
                <Button type="primary" onClick={() => void saveInterludeConfig()} loading={interludeSaving}>
                  保存插话配置
                </Button>
              </Space>
            </Card>
            <Card title="固定话术播放">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space wrap>
                  <Tag color="green">本机系统语音</Tag>
                  <Tag color={fixedSpeechState.status === 'failed' ? 'red' : fixedSpeechBusy ? 'processing' : 'blue'}>
                    状态：{fixedSpeechStatusLabel}
                  </Tag>
                  <Tag>预制：{fixedSpeechPresets.length} / 10</Tag>
                </Space>
                <Alert type={fixedSpeechNotice.type} showIcon message={fixedSpeechNotice.message} />
                {fixedSpeechFormError ? <Alert type="error" showIcon message={fixedSpeechFormError} /> : null}
                <Select
                  allowClear
                  placeholder="选择一条本地预制文本"
                  value={selectedFixedSpeechPresetId ?? undefined}
                  options={fixedSpeechPresets.map((preset) => ({
                    label: preset.title,
                    value: preset.id,
                  }))}
                  onChange={(value) => applyFixedSpeechPresetSelection(typeof value === 'string' ? value : null)}
                />
                <Input
                  aria-label="固定话术预制标题"
                  placeholder="预制标题"
                  value={fixedSpeechPresetTitle}
                  maxLength={80}
                  onChange={(event) => {
                    setFixedSpeechPresetTitle(event.target.value);
                    setFixedSpeechFormError(null);
                  }}
                />
                <Input.TextArea
                  aria-label="固定话术文本"
                  value={fixedSpeechText}
                  rows={5}
                  maxLength={500}
                  placeholder="输入要由系统语音朗读的文字"
                  onChange={(event) => {
                    setFixedSpeechText(event.target.value);
                    setFixedSpeechFormError(null);
                  }}
                />
                <Space wrap style={{ width: '100%', justifyContent: 'space-between' }}>
                  <Typography.Text type={fixedSpeechTextCount > 500 ? 'danger' : undefined}>
                    文本字数：{fixedSpeechTextCount} / 500
                  </Typography.Text>
                </Space>
                <Space wrap>
                  <Button
                    type="primary"
                    onClick={playCurrentFixedSpeechText}
                    loading={fixedSpeechBusy}
                    disabled={fixedSpeechPlayDisabledReason !== null}
                    title={fixedSpeechPlayDisabledReason ?? undefined}
                  >
                    播放当前文案
                  </Button>
                  <Button onClick={saveFixedSpeechPresetFromForm} disabled={fixedSpeechSaveDisabledReason !== null}>
                    {selectedFixedSpeechPreset ? '更新文案' : '添加文案'}
                  </Button>
                  <Button danger onClick={deleteSelectedFixedSpeechPreset} disabled={!selectedFixedSpeechPreset}>
                    删除预制文本
                  </Button>
                  <Button
                    onClick={cancelCurrentFixedSpeech}
                    disabled={!fixedSpeechBusy}
                  >
                    取消
                  </Button>
                </Space>
                {fixedSpeechPlayDisabledReason ? (
                  <Typography.Text type="secondary">{fixedSpeechPlayDisabledReason}</Typography.Text>
                ) : null}
                <Typography.Text type="secondary">
                  仅选择本机语音，文字不会发送到在线语音服务；朗读期间最终效果原声静音，结束、失败或取消后自动恢复。
                </Typography.Text>
              </Space>
            </Card>
            <Card title="实时诊断">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Tag color={diagnosticFresh ? 'green' : 'orange'}>
                  {diagnosticFresh ? '已收到独立播放器采样' : '等待独立播放器采样'}
                </Tag>
                {diagnosticMessage?.error ? <Alert type="warning" showIcon message={diagnosticMessage.error} /> : null}
                <canvas ref={waveformCanvasRef} width={640} height={96} aria-label="实时波形" style={{ display: 'block', width: '100%', height: 96 }} />
                <canvas ref={spectrumCanvasRef} width={640} height={96} aria-label="实时频谱" style={{ display: 'block', width: '100%', height: 96 }} />
              </Space>
            </Card>
            <Card
              title="研究参数校验"
              extra={
                <Space>
                  <Button onClick={() => void resetResearchParams()}>恢复默认</Button>
                  <Button type="primary" onClick={() => void validateResearchParams()} disabled={!researchParams}>
                    校验参数
                  </Button>
                </Space>
              }
              >
                <Alert
                type={researchValidationStatus === 'invalid' ? 'error' : researchValidationStatus === 'valid' ? 'success' : 'info'}
                showIcon
                message={
                  researchValidationStatus === 'invalid'
                    ? `参数有 ${researchValidation.length} 项错误`
                    : researchValidationStatus === 'valid'
                      ? '参数契约校验通过（当前仅校验，未执行媒体算法）'
                      : '参数先由 Rust 校验；点击对应模块的应用按钮后，在后台生成当前源视频的预览缓存'
                }
                  description={researchValidation.slice(0, 3).map((item) => `${item.field}：${item.message}`).join('；') || undefined}
                />
                <Typography.Text type="secondary">
                  音频参数由随机微扰生成；此处只做 Rust 参数契约校验和错误展示。
                </Typography.Text>
            </Card>
          </section>

          <section className="desktop-column desktop-column-video" aria-label="视频处理与实时参数">
            <Card title="视频处理">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space align="center">
                  <Switch
                    aria-label="视频处理"
                    checked={videoProcessingEnabled}
                    onChange={(checked) =>
                      void updateProcessingSwitches({
                        video_processing_enabled: checked,
                        audio_processing_enabled: audioProcessingEnabled,
                        realtime_audio_variant_enabled: realtimeAudioVariantEnabled,
                      })
                    }
                  />
                  <Typography.Text>视频处理</Typography.Text>
                </Space>
                <Space wrap>
                  <Tag color={getProcessingStatusColor(videoProcessingStatus)}>
                    视频状态：{videoProcessingStatus}
                  </Tag>
                  <Tag>开关：{videoProcessingEnabled ? 'enabled' : 'disabled'}</Tag>
                </Space>
                <Space wrap>
                  <Button
                    onClick={() => void applyMediaProcessing('video')}
                    loading={mediaProcessingBusy === 'video'}
                    disabled={
                      !snapshot?.source_media ||
                      !researchParams ||
                      !videoProcessingEnabled ||
                      audioProcessingEnabled ||
                      mediaProcessingBusy !== null ||
                      videoProcessingStatus === 'processing' ||
                      runtimeResourceBusy
                    }
                    title={audioProcessingEnabled ? '视频和声音同时开启时，请使用“音视频同时应用”' : undefined}
                  >
                    应用视频参数
                  </Button>
                  <Button
                    type="primary"
                    onClick={() => void applyMediaProcessing('both')}
                    loading={mediaProcessingBusy === 'both'}
                    disabled={
                      !snapshot?.source_media ||
                      !researchParams ||
                      !videoProcessingEnabled ||
                      !audioProcessingEnabled ||
                      mediaProcessingBusy !== null ||
                      videoProcessingStatus === 'processing' ||
                      audioProcessingStatus === 'processing' ||
                      runtimeResourceBusy
                    }
                  >
                    音视频同时应用
                  </Button>
                  {videoProcessingEnabled && audioProcessingEnabled ? (
                    <Typography.Text type="secondary">当前同时开启视频和声音处理，请使用联合应用；两个单独按钮会保持禁用。</Typography.Text>
                  ) : null}
                  <Tag color={mediaEngineCapabilities?.available ? 'green' : 'orange'}>
                    媒体引擎：{mediaEngineCapabilities?.available ? '可用' : '不可用'}
                  </Tag>
                </Space>
                <Alert
                  type={mediaEngineCapabilities?.available ? 'success' : 'warning'}
                  showIcon
                  message={
                    mediaEngineCapabilities?.available
                      ? 'FFmpeg/FFprobe 已就绪，处理完成后立即切换到新效果（保持当前进度）'
                      : `本地媒体引擎不可用：${mediaEngineCapabilities?.reason ?? '未完成能力探测'}`
                  }
                />
              </Space>
            </Card>
            <Card title="视频实时参数预览">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space wrap>
                  <Tag color={videoProcessingEnabled ? 'green' : 'default'}>
                    视频动态：{videoProcessingEnabled ? '开启' : '关闭'}
                  </Tag>
                  <Tag>第 {runtimeCycle} 次变化</Tag>
                  <InputNumber
                    aria-label="视频周期最小秒"
                    addonBefore="视频最小"
                    addonAfter="秒"
                    min={PERIOD_HARD_MIN_MS / 1000}
                    max={PERIOD_HARD_MAX_MS / 1000}
                    step={1}
                    value={videoPeriodRange.minMs / 1000}
                    onChange={(value) => {
                      if (typeof value !== 'number' || !Number.isFinite(value)) return;
                      setVideoPeriodRange((current) =>
                        normalizeVideoPeriodRange(Math.round(value * 1000), current.maxMs),
                      );
                    }}
                  />
                  <InputNumber
                    aria-label="视频周期最大秒"
                    addonBefore="视频最大"
                    addonAfter="秒"
                    min={PERIOD_HARD_MIN_MS / 1000}
                    max={PERIOD_HARD_MAX_MS / 1000}
                    step={1}
                    value={videoPeriodRange.maxMs / 1000}
                    onChange={(value) => {
                      if (typeof value !== 'number' || !Number.isFinite(value)) return;
                      setVideoPeriodRange((current) =>
                        normalizeVideoPeriodRange(current.minMs, Math.round(value * 1000)),
                      );
                    }}
                  />
                </Space>
                <div>
                  <Typography.Text type="secondary">
                    视频区间 {(videoPeriodRange.minMs / 1000).toFixed(0)}–{(videoPeriodRange.maxMs / 1000).toFixed(0)} s · 本轮 {(videoPeriodMs / 1000).toFixed(1)} s
                    {runtimeRemainingMs === null
                      ? ' · 未启动'
                      : ` · 剩余 ${(runtimeRemainingMs / 1000).toFixed(1)} s`}
                  </Typography.Text>
                  <Progress
                    percent={
                      runtimeRemainingMs === null || videoPeriodMs <= 0
                        ? 0
                        : Math.min(100, Math.round(((videoPeriodMs - runtimeRemainingMs) / videoPeriodMs) * 100))
                    }
                    status={runtimeActive ? 'active' : 'normal'}
                    showInfo={false}
                    size="small"
                  />
                </div>
                {runtimeActive && researchParams && runtimePreview ? (
                  <Descriptions column={2} size="small" bordered>
                    <Descriptions.Item label="亮度">
                      {researchParams.video.brightness_percent.toFixed(2)} → {runtimePreview.video_brightness_percent.toFixed(2)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="对比度">
                      {researchParams.video.contrast_percent.toFixed(2)} → {runtimePreview.video_contrast_percent.toFixed(2)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="饱和度">
                      {researchParams.video.saturation_percent.toFixed(2)} → {runtimePreview.video_saturation_percent.toFixed(2)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="色相">
                      {researchParams.video.hue_rotation_degrees.toFixed(2)} → {runtimePreview.video_hue_rotation_degrees.toFixed(2)}°
                    </Descriptions.Item>
                    <Descriptions.Item label="模糊">
                      {researchParams.video.blur_radius_px.toFixed(2)} → {runtimePreview.video_blur_radius_px.toFixed(2)} px
                    </Descriptions.Item>
                    <Descriptions.Item label="像素缩放">
                      {researchParams.video.pixel_scale_percent.toFixed(2)} → {runtimePreview.video_pixel_scale_percent.toFixed(2)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="X 偏移">
                      {researchParams.video.space_x_offset_px.toFixed(2)} → {runtimePreview.video_space_x_offset_px.toFixed(2)} px
                    </Descriptions.Item>
                    <Descriptions.Item label="Y 偏移">
                      {researchParams.video.space_y_offset_px.toFixed(2)} → {runtimePreview.video_space_y_offset_px.toFixed(2)} px
                    </Descriptions.Item>
                    <Descriptions.Item label="锐化">{researchParams.video.sharpen_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="噪点">{researchParams.video.noise_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="细节增强">{researchParams.video.detail_enhancement_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="动态裁剪">{researchParams.video.dynamic_crop_percent.toFixed(2)}%/边</Descriptions.Item>
                    <Descriptions.Item label="像素扰动">{researchParams.video.pixel_jitter_px.toFixed(2)} px</Descriptions.Item>
                    <Descriptions.Item label="裁剪边缘平滑">{researchParams.video.crop_edge_smoothing.toFixed(3)}</Descriptions.Item>
                    <Descriptions.Item label="帧率微扰">{researchParams.video.frame_rate_jitter_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="帧率微扰频率">{researchParams.video.frame_rate_perturbation_frequency_hz.toFixed(3)} Hz</Descriptions.Item>
                    <Descriptions.Item label="帧率微扰幅度">{researchParams.video.frame_rate_perturbation_amplitude_fps.toFixed(3)} fps</Descriptions.Item>
                    <Descriptions.Item label="帧内微扰">{researchParams.video.frame_inner_perturbation_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="帧间微扰">{researchParams.video.frame_inter_perturbation_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="色域转换">{researchParams.video.color_space_conversion_strength_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="目标频率">{researchParams.research.target_frequency_hz == null ? '关闭' : `${researchParams.research.target_frequency_hz.toFixed(1)} Hz`}</Descriptions.Item>
                    <Descriptions.Item label="核心频率">{researchParams.research.core_frequency_hz == null ? '跟随目标' : `${researchParams.research.core_frequency_hz.toFixed(1)} Hz`}</Descriptions.Item>
                    <Descriptions.Item label="波频强度">{researchParams.research.wave_intensity.toFixed(3)}</Descriptions.Item>
                    <Descriptions.Item label="波频电平">{researchParams.research.wave_level.toFixed(3)}</Descriptions.Item>
                    <Descriptions.Item label="波频颗粒数">{researchParams.research.wave_grain_count}</Descriptions.Item>
                    <Descriptions.Item label="动态均衡阈值">{researchParams.research.dynamic_eq_threshold.toFixed(2)}</Descriptions.Item>
                    <Descriptions.Item label="通道偏移">{researchParams.research.channel_offset_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="空间维度">{researchParams.research.space_dimension}</Descriptions.Item>
                    <Descriptions.Item label="空间 X 偏移">{researchParams.research.frequency_space_x_offset_px.toFixed(2)} px</Descriptions.Item>
                    <Descriptions.Item label="空间 Y 偏移">{researchParams.research.frequency_space_y_offset_px.toFixed(2)} px</Descriptions.Item>
                    <Descriptions.Item label="帧扰动概率">{researchParams.research.frame_perturbation_probability_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="随机图形透明度">{researchParams.research.random_graphic_opacity_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="随机图形大小">{researchParams.research.random_graphic_size_px.toFixed(1)} px</Descriptions.Item>
                    <Descriptions.Item label="抽象人脸数量">{researchParams.research.abstract_face_count}</Descriptions.Item>
                    <Descriptions.Item label="抽象人脸大小">{researchParams.research.abstract_face_size_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="抽象人脸透明度">{researchParams.research.abstract_face_opacity_percent.toFixed(2)}%</Descriptions.Item>
                    <Descriptions.Item label="画面偏移">{researchParams.research.overlay_offset_px.toFixed(2)} px</Descriptions.Item>
                    <Descriptions.Item label="切片长度">{(researchParams.research.slice_length_ms / 1000).toFixed(2)} s</Descriptions.Item>
                    <Descriptions.Item label="切片最小长度">{(researchParams.research.slice_min_length_ms / 1000).toFixed(2)} s</Descriptions.Item>
                    <Descriptions.Item label="切片触发间隔">{(researchParams.research.slice_trigger_interval_ms / 1000).toFixed(2)} s</Descriptions.Item>
                    {VISUAL_BAND_FREQUENCIES_HZ.map((hz) => (
                      <Descriptions.Item key={hz} label={`${hz}Hz`}>
                        {(researchParams.research.band_weights[String(hz)] ?? 1).toFixed(3)}×
                      </Descriptions.Item>
                    ))}
                  </Descriptions>
                ) : (
                  <Alert type="info" showIcon message="打开视频处理开关并播放后，运行时预览会按周期变化；关闭后恢复配置基线。" />
                )}
                {runtimeChannelError ? <Alert type="warning" showIcon message={runtimeChannelError} /> : null}
              </Space>
            </Card>
            <Card title="本地研究分析 Worker">
              <Space direction="vertical" style={{ width: '100%' }}>
                <Alert
                  type={researchWorkerCapabilities?.available ? 'success' : 'warning'}
                  showIcon
                  message={
                    researchWorkerCapabilities?.available
                      ? `研究 Worker 已配置：${researchWorkerCapabilities.executable}`
                      : `研究 Worker 不可用：${researchWorkerCapabilities?.reason ?? '未完成能力探测'}`
                  }
                />
                <Space wrap>
                  <Button
                    type="primary"
                    onClick={() => void startResearchAnalysis(false)}
                    loading={researchActionBusy}
                    disabled={!researchParams || !snapshot?.source_media || !researchWorkerCapabilities?.available || researchStatus?.state === 'running'}
                  >
                    开始分析
                  </Button>
                  <Button
                    onClick={() => void startResearchAnalysis(true)}
                    loading={researchActionBusy}
                    disabled={!researchParams || !snapshot?.source_media || !researchWorkerCapabilities?.available || researchStatus?.state === 'running'}
                  >
                    分析并生成研究 MP4
                  </Button>
                  <Button danger loading={researchCancelBusy} onClick={() => void cancelResearchAnalysis()} disabled={researchStatus?.state !== 'running' || researchCancelBusy}>
                    取消
                  </Button>
                  <Tag color={researchStatus?.state === 'ready' ? 'green' : researchStatus?.state === 'failed' ? 'red' : 'blue'}>
                    状态：{researchStatus?.state ?? 'idle'}
                  </Tag>
                </Space>
                <Checkbox disabled>研究结果仅用于本地授权分析，不参与播放决策</Checkbox>
                {researchStatus?.error ? <Alert type="error" showIcon message={researchStatus.error} /> : null}
                {researchStatus?.state === 'ready' ? (
                  <Descriptions column={2} size="small">
                    <Descriptions.Item label="内容相似度">{researchStatus.content_similarity_percent ?? '-'}%</Descriptions.Item>
                    <Descriptions.Item label="媒体鲁棒性">{researchStatus.media_robustness_score ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="隐形标记状态">{researchStatus.invisible_mark_status ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="随机种子">{researchStatus.random_seed ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="算法版本">{researchStatus.algorithm_version ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="报告 SHA-256">{researchStatus.report_sha256 ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="原始 MP4 SHA-256">{researchStatus.source_mp4_sha256 ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="阶段输入 MP4 SHA-256">{researchStatus.input_mp4_sha256 ?? '-'}</Descriptions.Item>
                    <Descriptions.Item label="当前 MP4 SHA-256">{researchStatus.current_mp4_sha256 ?? '-'}</Descriptions.Item>
                  </Descriptions>
                ) : null}
                <Space wrap>
                  <Popconfirm
                    title="删除已生成缓存？"
                    description="请先暂停或停止声音处理播放；将删除未被当前或待切换引用的处理后 MP4，以及超过 10 分钟的临时文件。"
                    okText="确认删除"
                    cancelText="取消"
                    onConfirm={() => void cleanupLocalCaches()}
                  >
                    <Button loading={cacheCleanupBusy} disabled={cacheCleanupBusy}>删除已生成缓存</Button>
                  </Popconfirm>
                  {cacheCleanup ? (
                    <Tag>
                      已删除 {cacheCleanup.removed_files} 个文件 / {(cacheCleanup.removed_bytes / 1024 / 1024).toFixed(1)} MB；剩余 {(cacheCleanup.remaining_bytes / 1024 / 1024).toFixed(1)} MB
                    </Tag>
                  ) : null}
                </Space>
                {cacheCleanupError ? <Alert type="error" showIcon message={cacheCleanupError} /> : null}
              </Space>
            </Card>
          </section>
        </div>
      </Layout.Content>
    </Layout>
  );
}

export default function App() {
  return (
    <ConfigProvider csp={{ nonce: getCspNonce() }}>
      <AntApp>{isFinalEffectWindow ? <FinalEffectWindow /> : <DesktopApp />}</AntApp>
    </ConfigProvider>
  );
}
