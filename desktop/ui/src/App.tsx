import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AudioOutlined,
  FolderOpenOutlined,
  MessageOutlined,
  MutedOutlined,
  PauseCircleOutlined,
  PictureOutlined,
  PlayCircleOutlined,
  ReloadOutlined,
  SettingOutlined,
  SoundOutlined,
  StopOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import { App as AntApp, Alert, Button, Card, Checkbox, Descriptions, Drawer, Empty, Input, InputNumber, Layout, Popconfirm, Segmented, Select, Slider, Space, Steps, Switch, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { HashRouter, Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { advanceInterludePresetPeriodPlan, buildInterludeScheduleKey, chooseInterludeIndex, createInterludePresetPeriodPlan, interludeFileNameFromPath, interludeIntervalMsToSeconds, interludeIntervalProgress, interludeIntervalSecondsToMs, interludePresetPeriodProgress, interludeVolumeDbToPercent, interludeVolumePercentToDb, INTERLUDE_LIMITS, INTERLUDE_PRESET_PERIOD_LIMITS, nextInterludeAtMs, resolveInterludeClockPlaybackRate, resolvePlaybackAudioSource, shouldPauseInterlude } from './interlude-player';
import type { BaseAudioSource, InterludePresetPeriodPlan } from './interlude-player';
import {
  loadInterludeConfig,
  saveInterludeConfig as saveInterludeConfigToStorage,
  type PersistedInterludeConfig,
} from './interlude-config-storage';
import {
  AUDIO_MIX_PICK_HARD_MAX,
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_MIX_PICK_MAX,
  DEFAULT_AUDIO_MIX_PICK_MIN,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  AUDIO_PARAM_CROSSFADE_SEC,
  buildAudioFxSignature,
  buildAudioVariantsFromCycle,
  loadAudioMixSession,
  PERIOD_HARD_MAX_MS,
  PERIOD_HARD_MIN_MS,
  normalizeAudioMixPickMax,
  normalizeAudioMixPickMin,
  normalizeAudioPeriodRange,
  normalizeVideoPeriodRange,
  loadAudioPeriodRange,
  loadVideoPeriodRange,
  sampleAudioCycle,
  samplePeriodMsInRange,
  saveAudioMixSession,
  saveAudioPeriodRange,
  saveVideoPeriodRange,
  toRuntimePreviewParameters,
} from './runtime-parameter-scheduler';
import type { AudioCycleSample, PeriodRangeMs, RuntimePreviewParameters, SubtleAudioSample } from './runtime-parameter-scheduler';
import {
  clearCycleRetry,
  clearAudioCycleRetry,
  createCycleRetryState,
  createAudioCycleRetryState,
  isCycleRetryReady,
  isAudioCycleRetryReady,
  recordCycleRetryFailure,
  recordAudioCycleRetryFailure,
} from './audio-cycle-retry';
import { appendAudioCycleSnapshot } from './audioCycleSnapshot';
import {
  alignMediaPositionToVideoFrame,
  createMediaCycleQueue,
  getMediaCycleProgressPercent,
  resolveSourceBoundedVideoCycleQueueTargets,
} from './media-cycle-planner';
import type { MediaCyclePlan, MediaCycleQueue, MediaCycleSeed } from './media-cycle-planner';
import { flushLatestPendingApply } from './media-apply-queue';
import {
  createMediaArtifactTimeline,
  doesAudioWindowOverlapArtifact,
} from './media-artifact-timeline';
import type { MediaArtifactTimeline } from './media-artifact-timeline';
import {
  isMediaArtifactIdentityCurrent,
  mapAbsolutePositionToCandidateTime,
  mapAbsolutePositionToSourceClock,
  resolveMediaArtifactSwitchTime,
} from './media-artifact-switch';
import { AudioParameterControls, MediaParameterPanels, type MediaEffectParams } from './media-parameter-panels';
import {
  sampleAutomaticVideoParameters,
} from './video-parameter-randomizer';
import { buildAudioCapabilityRows, resolveAudioStreamBranches } from './audio-processing-capabilities';
import {
  canUseRealtimeVideoBackend,
  parseMediaVideoBackendStatus,
  projectMediaVideoBackendStatus,
  type MediaVideoBackendStatus,
} from './media-video-backend-status';
import {
  classifyAudioOutputSync,
  clearResolvedPortAudioSyncError,
  getPortAudioSourceRetryMode,
  isExpectedAudioOutputSyncCancellation,
  isRetryableAudioOutputSyncCode,
} from './audio-output-recovery';
import {
  resolveSourceAudioSync,
  resolveSynchronizedVideoPlaybackRate,
} from './audio-playback-sync';
import {
  resolvePlaybackBoundaryDuration,
  shouldRestartCurrentSourceImmediately,
  shouldRestartPlayback,
} from './playback-loop';
import {
  buildFinalEffectMediaIdentity,
  buildFinalEffectWindowResizeKey,
  isFinalEffectMediaIdentityCurrent,
} from './final-effect-window-size';
import { capturePlaybackPosition, clampMediaTime, clampVolume, createAudioSyncClock, formatMediaTime, isPlaybackMediaControlMessage, isPlaybackMediaStateMessage, resolvePlaybackResumePosition, shouldAcceptPlaybackMediaState, shouldApplyPlaybackSeek, shouldIssuePlaybackCommand } from './playback-control-message';
import type { AudioSyncClock, PlaybackMediaControlMessage, PlaybackMediaStateMessage, PlaybackPositionCheckpoint } from './playback-control-message';
import {
  createPlaybackClockHealth,
  failPlaybackClockRecovery,
  observePlaybackClock,
  PLAYBACK_CLOCK_RECOVERY_TIMEOUT_MS,
  resetPlaybackClockObservation,
} from './playback-clock-health';
import type { PlaybackClockHealth } from './playback-clock-health';
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
import { CompactNumberField } from './desktop/compact-number-field';
import { ControlPlaneGate } from './desktop/control-plane-gate';
import {
  AudioDiagnosticDisplay,
  type AudioDiagnosticDisplayHandle,
  type AudioDiagnosticSummary,
} from './desktop/audio-diagnostic-display';
import {
  DIAGNOSTIC_LINE_SAMPLE_COUNT,
  DIAGNOSTIC_PUBLISH_INTERVAL_MS,
  DIAGNOSTIC_SAMPLE_COUNT,
  type DiagnosticMessage,
} from './desktop/audio-diagnostic-policy';
import { AudioProcessingPanel } from './desktop/audio-processing-panel';
import { DesktopPanel } from './desktop/desktop-panel';
import { DESKTOP_DRAWER_ROOT_STYLE, FeatureDrawer, FeatureDrawerField, FeatureDrawerSection } from './desktop/feature-drawer';
import { DesktopColumn, DesktopShell } from './desktop/desktop-shell';
import { InlineRecoveryAlert } from './desktop/inline-recovery-alert';
import { InterludePresetParameterRow } from './desktop/interlude-preset-parameter-row';
import { MediaCycleCard } from './desktop/media-cycle-card';
import { PlaybackPoolPanel, type PlaybackPoolSource } from './desktop/playback-pool-panel';
import { PortAudioDevicePanel } from './desktop/portaudio-device-panel';

const PLAYBACK_CHANNEL_NAME = 'autolive-playback-ui-v1';
const FIXED_SPEECH_ACK_TIMEOUT_MS = 3_000;
const RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500;
const PLAYBACK_SNAPSHOT_POLL_MS = 1_000;
const AUDIO_OUTPUT_STATUS_POLL_MS = 2_000;
const AUDIO_VIDEO_REALIGN_THRESHOLD_MS = 80;
const AUDIO_VIDEO_REALIGN_CONSECUTIVE_POLLS = 3;
const AUDIO_VIDEO_REALIGN_COOLDOWN_MS = 10_000;
// 播放时钟以 250ms 采样；门禁覆盖一个采样周期和 WebView 事件抖动。
const MEDIA_ARTIFACT_SWITCH_TOLERANCE_MS = 350;
const MEDIA_ARTIFACT_CONFIRM_TIMEOUT_MS = 6_000;
const MAX_IPC_STRING_LENGTH = 32_768;
const MAX_IPC_OBJECT_FIELDS = 512;
const MAX_IPC_VALUE_NODES = 4_096;
const MAX_INTERLUDE_AUDIO_FILES = 1_000;
const VIDEO_BACKEND_SOURCE_REVISION = 0;
const MPV_REALTIME_VIDEO_ENABLED = true;
// PortAudio 运行后由最终效果窗静音 WebView 主轨，避免双播；失败则保持 WebView。
const PORTAUDIO_FORMAL_SOURCE_SYNC_READY = true;
const AUTO_PORTAUDIO_ENABLED = true;
const AUTO_PORTAUDIO_RETRY_COOLDOWN_MS = 5_000;
const MEDIA_CANDIDATE_SAFETY_TAIL_MS = 500;
// 首块截止必须服从用户设置的周期；GPU/CPU 都不能用额外墙钟追赶迟到候选。
const AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE = 'audio_candidate_outside_source_audio_window';
const PORTAUDIO_SAMPLE_RATE_HZ = 44_100;
const PORTAUDIO_MIN_MEMORY_BUFFER_KIB = 128;
const PORTAUDIO_MAX_MEMORY_BUFFER_KIB = 2_048;
const PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB = 1_024;
const PORTAUDIO_DEFAULT_FRAMES_PER_BUFFER = 256;
const SUPPORTED_MEDIA_EXTENSIONS = [
  'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'ts', 'm2ts', 'flv', 'wmv', '3gp',
  'mp3', 'wav', 'm4a', 'aac', 'ogg', 'flac',
] as const;
const SUPPORTED_VIDEO_EXTENSIONS = SUPPORTED_MEDIA_EXTENSIONS.slice(0, 11);
const PLAYBACK_POOL_LIMIT = 100;
const AUDIO_PARAMETER_SECTIONS = ['audio'] as const;
const VIDEO_PARAMETER_SECTIONS = ['video', 'advanced'] as const;

type AudioCycleDiagnosticSnapshot = {
  sequence: number;
  captured_at_ms: number;
  sample_rate_hz: number;
  captured_frame_count: number;
  line: number[];
  rms_dbfs: number;
  peak_dbfs: number;
  low_band_rms_dbfs: number;
  cutoff_hz: number;
  mfcc: number[];
  mfcc_available: boolean;
  noise_floor_dbfs: number;
  snr_db: number | null;
  formants_hz: Array<number | null>;
  current_formant_hz: number | null;
  has_pcm: boolean;
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
  cancel?: (status: RuntimeResourceStatus) => void;
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

type PlaybackSnapshot = {
  playback_generation: number;
  playback_state: string;
  loop_index: number;
  current_position_ms: number;
  source_media: PlaybackPoolSource | null;
  source_media_pool: PlaybackPoolSource[];
  source_media_index: number | null;
  playback_pool_cycle: number;
  current_video_source: string | null;
  current_video_reference: string | null;
  current_audio_artifact_reference: string | null;
  current_audio_artifact_sha256: string | null;
  current_audio_media_plan_id: string | null;
  current_audio_media_sequence: number | null;
  current_audio_media_playback_generation: number | null;
  current_audio_media_source_revision: number | null;
  current_audio_media_target_absolute_position_ms: number | null;
  current_audio_media_source_start_ms: number | null;
  current_audio_media_output_duration_ms: number | null;
  current_audio_media_valid_until_absolute_position_ms: number | null;
  pending_audio_artifact_reference: string | null;
  pending_audio_artifact_sha256: string | null;
  pending_audio_media_plan_id: string | null;
  pending_audio_media_sequence: number | null;
  pending_audio_media_playback_generation: number | null;
  pending_audio_media_source_revision: number | null;
  pending_audio_media_target_absolute_position_ms: number | null;
  pending_audio_media_source_start_ms: number | null;
  pending_audio_media_output_duration_ms: number | null;
  pending_audio_media_valid_until_absolute_position_ms: number | null;
  video_processing_enabled: boolean;
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
  audio_stream_params?: MediaEffectParams['audio'] | null;
  audio_stream_variants?: MediaEffectParams['audio'][];
  audio_stream_revision: number;
  audio_processing_status: string;
  audio_processing_progress_percent: number;
  audio_processing_runtime: boolean;
  audio_processing_gain_db: number;
  interlude?: InterludeSnapshot | null;
};

type InterludeRuntimeMessage = {
  version: 1;
  type: 'interlude-runtime';
  status: 'idle' | 'starting' | 'playing' | 'failed';
  file_cycle: number;
  preset_segment: number;
  preset_ids: string[];
  file_name: string | null;
  media_position_ms: number;
  period_ms: number | null;
  next_boundary_ms: number | null;
  file_progress_percent: number;
  progress_percent: number;
  output_backend: 'portaudio' | 'webview' | null;
  error: string | null;
  started_at_ms: number | null;
  sent_at_ms: number;
};

type StartPortAudioInterludeResult = {
  generation: number;
};

type InterludeVolumeControlMessage = {
  version: 1;
  type: 'interlude-volume-control';
  volume_db: number;
};

type PlaybackItemCompletionResult = {
  snapshot: PlaybackSnapshot;
  source_changed: boolean;
};

type InterludeSnapshot = {
  enabled: boolean;
  directory: string | null;
  audio_selection_mode?: InterludeAudioSelectionMode;
  audio_fixed_preset_id?: string;
  audio_preset_ids?: string[];
  audio_mix_enabled?: boolean;
  audio_mix_pick_min?: number;
  audio_mix_pick_max?: number;
  audio_variation_mode?: InterludeAudioVariationMode;
  audio_variation_period_min_ms?: number;
  audio_variation_period_max_ms?: number;
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

type InterludeAudioSelectionMode = 'fixed' | 'random';
type InterludeAudioVariationMode = 'each_playback' | 'periodic';

type InterludeConfigDraft = {
  enabled: boolean;
  directory: string | null;
  audioSelectionMode: InterludeAudioSelectionMode;
  audioFixedPresetId: string;
  audioPresetIds: string[];
  audioMixEnabled: boolean;
  audioMixPickMin: number;
  audioMixPickMax: number;
  audioVariationPeriodMinMs: number;
  audioVariationPeriodMaxMs: number;
  intervalMinMs: number;
  intervalMaxMs: number;
  volumeDb: number;
  duckingDepthDb: number;
  duckingAttackMs: number;
  duckingReleaseMs: number;
};

type PrepareWebViewInterludeResult = {
  state: 'processed' | 'original_fallback';
  path: string;
  output_size_bytes: number | null;
  ambient_sound_source: string | null;
  reason_code: string | null;
  reason: string | null;
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
  reason_code: string | null;
  retryable: boolean;
  xrun_count: number;
  hardware_state: string;
  callback_status_flags: number;
  callback_status_flags_count: number;
  callback_underrun_count: number;
  producer_drop_count: number;
  output_latency_ms: number;
  playback_watermark_ms: number;
  actual_sample_rate_hz: number | null;
  audio_timeline_position_ms: number | null;
  av_offset_ms: number | null;
  callback_stalled_ms: number | null;
  pcm_stalled_ms: number | null;
  recovery_required: boolean;
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

function getCommandErrorCode(cause: unknown): string | null {
  if (!cause || typeof cause !== 'object' || !('code' in cause)) return null;
  const code = (cause as { code?: unknown }).code;
  return typeof code === 'string' && code.length > 0 ? code : null;
}

function resolveAudioSyncClock(
  snapshot: PlaybackSnapshot | null | undefined,
  mediaState: PlaybackMediaStateMessage | null | undefined,
  video: HTMLVideoElement | null,
  localLoopIndex?: number,
): AudioSyncClock | null {
  if (!snapshot) return null;
  if (mediaState?.playback_generation === snapshot.playback_generation && mediaState.duration_ms > 0) {
    return {
      playback_generation: mediaState.playback_generation,
      loop_index: mediaState.loop_index,
      position_ms: mediaState.position_ms,
      duration_ms: mediaState.duration_ms,
      absolute_position_ms: mediaState.absolute_position_ms,
    };
  }
  const durationMs = snapshot.source_media?.duration_ms;
  if (typeof durationMs !== 'number' || !Number.isSafeInteger(durationMs) || durationMs <= 0) {
    return null;
  }
  const videoPositionMs = video && Number.isFinite(video.currentTime)
    ? Math.round(video.currentTime * 1_000)
    : snapshot.current_position_ms;
  const positionMs = Math.min(durationMs, Math.max(0, videoPositionMs));
  const loopIndex = localLoopIndex ?? snapshot.loop_index;
  return createAudioSyncClock({
    playbackGeneration: snapshot.playback_generation,
    loopIndex,
    positionMs,
    durationMs,
  });
}

type AudioOutputDevice = {
  id: string;
  name: string;
  host_api: string;
  max_output_channels: number;
  default_sample_rate_hz: number;
};

type PlannedAudioCyclePayload = {
  sample: AudioCycleSample;
  audio: MediaEffectParams['audio'];
  audioVariants: MediaEffectParams['audio'][];
  periodMs: number;
};

type PreparedVideoMediaCandidate = {
  params: MediaEffectParams;
  videoCyclePlan: MediaCyclePlan<PlannedVideoCyclePayload> | null;
  videoEffectsEnabled: boolean;
  realtimePrepared: boolean;
  timeline: MediaArtifactTimeline;
  sourcePath: string;
  sourceMediaIndex: number;
  playbackGeneration: number;
};

type PreparedAudioMediaCandidate = {
  audioCyclePlan: MediaCyclePlan<PlannedAudioCyclePayload>;
  timeline: MediaArtifactTimeline;
  sourcePath: string;
  playbackGeneration: number;
  loopSource: boolean;
  artifactReference: string | null;
};

type PlannedVideoCyclePayload = {
  seed: number;
  videoEffectsEnabled: boolean;
  skipVideoProcessing: boolean;
  video: MediaEffectParams['video'];
  advanced: MediaEffectParams['advanced'];
  periodMs: number;
};

function getActualAudioOutputLabel(status: AudioOutputBackendStatus | null): 'PortAudio' | 'WebView' {
  return status?.running === true && status.selected_backend?.trim().toLowerCase() === 'portaudio'
    ? 'PortAudio'
    : 'WebView';
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

type CacheCleanupResult = {
  removed_files: number;
  removed_bytes: number;
  remaining_bytes: number;
};

type MediaParameterValidationResult = {
  valid: boolean;
  errors: Array<{ field: string; message: string }>;
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
  'video_rotation_degrees',
];

const RUNTIME_PARAMETER_BOOLEAN_FIELDS: Array<keyof RuntimePreviewParameters> = [
  'video_horizontal_flip_enabled',
  'video_vertical_flip_enabled',
];

const RUNTIME_PARAMETER_OPTIONAL_FIELDS: Array<keyof RuntimePreviewParameters> = [
  'audio_input_gain_db',
  'audio_output_gain_db',
  'audio_loudness_adjustment_db',
  'audio_pitch_shift_semitones',
  'audio_playback_speed',
  'audio_fade_in_ms',
  'audio_fade_out_ms',
  'audio_reverb_wet_percent',
  'audio_noise_reduction_percent',
  'audio_phase_perturbation_percent',
  'audio_vibrato_frequency_hz',
  'audio_vibrato_depth_percent',
  'audio_environment_noise_percent',
  'audio_environment_noise_dbfs',
  'audio_filter_q',
  'audio_sample_rate_hz',
  'audio_output_bitrate_kbps',
];

function isSafeNonNegativeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isMediaWorkerBusyError(cause: unknown): boolean {
  return getCommandErrorCode(cause) === 'media_worker_already_running';
}

function mergeAudioDisplayParams(
  base: MediaEffectParams['audio'],
  actual: Record<string, unknown> | null,
): MediaEffectParams['audio'] {
  if (!actual) return base;
  const merged: Record<string, unknown> = { ...base };
  Object.entries(actual).forEach(([key, value]) => {
    if (value !== null && value !== undefined) merged[key] = value;
  });
  return merged as unknown as MediaEffectParams['audio'];
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNullableSafeNonNegativeInteger(value: unknown): value is number | null {
  return value === null || isSafeNonNegativeInteger(value);
}

function isBoundedString(value: unknown, allowEmpty = true): value is string {
  return typeof value === 'string'
    && value.length <= MAX_IPC_STRING_LENGTH
    && (allowEmpty || value.length > 0);
}

type IpcValueValidationBudget = {
  remaining: number;
  seen: WeakSet<object>;
};

function isBoundedIpcValue(
  value: unknown,
  depth = 0,
  budget: IpcValueValidationBudget = {
    remaining: MAX_IPC_VALUE_NODES,
    seen: new WeakSet<object>(),
  },
): boolean {
  budget.remaining -= 1;
  if (budget.remaining < 0) return false;
  if (value === null || typeof value === 'boolean' || isBoundedString(value)) return true;
  if (typeof value === 'number') return Number.isFinite(value);
  if (depth >= 6) return false;
  if (Array.isArray(value)) {
    if (value.length > MAX_INTERLUDE_AUDIO_FILES || budget.seen.has(value)) return false;
    budget.seen.add(value);
    const valid = value.every((item) => isBoundedIpcValue(item, depth + 1, budget));
    budget.seen.delete(value);
    return valid;
  }
  if (!value || typeof value !== 'object') return false;
  if (budget.seen.has(value)) return false;
  budget.seen.add(value);
  const entries = Object.entries(value as Record<string, unknown>);
  const valid = entries.length <= MAX_IPC_OBJECT_FIELDS
    && entries.every(
      ([key, item]) => key.length <= 128 && isBoundedIpcValue(item, depth + 1, budget),
    );
  budget.seen.delete(value);
  return valid;
}

function isPlaybackPoolSource(value: unknown): value is PlaybackPoolSource {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const nullablePositiveSafeInteger = (item: unknown) =>
    item === null || (isSafeNonNegativeInteger(item) && item > 0);
  return isBoundedString(record.source_path, false)
    && (record.media_kind === 'video' || record.media_kind === 'audio')
    && isBoundedString(record.playback_reference, false)
    && ['direct', 'remuxed', 'transcoded'].includes(record.compatibility_mode as string)
    && isBoundedString(record.file_name, false)
    && isSafeNonNegativeInteger(record.file_size_bytes)
    && isNullableSafeNonNegativeInteger(record.duration_ms)
    && nullablePositiveSafeInteger(record.width)
    && nullablePositiveSafeInteger(record.height)
    && (record.frame_rate_fps === null || (isFiniteNumber(record.frame_rate_fps) && record.frame_rate_fps > 0))
    && nullablePositiveSafeInteger(record.audio_sample_rate_hz)
    && nullablePositiveSafeInteger(record.audio_channel_count)
    && isNullableSafeNonNegativeInteger(record.audio_start_ms)
    && isNullableSafeNonNegativeInteger(record.audio_end_ms)
    && (
      (record.audio_start_ms === null && record.audio_end_ms === null)
      || (
        typeof record.audio_start_ms === 'number'
        && typeof record.audio_end_ms === 'number'
        && record.audio_end_ms > record.audio_start_ms
        && (record.duration_ms === null || record.audio_end_ms <= record.duration_ms)
      )
    )
    && (record.mp4_sha256 === null || (isBoundedString(record.mp4_sha256) && /^[0-9a-f]{64}$/i.test(record.mp4_sha256)))
    && ['disabled', 'pending', 'ready', 'failed'].includes(record.mp4_hash_status as string);
}

function isInterludeSnapshot(value: unknown): value is InterludeSnapshot {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const audioFiles = record.audio_files;
  const audioPresetIds = record.audio_preset_ids;
  if (
    !Array.isArray(audioFiles)
    || audioFiles.length > MAX_INTERLUDE_AUDIO_FILES
    || !audioFiles.every((path) => isBoundedString(path, false))
    || !Array.isArray(audioPresetIds)
    || audioPresetIds.length > AUDIO_VALUE_PRESETS.length
    || !audioPresetIds.every((id) => isBoundedString(id, false))
  ) {
    return false;
  }
  return typeof record.enabled === 'boolean'
    && (record.directory === null || isBoundedString(record.directory, false))
    && (record.audio_selection_mode === 'fixed' || record.audio_selection_mode === 'random')
    && isBoundedString(record.audio_fixed_preset_id, false)
    && typeof record.audio_mix_enabled === 'boolean'
    && isSafeNonNegativeInteger(record.audio_mix_pick_min)
    && isSafeNonNegativeInteger(record.audio_mix_pick_max)
    && record.audio_mix_pick_min >= 1
    && record.audio_mix_pick_max <= AUDIO_MIX_PICK_HARD_MAX
    && record.audio_mix_pick_min <= record.audio_mix_pick_max
    && (record.audio_variation_mode === 'each_playback' || record.audio_variation_mode === 'periodic')
    && isSafeNonNegativeInteger(record.audio_variation_period_min_ms)
    && isSafeNonNegativeInteger(record.audio_variation_period_max_ms)
    && record.audio_variation_period_min_ms <= record.audio_variation_period_max_ms
    && isSafeNonNegativeInteger(record.audio_count)
    && record.audio_count === audioFiles.length
    && isBoundedString(record.status)
    && (record.error === null || isBoundedString(record.error))
    && isSafeNonNegativeInteger(record.interval_min_ms)
    && isSafeNonNegativeInteger(record.interval_max_ms)
    && record.interval_min_ms <= record.interval_max_ms
    && isFiniteNumber(record.volume_db)
    && isFiniteNumber(record.ducking_depth_db)
    && isSafeNonNegativeInteger(record.ducking_attack_ms)
    && isSafeNonNegativeInteger(record.ducking_release_ms);
}

function isPlaybackSnapshot(value: unknown): value is PlaybackSnapshot {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const sourceMediaPool = record.source_media_pool;
  if (
    !Array.isArray(sourceMediaPool)
    || sourceMediaPool.length > 100
    || !sourceMediaPool.every(isPlaybackPoolSource)
  ) {
    return false;
  }
  const sourceMedia = record.source_media;
  const sourceMediaIndex = record.source_media_index;
  const sourceCombinationValid = sourceMediaPool.length === 0
    ? sourceMedia === null && (sourceMediaIndex === null || sourceMediaIndex === 0)
    : isPlaybackPoolSource(sourceMedia)
      && isSafeNonNegativeInteger(sourceMediaIndex)
      && sourceMediaIndex < sourceMediaPool.length
      && sourceMediaPool[sourceMediaIndex].source_path === sourceMedia.source_path;
  const nullableBoundedString = (item: unknown) => item === null || isBoundedString(item);
  const nullableHash = (item: unknown) => item === null
    || (isBoundedString(item) && /^[0-9a-f]{64}$/i.test(item));
  const processingStatuses = ['disabled', 'unavailable', 'not_applicable', 'configured', 'processing', 'runtime', 'ready', 'failed'];
  const audioStreamVariants = record.audio_stream_variants;
  return sourceCombinationValid
    && ['ready', 'playing', 'paused', 'stopped'].includes(record.playback_state as string)
    && isSafeNonNegativeInteger(record.playback_generation)
    && isSafeNonNegativeInteger(record.loop_index)
    && isSafeNonNegativeInteger(record.current_position_ms)
    && isSafeNonNegativeInteger(record.playback_pool_cycle)
    && nullableBoundedString(record.current_video_source)
    && nullableBoundedString(record.current_video_reference)
    && nullableBoundedString(record.current_audio_artifact_reference)
    && nullableHash(record.current_audio_artifact_sha256)
    && nullableBoundedString(record.current_audio_media_plan_id)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_sequence)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_playback_generation)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_source_revision)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_target_absolute_position_ms)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_source_start_ms)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_output_duration_ms)
    && isNullableSafeNonNegativeInteger(record.current_audio_media_valid_until_absolute_position_ms)
    && nullableBoundedString(record.pending_audio_artifact_reference)
    && nullableHash(record.pending_audio_artifact_sha256)
    && nullableBoundedString(record.pending_audio_media_plan_id)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_sequence)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_playback_generation)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_source_revision)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_target_absolute_position_ms)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_source_start_ms)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_output_duration_ms)
    && isNullableSafeNonNegativeInteger(record.pending_audio_media_valid_until_absolute_position_ms)
    && typeof record.video_processing_enabled === 'boolean'
    && typeof record.audio_processing_enabled === 'boolean'
    && typeof record.realtime_audio_variant_enabled === 'boolean'
    && nullableBoundedString(record.current_audio_source)
    && nullableBoundedString(record.current_audio_reference)
    && isSafeNonNegativeInteger(record.current_audio_start_at_ms)
    && (record.effective_audio_source === undefined
      || record.effective_audio_source === null
      || ['realtime_variant', 'processed_original', 'original'].includes(record.effective_audio_source as string))
    && nullableHash(record.current_mp4_sha256)
    && nullableHash(record.current_audio_sha256)
    && isBoundedString(record.audio_decision)
    && isBoundedString(record.worker_status)
    && nullableBoundedString(record.fallback_reason)
    && typeof record.pending_audio_candidate === 'boolean'
    && nullableBoundedString(record.pending_audio_reference)
    && isNullableSafeNonNegativeInteger(record.pending_audio_start_at_ms)
    && isNullableSafeNonNegativeInteger(record.pending_audio_duration_ms)
    && isBoundedString(record.audio_processing_parameters_version)
    && isBoundedIpcValue(record.audio_stream_params)
    && Array.isArray(audioStreamVariants)
    && audioStreamVariants.length <= AUDIO_MIX_PICK_HARD_MAX
    && audioStreamVariants.every((variant) => isBoundedIpcValue(variant))
    && (record.audio_stream_variant_count === undefined
      || record.audio_stream_variant_count === null
      || (isSafeNonNegativeInteger(record.audio_stream_variant_count)
        && record.audio_stream_variant_count <= AUDIO_MIX_PICK_HARD_MAX
        && (record.audio_stream_variant_count === 0
          || record.audio_stream_variant_count === Math.max(1, audioStreamVariants.length))))
    && isSafeNonNegativeInteger(record.audio_stream_revision)
    && processingStatuses.includes(record.audio_processing_status as string)
    && isSafeNonNegativeInteger(record.audio_processing_progress_percent)
    && record.audio_processing_progress_percent <= 100
    && typeof record.audio_processing_runtime === 'boolean'
    && isFiniteNumber(record.audio_processing_gain_db)
    && isInterludeSnapshot(record.interlude);
}

function isPlaybackItemCompletionResult(value: unknown): value is PlaybackItemCompletionResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return typeof record.source_changed === 'boolean' && isPlaybackSnapshot(record.snapshot);
}

function isRuntimePreviewParameters(value: unknown): value is RuntimePreviewParameters {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return RUNTIME_PARAMETER_FIELDS.every((field) => isFiniteNumber(record[field]))
    && RUNTIME_PARAMETER_BOOLEAN_FIELDS.every((field) => typeof record[field] === 'boolean')
    && RUNTIME_PARAMETER_OPTIONAL_FIELDS.every(
      (field) => record[field] === undefined || isFiniteNumber(record[field]),
    );
}

function isAudioOutputBackendMessage(value: unknown): value is AudioOutputBackendMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return (
    record.version === 1
    && record.type === 'audio-output-backend'
    && typeof record.preferred_portaudio === 'boolean'
    && typeof record.running === 'boolean'
    && (record.selected_backend === 'portaudio' || record.selected_backend === 'webview')
    && (record.channels === undefined
      || (isSafeNonNegativeInteger(record.channels) && record.channels >= 1 && record.channels <= 32))
    && (record.sample_rate_hz === undefined
      || (isSafeNonNegativeInteger(record.sample_rate_hz)
        && record.sample_rate_hz >= 8_000
        && record.sample_rate_hz <= 384_000))
    && (!record.running || (record.preferred_portaudio && record.selected_backend === 'portaudio'))
  );
}

function isAudioOutputBackendClosedMessage(value: unknown): value is AudioOutputBackendClosedMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return record.version === 1 && record.type === 'audio-output-backend-closed';
}

function isRuntimeParameterMessage(value: unknown): value is RuntimeParameterMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const enabled = record.audio_processing_enabled === true || record.video_processing_enabled === true;
  return (
    record.version === 1 &&
    record.type === 'runtime-parameters' &&
    typeof record.audio_processing_enabled === 'boolean' &&
    typeof record.video_processing_enabled === 'boolean' &&
    (record.playback_generation === null || isSafeNonNegativeInteger(record.playback_generation)) &&
    (enabled ? isRuntimePreviewParameters(record.payload) : record.payload === null)
  );
}

function isInterludeRuntimeMessage(value: unknown): value is InterludeRuntimeMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return record.version === 1
    && record.type === 'interlude-runtime'
    && ['idle', 'starting', 'playing', 'failed'].includes(record.status as string)
    && isSafeNonNegativeInteger(record.file_cycle)
    && isSafeNonNegativeInteger(record.preset_segment)
    && Array.isArray(record.preset_ids)
    && record.preset_ids.length <= AUDIO_MIX_PICK_HARD_MAX
    && record.preset_ids.every((id) => typeof id === 'string' && INTERLUDE_AUDIO_PRESET_IDS.has(id))
    && (record.file_name === null || (
      typeof record.file_name === 'string'
      && record.file_name.length > 0
      && record.file_name.length <= 512
      && !/[\\/]/.test(record.file_name)
    ))
    && isSafeNonNegativeInteger(record.media_position_ms)
    && (record.period_ms === null || isSafeNonNegativeInteger(record.period_ms))
    && (record.next_boundary_ms === null || isSafeNonNegativeInteger(record.next_boundary_ms))
    && isFiniteNumber(record.file_progress_percent)
    && record.file_progress_percent >= 0
    && record.file_progress_percent <= 100
    && isFiniteNumber(record.progress_percent)
    && record.progress_percent >= 0
    && record.progress_percent <= 100
    && (record.output_backend === null || record.output_backend === 'portaudio' || record.output_backend === 'webview')
    && (record.error === null || isBoundedString(record.error))
    && (record.status === 'playing'
      ? isSafeNonNegativeInteger(record.started_at_ms)
      : record.started_at_ms === null)
    && isSafeNonNegativeInteger(record.sent_at_ms);
}

function isInterludeVolumeControlMessage(value: unknown): value is InterludeVolumeControlMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return record.version === 1
    && record.type === 'interlude-volume-control'
    && isFiniteNumber(record.volume_db)
    && record.volume_db >= INTERLUDE_LIMITS.volumeDb.min
    && record.volume_db <= INTERLUDE_LIMITS.volumeDb.max;
}

function isPlaybackControlMessage(value: unknown): value is PlaybackControlMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return record.version === 1 && record.type === 'playback-control' && ['pause', 'resume', 'stop'].includes(record.action as string);
}

function isAudioOutputBackendStatus(value: unknown): value is AudioOutputBackendStatus {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const nonNegativeIntegerFields = [
    'xrun_count',
    'callback_status_flags',
    'callback_status_flags_count',
    'callback_underrun_count',
    'producer_drop_count',
    'output_latency_ms',
    'playback_watermark_ms',
    'ring_len_samples',
    'ring_capacity_samples',
  ];
  const nullableNonNegativeIntegerFields = [
    'actual_sample_rate_hz',
    'audio_timeline_position_ms',
    'callback_stalled_ms',
    'pcm_stalled_ms',
    'current_audio_ffmpeg_pid',
    'pending_audio_ffmpeg_pid',
  ];
  if (
    !nonNegativeIntegerFields.every((field) => isSafeNonNegativeInteger(record[field]))
    || !nullableNonNegativeIntegerFields.every((field) => isNullableSafeNonNegativeInteger(record[field]))
  ) {
    return false;
  }
  return typeof record.available === 'boolean'
    && (record.selected_backend === 'portaudio' || record.selected_backend === 'webview')
    && typeof record.preferred_portaudio === 'boolean'
    && typeof record.running === 'boolean'
    && (record.reason === null || isBoundedString(record.reason))
    && (record.reason_code === null || isBoundedString(record.reason_code))
    && typeof record.retryable === 'boolean'
    && ['unsupported', 'not_created', 'active', 'stopped', 'inactive', 'unknown', 'query_error'].includes(record.hardware_state as string)
    && (record.av_offset_ms === null || (typeof record.av_offset_ms === 'number' && Number.isSafeInteger(record.av_offset_ms)))
    && typeof record.recovery_required === 'boolean'
    && (record.ring_len_samples as number) <= (record.ring_capacity_samples as number)
    && (record.device_index === null || isSafeNonNegativeInteger(record.device_index))
    && isSafeNonNegativeInteger(record.memory_buffer_kib)
    && record.memory_buffer_kib >= PORTAUDIO_MIN_MEMORY_BUFFER_KIB
    && record.memory_buffer_kib <= PORTAUDIO_MAX_MEMORY_BUFFER_KIB
    && isSafeNonNegativeInteger(record.frames_per_buffer)
    && record.frames_per_buffer > 0
    && record.frames_per_buffer <= 65_536
    && isSafeNonNegativeInteger(record.sample_rate_hz)
    && record.sample_rate_hz >= 8_000
    && record.sample_rate_hz <= 384_000
    && isSafeNonNegativeInteger(record.channels)
    && record.channels >= 1
    && record.channels <= 32
    && isSafeNonNegativeInteger(record.audio_task_count)
    && record.audio_task_count <= 2
    && (!record.running || (record.preferred_portaudio && record.selected_backend === 'portaudio'));
}

function isAudioOutputDeviceList(value: unknown): value is AudioOutputDevice[] {
  if (!Array.isArray(value) || value.length > 512) return false;
  const hostApis = ['default', 'wasapi', 'asio', 'mme', 'dsound', 'wdmks', 'other'];
  return value.every((device) => {
    if (!device || typeof device !== 'object' || Array.isArray(device)) return false;
    const record = device as Record<string, unknown>;
    return isBoundedString(record.id, false)
      && isBoundedString(record.name, false)
      && typeof record.host_api === 'string'
      && hostApis.includes(record.host_api.trim().toLowerCase())
      && isSafeNonNegativeInteger(record.max_output_channels)
      && record.max_output_channels <= 32
      && isSafeNonNegativeInteger(record.default_sample_rate_hz)
      && record.default_sample_rate_hz >= 8_000
      && record.default_sample_rate_hz <= 384_000;
  });
}

function isMediaEffectParams(value: unknown): value is MediaEffectParams {
  if (!value || typeof value !== 'object' || Array.isArray(value) || !isBoundedIpcValue(value)) return false;
  const record = value as Record<string, unknown>;
  return ['audio', 'video', 'advanced'].every((section) => {
    const item = record[section];
    return Boolean(item) && typeof item === 'object' && !Array.isArray(item);
  });
}

function isMediaParameterValidationResult(value: unknown): value is MediaParameterValidationResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return typeof record.valid === 'boolean'
    && Array.isArray(record.errors)
    && record.errors.length <= MAX_IPC_OBJECT_FIELDS
    && record.errors.every((error) => {
      if (!error || typeof error !== 'object' || Array.isArray(error)) return false;
      const item = error as Record<string, unknown>;
      return isBoundedString(item.field) && isBoundedString(item.message);
    });
}

function isPrepareWebViewInterludeResult(value: unknown): value is PrepareWebViewInterludeResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return (record.state === 'processed' || record.state === 'original_fallback')
    && isBoundedString(record.path, false)
    && isNullableSafeNonNegativeInteger(record.output_size_bytes)
    && (record.ambient_sound_source === null || isBoundedString(record.ambient_sound_source))
    && (record.reason_code === null || isBoundedString(record.reason_code))
    && (record.reason === null || isBoundedString(record.reason));
}

function isStartPortAudioInterludeResult(value: unknown): value is StartPortAudioInterludeResult {
  if (!value || typeof value !== 'object') return false;
  const generation = (value as Record<string, unknown>).generation;
  return typeof generation === 'number' && Number.isSafeInteger(generation) && generation > 0;
}

function isMediaEngineCapabilities(value: unknown): value is MediaEngineCapabilities {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return typeof record.available === 'boolean'
    && (record.ffmpeg_version === null || isBoundedString(record.ffmpeg_version))
    && (record.ffprobe_version === null || isBoundedString(record.ffprobe_version))
    && (record.reason === null || isBoundedString(record.reason));
}

function isRuntimeResourceStatus(value: unknown): value is RuntimeResourceStatus {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const byteFields = [
    'downloaded_bytes',
    'total_bytes',
    'bytes_per_second',
    'installed_bytes',
  ];
  return ['not-installed', 'checking', 'downloading', 'verifying', 'ready', 'failed', 'cancelled']
    .includes(record.state as string)
    && (record.component === null || record.component === 'media')
    && (record.current_file === null || isBoundedString(record.current_file))
    && byteFields.every((field) => isSafeNonNegativeInteger(record[field]))
    && isBoundedString(record.resource_root)
    && (record.error === null || isBoundedString(record.error));
}

function isCacheCleanupResult(value: unknown): value is CacheCleanupResult {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return ['removed_files', 'removed_bytes', 'remaining_bytes']
    .every((field) => isSafeNonNegativeInteger(record[field]));
}

async function invokePlaybackSnapshot(
  command: string,
  args?: Record<string, unknown>,
): Promise<PlaybackSnapshot> {
  const response = await invoke<unknown>(command, args);
  if (!isPlaybackSnapshot(response)) throw new Error('桌面播放状态响应无效');
  return response;
}

async function invokeAudioOutputBackendStatus(
  command: string,
  args?: Record<string, unknown>,
): Promise<AudioOutputBackendStatus> {
  const response = await invoke<unknown>(command, args);
  if (!isAudioOutputBackendStatus(response)) throw new Error('音频出口状态响应无效');
  return response;
}

async function invokeRuntimeResourceStatus(
  command: string,
  args: Record<string, unknown>,
): Promise<RuntimeResourceStatus> {
  const response = await invoke<unknown>(command, args);
  if (!isRuntimeResourceStatus(response)) throw new Error('运行资源状态响应无效');
  return response;
}

function isAudioCycleDiagnosticSnapshot(value: unknown): value is AudioCycleDiagnosticSnapshot {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.sequence === 'number'
    && Number.isSafeInteger(record.sequence)
    && typeof record.captured_at_ms === 'number'
    && Number.isSafeInteger(record.captured_at_ms)
    && record.captured_at_ms >= 0
    && typeof record.sample_rate_hz === 'number'
    && Number.isFinite(record.sample_rate_hz)
    && record.sample_rate_hz > 0
    && typeof record.captured_frame_count === 'number'
    && Number.isSafeInteger(record.captured_frame_count)
    && record.captured_frame_count >= 0
    && Array.isArray(record.line)
    && (record.line.length === 0 || record.line.length === DIAGNOSTIC_LINE_SAMPLE_COUNT)
    && record.line.every((sample) => typeof sample === 'number' && Number.isFinite(sample) && sample >= -1 && sample <= 1)
    && ['rms_dbfs', 'peak_dbfs', 'low_band_rms_dbfs', 'cutoff_hz', 'noise_floor_dbfs'].every(
      (field) => typeof record[field] === 'number' && Number.isFinite(record[field]),
    )
    && (record.snr_db === null || (typeof record.snr_db === 'number' && Number.isFinite(record.snr_db)))
    && (record.current_formant_hz === null || (typeof record.current_formant_hz === 'number' && Number.isFinite(record.current_formant_hz)))
    && typeof record.mfcc_available === 'boolean'
    && Array.isArray(record.mfcc)
    && record.mfcc.length <= 40
    && record.mfcc.every((coefficient) => typeof coefficient === 'number' && Number.isFinite(coefficient))
    && Array.isArray(record.formants_hz)
    && record.formants_hz.length === 3
    && record.formants_hz.every((formant) => formant === null || (typeof formant === 'number' && Number.isFinite(formant)))
    && typeof record.has_pcm === 'boolean'
  );
}

function useDocumentVisibility(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState !== 'hidden');
  useEffect(() => {
    const handleVisibilityChange = () => setVisible(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, []);
  return visible;
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

type ProcessingStatusKey = 'disabled' | 'not_applicable' | 'configured' | 'processing' | 'ready' | 'runtime' | 'unavailable' | 'failed' | 'unsupported';
function getProcessingStatusKey(status: string | null | undefined, enabled: boolean, configured = false): ProcessingStatusKey {
  if (!enabled) return 'disabled';
  if (status === 'unavailable' && configured) return 'configured';
  if (status === 'not_applicable' || status === 'configured' || status === 'processing' || status === 'ready' || status === 'runtime' || status === 'unavailable' || status === 'failed') {
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
    case 'not_applicable':
      return '当前音频素材不适用';
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
  unit?: string;
  digits?: number;
}[] = [
  { key: 'natural_voice_mode', label: '自然真人模式' },
  { key: 'pitch_shift_semitones', label: '音高', unit: '半音', digits: 3 },
  { key: 'spectral_perturbation_percent', label: '频谱微扰', unit: '%', digits: 3 },
  { key: 'environment_noise_percent', label: '环境噪声', unit: '%', digits: 3 },
  { key: 'environment_noise_dbfs', label: '环境噪声电平', unit: 'dBFS', digits: 3 },
  { key: 'mfcc_shift_percent', label: 'MFCC 偏移', unit: '%', digits: 3 },
  { key: 'phase_perturbation_percent', label: '相位扰动', unit: '%', digits: 3 },
  { key: 'loudness_adjustment_db', label: '响度调整', unit: 'dB', digits: 3 },
  { key: 'input_gain_db', label: '输入增益', unit: 'dB', digits: 3 },
  { key: 'output_gain_db', label: '输出增益', unit: 'dB', digits: 3 },
  { key: 'playback_speed', label: '播放速度', unit: 'x', digits: 3 },
  { key: 'low_eq_db', label: '低频 EQ', unit: 'dB', digits: 3 },
  { key: 'mid_eq_db', label: '中频 EQ', unit: 'dB', digits: 3 },
  { key: 'high_eq_db', label: '高频 EQ', unit: 'dB', digits: 3 },
  { key: 'noise_reduction_percent', label: '降噪', unit: '%', digits: 3 },
  { key: 'ambient_sound_mix_percent', label: '环境声混合', unit: '%', digits: 3 },
  { key: 'fade_in_ms', label: '淡入', unit: 'ms', digits: 3 },
  { key: 'fade_out_ms', label: '淡出', unit: 'ms', digits: 3 },
  { key: 'dry_wet_percent', label: '干湿比', unit: '%', digits: 3 },
  { key: 'reverb_wet_percent', label: '轻混响', unit: '%', digits: 3 },
  { key: 'mfcc_dimensions', label: 'MFCC 维度', unit: '阶', digits: 0 },
  { key: 'snr_variation_db', label: 'SNR 浮动', unit: 'dB', digits: 3 },
  { key: 'formant_shift_percent', label: '共振峰偏移', unit: '%', digits: 3 },
  { key: 'vibrato_frequency_hz', label: '颤音频率', unit: 'Hz', digits: 3 },
  { key: 'vibrato_depth_percent', label: '颤音深度', unit: '%', digits: 3 },
  { key: 'spectrum_blind_spot_percent', label: '频谱盲区', unit: '%', digits: 3 },
  { key: 'snr_target_db', label: '目标信噪比', unit: 'dB', digits: 3 },
  { key: 'filter_q', label: '滤波 Q', digits: 3 },
  { key: 'sample_rate_hz', label: '输出采样率', unit: 'Hz', digits: 0 },
  { key: 'output_bitrate_kbps', label: '输出码率', unit: 'kbps', digits: 0 },
  { key: 'voice_library_id', label: '音色库 ID' },
  { key: 'high_frequency_perturbation_enabled', label: '高频扰动' },
  { key: 'high_frequency_perturbation_interval_ms', label: '高频扰动间隔', unit: 'ms', digits: 0 },
  { key: 'high_frequency_perturbation_strength_percent', label: '高频扰动强度', unit: '%', digits: 3 },
  { key: 'high_frequency_perturbation_level_db', label: '高频扰动电平', unit: 'dB', digits: 3 },
];

function formatAudioPresetFieldValue(
  key: keyof SubtleAudioSample,
  value: SubtleAudioSample[keyof SubtleAudioSample],
  unit = '',
  digits = 3,
) {
  if (value === null) return key === 'snr_target_db' ? '自动（源素材基线）' : '跟随源素材';
  if (typeof value === 'boolean') return value ? '开启' : '关闭';
  if (typeof value === 'string') {
    if (key === 'natural_voice_mode') return value === 'natural_dynamic' ? '自然动态' : '保持原声';
    return value;
  }
  return formatAudioPreviewValue(value, unit, digits);
}

function isSupportedInterludeVideoSource(path: string) {
  const normalizedPath = path.trim().toLowerCase();
  return SUPPORTED_VIDEO_EXTENSIONS.some((extension) => normalizedPath.endsWith(`.${extension}`));
}

function getPlaybackDisplayLabel(state: PlaybackDisplayState) {
  switch (state) {
    case 'loading':
      return '读取中';
    case 'no-source':
      return '未导入媒体';
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
  return db <= -60 ? 0 : Math.pow(10, db / 20);
}

function getEffectiveAudioSource(snapshot: PlaybackSnapshot | null): BaseAudioSource {
  return resolvePlaybackAudioSource({
    effectiveAudioSource: snapshot?.effective_audio_source,
    currentAudioSource: snapshot?.current_audio_source,
    currentVideoSource: snapshot?.current_video_source,
  });
}

function playbackVideoUrl(snapshot: PlaybackSnapshot | null | undefined): string | null {
  return toAssetUrl(snapshot?.source_media?.playback_reference ?? null);
}

function pendingAudioArtifactTimeline(snapshot: PlaybackSnapshot | null | undefined): MediaArtifactTimeline | null {
  if (
    !snapshot?.pending_audio_media_plan_id
    || snapshot.pending_audio_media_sequence === null
    || snapshot.pending_audio_media_playback_generation === null
    || snapshot.pending_audio_media_source_revision === null
    || snapshot.pending_audio_media_target_absolute_position_ms === null
    || snapshot.pending_audio_media_source_start_ms === null
    || snapshot.pending_audio_media_output_duration_ms === null
    || snapshot.pending_audio_media_valid_until_absolute_position_ms === null
    || snapshot.pending_audio_media_valid_until_absolute_position_ms
      <= snapshot.pending_audio_media_target_absolute_position_ms
  ) return null;
  return {
    planId: snapshot.pending_audio_media_plan_id,
    sequence: snapshot.pending_audio_media_sequence,
    playbackGeneration: snapshot.pending_audio_media_playback_generation,
    sourceRevision: snapshot.pending_audio_media_source_revision,
    targetAbsolutePositionMs: snapshot.pending_audio_media_target_absolute_position_ms,
    sourceStartMs: snapshot.pending_audio_media_source_start_ms,
    outputDurationMs: snapshot.pending_audio_media_output_duration_ms,
    validUntilAbsolutePositionMs: snapshot.pending_audio_media_valid_until_absolute_position_ms,
  };
}

function currentAudioArtifactTimeline(snapshot: PlaybackSnapshot | null | undefined): MediaArtifactTimeline | null {
  if (
    !snapshot?.current_audio_media_plan_id
    || snapshot.current_audio_media_sequence === null
    || snapshot.current_audio_media_playback_generation === null
    || snapshot.current_audio_media_source_revision === null
    || snapshot.current_audio_media_target_absolute_position_ms === null
    || snapshot.current_audio_media_source_start_ms === null
    || snapshot.current_audio_media_output_duration_ms === null
    || snapshot.current_audio_media_valid_until_absolute_position_ms === null
    || snapshot.current_audio_media_valid_until_absolute_position_ms
      <= snapshot.current_audio_media_target_absolute_position_ms
  ) return null;
  return {
    planId: snapshot.current_audio_media_plan_id,
    sequence: snapshot.current_audio_media_sequence,
    playbackGeneration: snapshot.current_audio_media_playback_generation,
    sourceRevision: snapshot.current_audio_media_source_revision,
    targetAbsolutePositionMs: snapshot.current_audio_media_target_absolute_position_ms,
    sourceStartMs: snapshot.current_audio_media_source_start_ms,
    outputDurationMs: snapshot.current_audio_media_output_duration_ms,
    validUntilAbsolutePositionMs: snapshot.current_audio_media_valid_until_absolute_position_ms,
  };
}

const INTERLUDE_AUDIO_PRESET_IDS = new Set(AUDIO_VALUE_PRESETS.map((preset) => preset.id));

function normalizeInterludeAudioPresetIds(values?: readonly string[] | null): string[] {
  const presetIds = [...new Set(values ?? [])].filter((id) => INTERLUDE_AUDIO_PRESET_IDS.has(id));
  return presetIds.length > 0 ? presetIds : [...DEFAULT_AUDIO_VALUE_PRESET_IDS];
}

function normalizeInterludePresetPeriodRange(minMs: number, maxMs: number): PeriodRangeMs {
  const min = Math.min(minMs, maxMs);
  const max = Math.max(minMs, maxMs);
  return {
    minMs: Math.min(INTERLUDE_PRESET_PERIOD_LIMITS.max, Math.max(INTERLUDE_PRESET_PERIOD_LIMITS.min, Math.round(min))),
    maxMs: Math.min(INTERLUDE_PRESET_PERIOD_LIMITS.max, Math.max(INTERLUDE_PRESET_PERIOD_LIMITS.min, Math.round(max))),
  };
}

function buildInterludeDraft(interlude?: Partial<InterludeSnapshot> | null): InterludeConfigDraft {
  const audioMixPickMax = normalizeAudioMixPickMax(
    interlude?.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
  );
  const audioVariationPeriod = normalizeInterludePresetPeriodRange(
    interlude?.audio_variation_period_min_ms ?? 8_000,
    interlude?.audio_variation_period_max_ms ?? 15_000,
  );
  return {
    enabled: interlude?.enabled ?? false,
    directory: interlude?.directory ?? null,
    audioSelectionMode: interlude?.audio_selection_mode === 'fixed' ? 'fixed' : 'random',
    audioFixedPresetId: normalizeInterludeAudioPresetIds([interlude?.audio_fixed_preset_id ?? 'p01'])[0],
    audioPresetIds: normalizeInterludeAudioPresetIds(interlude?.audio_preset_ids),
    audioMixEnabled: interlude?.audio_mix_enabled ?? false,
    audioMixPickMin: normalizeAudioMixPickMin(
      interlude?.audio_mix_pick_min ?? DEFAULT_AUDIO_MIX_PICK_MIN,
      audioMixPickMax,
    ),
    audioMixPickMax,
    audioVariationPeriodMinMs: audioVariationPeriod.minMs,
    audioVariationPeriodMaxMs: audioVariationPeriod.maxMs,
    intervalMinMs: interlude?.interval_min_ms ?? 8_000,
    intervalMaxMs: interlude?.interval_max_ms ?? 13_000,
    volumeDb: interlude?.volume_db ?? 0,
    duckingDepthDb: interlude?.ducking_depth_db ?? -60,
    duckingAttackMs: interlude?.ducking_attack_ms ?? 50,
    duckingReleaseMs: interlude?.ducking_release_ms ?? 250,
  };
}

function getInterludeValidationErrors(draft: InterludeConfigDraft): string[] {
  const errors: string[] = [];
  const inRange = (value: number, min: number, max: number) => Number.isFinite(value) && value >= min && value <= max;

  if (draft.enabled && !draft.directory) errors.push('启用随机插话前必须选择媒体目录');
  if (draft.audioSelectionMode === 'random' && draft.audioPresetIds.length === 0) errors.push('随机音轨至少保留一个声音预设');
  if (draft.audioSelectionMode === 'random' && draft.audioMixEnabled && (
    !inRange(draft.audioMixPickMin, 1, AUDIO_MIX_PICK_HARD_MAX)
    || !inRange(draft.audioMixPickMax, 1, AUDIO_MIX_PICK_HARD_MAX)
    || draft.audioMixPickMin > draft.audioMixPickMax
  )) errors.push('随机多轨范围必须为 1–4，且最少轨数不能大于最多轨数');
  if (draft.audioSelectionMode === 'random' && (
    !inRange(draft.audioVariationPeriodMinMs, INTERLUDE_PRESET_PERIOD_LIMITS.min, INTERLUDE_PRESET_PERIOD_LIMITS.max)
    || !inRange(draft.audioVariationPeriodMaxMs, INTERLUDE_PRESET_PERIOD_LIMITS.min, INTERLUDE_PRESET_PERIOD_LIMITS.max)
    || draft.audioVariationPeriodMinMs > draft.audioVariationPeriodMaxMs
  )) errors.push('变化周期超出允许范围，或最小值大于最大值');
  if (
    !inRange(draft.intervalMinMs, INTERLUDE_LIMITS.intervalMinMs.min, INTERLUDE_LIMITS.intervalMinMs.max)
    || !inRange(draft.intervalMaxMs, INTERLUDE_LIMITS.intervalMinMs.min, INTERLUDE_LIMITS.intervalMinMs.max)
    || draft.intervalMinMs > draft.intervalMaxMs
  ) errors.push('插话声音周期必须为 0.5–60 秒，且最小值不能大于最大值');
  if (!inRange(draft.volumeDb, INTERLUDE_LIMITS.volumeDb.min, INTERLUDE_LIMITS.volumeDb.max)) errors.push('插话音量必须为 -60–12 dB');
  if (!inRange(draft.duckingDepthDb, INTERLUDE_LIMITS.duckingDepthDb.min, INTERLUDE_LIMITS.duckingDepthDb.max)) errors.push('原声压低必须为 -60–0 dB');
  if (!inRange(draft.duckingAttackMs, INTERLUDE_LIMITS.duckingAttackMs.min, INTERLUDE_LIMITS.duckingAttackMs.max)) errors.push('原声压低过渡必须为 0–1000 ms');
  if (!inRange(draft.duckingReleaseMs, INTERLUDE_LIMITS.duckingReleaseMs.min, INTERLUDE_LIMITS.duckingReleaseMs.max)) errors.push('原声恢复过渡必须为 0–3000 ms');
  return errors;
}

function FinalEffectWindow() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const sourceAudioRef = useRef<HTMLAudioElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const processedAudioARef = useRef<HTMLAudioElement | null>(null);
  const processedAudioBRef = useRef<HTMLAudioElement | null>(null);
  const interludeAudioRef = useRef<HTMLAudioElement | null>(null);
  const videoDryGainRef = useRef<GainNode | null>(null);
  const processedSlotGainARef = useRef<GainNode | null>(null);
  const processedSlotGainBRef = useRef<GainNode | null>(null);
  const mainProgramGainRef = useRef<GainNode | null>(null);
  const processedActiveSlotRef = useRef<0 | 1>(0);
  const processedAudioGainTargetRef = useRef<'dry' | 'slot-a' | 'slot-b' | null>(null);
  const processedAudioPlayingRef = useRef(false);
  const processedAudioUrlRef = useRef<string | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const audioContextTransitionRef = useRef<Promise<void> | null>(null);
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
  const mainMediaVolumeGainRef = useRef<GainNode | null>(null);
  const portAudioHardwareRef = useRef(false);
  const audioSourceSyncRef = useRef({
    latestRequest: 0,
    pending: false,
    running: false,
    recoverUnhealthy: false,
    reanchorLoopBoundary: false,
  });
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
  const interludePortAudioRef = useRef(false);
  const interludeStartingRef = useRef(false);
  const webviewInterludeCachePathRef = useRef<string | null>(null);
  const interludeOperationRef = useRef(0);
  const interludeReleaseMsRef = useRef(0);
  const audioDiagnosticsReadyRef = useRef(false);
  const realtimeAudioPlayingRef = useRef(false);
  const loopSourceKeyRef = useRef<string | null>(null);
  const loopGenerationRef = useRef<number | null>(null);
  const loopSequenceRef = useRef(0);
  const sourceAudioLoopIndexRef = useRef<number | null>(null);
  const clockSessionRef = useRef(crypto.randomUUID());
  const clockEpochRef = useRef(Date.now());
  const clockSequenceRef = useRef(0);
  const clockIdentityRef = useRef<string | null>(null);
  const sourceRevisionRef = useRef(0);
  const lastRustPositionSyncAtRef = useRef(0);
  const latestAbsolutePositionMsRef = useRef(0);
  const playbackClockHealthRef = useRef<PlaybackClockHealth>(
    createPlaybackClockHealth('', 0, performance.now()),
  );
  const clockLoadingGraceUntilRef = useRef(0);
  const clockRecoveryRef = useRef<{
    identity: string;
    promise: Promise<void>;
    cancel: () => void;
  } | null>(null);
  const clockFailureHandledIdentityRef = useRef<string | null>(null);
  const loopSyncPromiseRef = useRef<Promise<void> | null>(null);
  const lastRestartTokenRef = useRef<string | number | null>(null);
  // 续播点只属于已经完成 metadata 加载的播放代次，旧 video 的迟到事件不得污染下一项。
  const playbackPositionRef = useRef<PlaybackPositionCheckpoint | null>(null);
  const activeAudioArtifactTimelineRef = useRef<{
    reference: string;
    sourcePath: string;
    timeline: MediaArtifactTimeline;
  } | null>(null);
  const pendingAudioArtifactRef = useRef<{
    reference: string;
    sourcePath: string;
    timeline: MediaArtifactTimeline;
    element: HTMLAudioElement;
    preloaded: boolean;
    activating: boolean;
  } | null>(null);
  const loadedPlaybackGenerationRef = useRef<number | null>(null);
  const [processedAudioUrl, setProcessedAudioUrl] = useState<string | null>(null);
  const [processedAudioReady, setProcessedAudioReady] = useState(false);
  const interludeScheduleKeyRef = useRef<string | null>(null);
  const nextInterludeAtMsRef = useRef<number | null>(null);
  const interludeIntervalStartedAtMsRef = useRef<number | null>(null);
  const lastInterludeIndexRef = useRef<number | null>(null);
  const interludePresetPeriodPlanRef = useRef<InterludePresetPeriodPlan | null>(null);
  const interludeSelectedPathRef = useRef<string | null>(null);
  const interludeSessionSnapshotRef = useRef<InterludeSnapshot | null>(null);
  const interludePortAudioGenerationRef = useRef<number | null>(null);
  const interludePresetSwitchingRef = useRef(false);
  const lastInterludeAudioPresetIdsRef = useRef<string[]>([]);
  const interludeFileCycleRef = useRef(0);
  const interludeRuntimeRef = useRef<InterludeRuntimeMessage>({
    version: 1,
    type: 'interlude-runtime',
    status: 'idle',
    file_cycle: 0,
    preset_segment: 0,
    preset_ids: [],
    file_name: null,
    media_position_ms: 0,
    period_ms: null,
    next_boundary_ms: null,
    file_progress_percent: 0,
    progress_percent: 0,
    output_backend: null,
    error: null,
    started_at_ms: null,
    sent_at_ms: Date.now(),
  });
  const interludeActiveRef = useRef(false);
  const interludePausedRef = useRef(false);
  const interludeStopTimerRef = useRef<number | null>(null);
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [fixedSpeechActive, setFixedSpeechActive] = useState(false);
  const [interludeAudioUrl, setInterludeAudioUrl] = useState<string | null>(null);
  const snapshotRef = useRef<PlaybackSnapshot | null>(null);
  const finalEffectResizeKeyRef = useRef<string | null>(null);
  const finalEffectResizeQueueRef = useRef<Promise<void>>(Promise.resolve());
  const [audioDiagnosticsReady, setAudioDiagnosticsReady] = useState(false);
  const [portAudioHardwareEnabled, setPortAudioHardwareEnabled] = useState(false);
  const [realtimeAudioPlaying, setRealtimeAudioPlaying] = useState(false);
  const [userMuted, setUserMuted] = useState(false);
  const [userVolume, setUserVolume] = useState(1);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [finalEffectResizeError, setFinalEffectResizeError] = useState<string | null>(null);
  const documentVisible = useDocumentVisibility();
  const finalEffectPollingActive = documentVisible && snapshot?.playback_state === 'playing';
  const currentMediaIsAudio = snapshot?.source_media?.media_kind === 'audio';
  const portAudioSourcePath = null;
  const finalEffectMediaIdentity = buildFinalEffectMediaIdentity({
    playbackGeneration: snapshot?.playback_generation,
    sourceIdentity: sourceUrl,
  });
  const finalEffectLoadIdentity = `${finalEffectMediaIdentity ?? ''}:${sourceUrl ?? ''}`;

  function clearVideoElement(element: HTMLVideoElement) {
    element.pause();
    element.muted = true;
    element.removeAttribute('src');
    element.load();
  }

  function clearProcessedAudioSlot(element: HTMLAudioElement) {
    element.pause();
    element.removeAttribute('src');
    element.load();
  }

  function releaseCommittedAudioSlot(element: HTMLAudioElement, reference: string | null) {
    clearProcessedAudioSlot(element);
    if (!reference) return;
    void invoke('release_audio_media_candidate', {
      request: { path: reference },
    }).catch(() => undefined);
  }

  function releaseStoppedMediaArtifacts(currentSnapshot: PlaybackSnapshot) {
    if (videoRef.current) clearVideoElement(videoRef.current);
    sourceAudioRef.current?.pause();
    for (const slot of [processedAudioARef.current, processedAudioBRef.current]) {
      if (slot) clearProcessedAudioSlot(slot);
    }
    pendingAudioArtifactRef.current = null;
    activeAudioArtifactTimelineRef.current = null;
    processedAudioPlayingRef.current = false;
    processedAudioUrlRef.current = null;
    setProcessedAudioUrl(null);
    setProcessedAudioReady(false);

    const audioReferences = new Set([
      currentSnapshot.current_audio_artifact_reference,
      currentSnapshot.pending_audio_artifact_reference,
    ]);
    for (const reference of audioReferences) {
      if (!reference) continue;
      void invoke('release_audio_media_candidate', {
        request: { path: reference },
      }).catch(() => undefined);
    }
  }

  function currentProcessedAudioTimeSeconds(element: HTMLAudioElement): number {
    const active = activeAudioArtifactTimelineRef.current;
    const localMs = active
      ? mapAbsolutePositionToCandidateTime(
          latestAbsolutePositionMsRef.current,
          active.timeline.targetAbsolutePositionMs,
        )
      : null;
    const requested = localMs === null ? 0 : localMs / 1_000;
    return Number.isFinite(element.duration) && element.duration > 0
      ? Math.min(Math.max(0, requested), Math.max(0, element.duration - 0.05))
      : Math.max(0, requested);
  }

  function isCurrentFinalEffectVideo(
    video: HTMLVideoElement,
    expectedIdentity: string | null,
  ): boolean {
    const currentSnapshot = snapshotRef.current;
    const currentSourceIdentity = playbackVideoUrl(currentSnapshot);
    const sourceIdentityElement = currentSnapshot?.source_media?.media_kind === 'video'
      ? sourceAudioRef.current
      : video;
    return video === videoRef.current
      && isFinalEffectMediaIdentityCurrent(expectedIdentity, {
      playbackGeneration: currentSnapshot?.playback_generation,
      sourceIdentity: currentSourceIdentity,
    })
      && currentSourceIdentity !== null
      && sourceIdentityElement?.getAttribute('src') === currentSourceIdentity;
  }

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
    snapshotRef.current = nextSnapshot;
    const currentAudioTimeline = currentAudioArtifactTimeline(nextSnapshot);
    if (
      nextSnapshot.current_audio_artifact_reference
      && nextSnapshot.source_media?.source_path
      && currentAudioTimeline
    ) {
      const active = activeAudioArtifactTimelineRef.current;
      if (
        !active
        || active.reference !== nextSnapshot.current_audio_artifact_reference
        || active.timeline.planId !== currentAudioTimeline.planId
        || active.timeline.sequence !== currentAudioTimeline.sequence
      ) {
        activeAudioArtifactTimelineRef.current = {
          reference: nextSnapshot.current_audio_artifact_reference,
          sourcePath: nextSnapshot.source_media.source_path,
          timeline: currentAudioTimeline,
        };
      }
    } else {
      activeAudioArtifactTimelineRef.current = null;
    }
    setSnapshot(nextSnapshot);
    syncWebAudioContextState();
    const video = videoRef.current;
    if (nextSnapshot.playback_state === 'playing') {
      if (video?.paused) {
        void video.play().catch(() => {
          setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击播放区域继续。');
        });
      }
      if (sourceAudioRef.current?.paused) {
        void sourceAudioRef.current.play().catch(() => undefined);
      }
    }
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
    const user = clampVolume(userVolumeRef.current);
    const targetGain = (live ? 10 ** (totalDb / 20) : 1) * user;
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

  function scheduleMainProgramDuck(targetGain: number, durationMs: number) {
    const context = audioContextRef.current;
    const mainProgramGain = mainProgramGainRef.current;
    if (!context || !mainProgramGain) return;
    const now = context.currentTime;
    const durationSeconds = Math.max(0, durationMs) / 1_000;
    mainProgramGain.gain.cancelScheduledValues(now);
    mainProgramGain.gain.setValueAtTime(mainProgramGain.gain.value, now);
    if (durationSeconds === 0) {
      mainProgramGain.gain.setValueAtTime(clampVolume(targetGain), now);
    } else {
      mainProgramGain.gain.linearRampToValueAtTime(clampVolume(targetGain), now + durationSeconds);
    }
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
    const sourceAudio = sourceAudioRef.current;
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
    if (mainMediaVolumeGainRef.current) {
      mainMediaVolumeGainRef.current.gain.value = hardwareOut || muted ? 0 : clampVolume(volume);
    }
    if (speakerMuteGainRef.current) {
      // 硬件出口接管时静音整个 Web Audio 扬声器；用户媒体静音只作用主媒体总线。
      speakerMuteGainRef.current.gain.value = hardwareOut ? 0 : 1;
    }
    if (processedActive) {
      if (videoFxReverbWetRef.current) videoFxReverbWetRef.current.gain.value = 0;
      if (videoFxReverbFeedbackRef.current) videoFxReverbFeedbackRef.current.gain.value = 0;
      if (videoFxNoiseGainRef.current) videoFxNoiseGainRef.current.gain.value = 0;
    }
    if (video) {
      video.volume = 0;
      video.muted = true;
    }
    if (sourceAudio) {
      sourceAudio.volume = graphOwnsVideo
        ? 1
        : clampVolume(volume * (interludeActiveRef.current ? duckGainLevelRef.current : 1));
      sourceAudio.muted = hardwareOut
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
      // WebView 模式由媒体元素出声；PortAudio 模式只把它当结束时钟，避免双播。
      interludeAudio.volume = clampVolume(interludeGainLevelRef.current);
      interludeAudio.muted = interludePortAudioRef.current;
    }
    applyRealtimeVideoFx(
      runtimeAudioParamsRef.current,
      runtimeAudioEnabledRef.current && !replacementAudioActive && !processedActive && !(muted && !hardwareOut),
    );
  }

  function syncPortAudioMediaVolume() {
    if (!portAudioHardwareRef.current) return;
    void invoke<void>('set_portaudio_media_volume', {
      request: {
        volume: clampVolume(userVolumeRef.current),
        muted: userMutedRef.current,
      },
    }).catch((cause) => {
      setPlaybackError(getDisplayErrorMessage(cause, 'PortAudio 媒体音量更新失败'));
    });
  }

  function syncWebAudioContextState() {
    const context = audioContextRef.current;
    if (!context || context.state === 'closed' || audioContextTransitionRef.current) return;
    const transition = (async () => {
      while (audioContextRef.current === context && context.state !== 'closed') {
        // video 已接入 MediaElementSourceNode，挂起图会连带冻结媒体时钟。
        // PortAudio 接管时只把 speakerMuteGain 拉到 0，播放期间仍保持图运行。
        const shouldRun = snapshotRef.current?.playback_state === 'playing';
        if (shouldRun && context.state !== 'running') {
          await context.resume();
        } else if (!shouldRun && context.state !== 'suspended') {
          await context.suspend();
        }
        const stillShouldRun = snapshotRef.current?.playback_state === 'playing';
        if (stillShouldRun === shouldRun) break;
      }
    })().catch(() => undefined);
    audioContextTransitionRef.current = transition;
    void transition.finally(() => {
      if (audioContextTransitionRef.current !== transition) return;
      audioContextTransitionRef.current = null;
      if (audioContextRef.current !== context || context.state === 'closed') return;
      const shouldRun = snapshotRef.current?.playback_state === 'playing';
      if ((shouldRun && context.state !== 'running') || (!shouldRun && context.state !== 'suspended')) {
        syncWebAudioContextState();
      }
    });
  }

  function setPortAudioHardwareActive(requested: boolean) {
    const nextActive = requested && PORTAUDIO_FORMAL_SOURCE_SYNC_READY;
    const previousActive = portAudioHardwareRef.current;
    if (portAudioHardwareRef.current === nextActive) {
      syncUserAudioSettings();
      syncWebAudioContextState();
      return;
    }
    if (nextActive && !previousActive && interludeActiveRef.current && !interludePortAudioRef.current) {
      clearInterludePlayback({ releaseMs: 0, resetSchedule: false });
      const currentClockMs = performance.now();
      interludeIntervalStartedAtMsRef.current = currentClockMs;
      nextInterludeAtMsRef.current = currentClockMs;
    }
    if (!nextActive && previousActive && interludePortAudioRef.current) {
      interludePortAudioRef.current = false;
    }
    portAudioHardwareRef.current = nextActive;
    setPortAudioHardwareEnabled(nextActive);
    syncUserAudioSettings();
    syncPortAudioMediaVolume();
    syncWebAudioContextState();
  }

  function clearInterludeStopTimer() {
    if (interludeStopTimerRef.current !== null) {
      window.clearTimeout(interludeStopTimerRef.current);
      interludeStopTimerRef.current = null;
      const interludeAudio = interludeAudioRef.current;
      if (interludeAudio) {
        interludeAudio.pause();
        interludeAudio.currentTime = 0;
        interludeAudio.removeAttribute('src');
        interludeAudio.load();
      }
      interludeAudioUrlRef.current = null;
      setInterludeAudioUrl(null);
      releaseCurrentWebViewInterludeCache();
    }
  }

  function releaseWebViewInterludeCache(processedPath: string) {
    void invoke<unknown>('release_webview_interlude_cache', {
      request: { path: processedPath },
    }).catch(() => undefined);
  }

  function releaseCurrentWebViewInterludeCache() {
    const processedPath = webviewInterludeCachePathRef.current;
    webviewInterludeCachePathRef.current = null;
    if (processedPath) releaseWebViewInterludeCache(processedPath);
  }

  function shutdownInterludePlayback() {
    const shouldStopPortAudio = interludePortAudioRef.current || interludeStartingRef.current;
    interludeOperationRef.current += 1;
    if (shouldStopPortAudio) {
      void invoke<void>('stop_portaudio_interlude').catch(() => undefined);
    }
    if (interludeStopTimerRef.current !== null) {
      window.clearTimeout(interludeStopTimerRef.current);
      interludeStopTimerRef.current = null;
    }
    const interludeAudio = interludeAudioRef.current;
    if (interludeAudio) {
      interludeAudio.pause();
      interludeAudio.removeAttribute('src');
      interludeAudio.load();
    }
    interludeAudioUrlRef.current = null;
    interludeStartingRef.current = false;
    interludePortAudioRef.current = false;
    interludeActiveRef.current = false;
    interludePausedRef.current = false;
    interludeReleaseMsRef.current = 0;
    interludeGainLevelRef.current = 0;
    interludePresetPeriodPlanRef.current = null;
    interludeSelectedPathRef.current = null;
    interludeSessionSnapshotRef.current = null;
    interludePortAudioGenerationRef.current = null;
    interludePresetSwitchingRef.current = false;
    duckGainLevelRef.current = 1;
    scheduleMainProgramDuck(1, 0);
    syncUserAudioSettings();
    nextInterludeAtMsRef.current = null;
    interludeIntervalStartedAtMsRef.current = null;
    releaseCurrentWebViewInterludeCache();
    interludeFileCycleRef.current = 0;
    publishInterludeRuntime('idle');
  }

  function clearInterludePlayback(options?: {
    releaseMs?: number;
    resetSchedule?: boolean;
    resetIndex?: boolean;
    clearSource?: boolean;
  }) {
    const releaseMs = options?.releaseMs ?? interludeReleaseMsRef.current;
    const resetSchedule = options?.resetSchedule ?? true;
    const resetIndex = options?.resetIndex ?? false;
    const clearSource = options?.clearSource ?? true;
    const shouldStopPortAudio = interludePortAudioRef.current || interludeStartingRef.current;
    interludeOperationRef.current += 1;
    interludeStartingRef.current = false;
    interludePortAudioRef.current = false;
    interludeReleaseMsRef.current = 0;
    clearInterludeStopTimer();
    interludeActiveRef.current = false;
    interludePausedRef.current = false;
    interludeGainLevelRef.current = 0;
    interludePresetPeriodPlanRef.current = null;
    interludeSelectedPathRef.current = null;
    interludeSessionSnapshotRef.current = null;
    interludePortAudioGenerationRef.current = null;
    interludePresetSwitchingRef.current = false;
    duckGainLevelRef.current = 1;
    scheduleMainProgramDuck(1, releaseMs);
    syncUserAudioSettings();
    publishInterludeRuntime('idle');
    if (shouldStopPortAudio) {
      void invoke<void>('stop_portaudio_interlude').catch(() => undefined);
    }
    if (resetSchedule) {
      nextInterludeAtMsRef.current = null;
      interludeIntervalStartedAtMsRef.current = null;
    }
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
      releaseCurrentWebViewInterludeCache();
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
    if (interludePortAudioRef.current) {
      void invoke<void>('pause_portaudio_interlude').catch((cause) => {
        setPlaybackError(getDisplayErrorMessage(cause, 'PortAudio 插话暂停失败'));
      });
    }
  }

  function resumeInterludePlayback() {
    if (snapshotRef.current?.playback_state !== 'playing') return;
    if (!interludeActiveRef.current || !interludePausedRef.current || !interludeAudioUrlRef.current) return;
    interludePausedRef.current = false;
    if (interludePortAudioRef.current) {
      void invoke<void>('resume_portaudio_interlude').catch((cause) => {
        setPlaybackError(getDisplayErrorMessage(cause, 'PortAudio 插话恢复失败'));
      });
    }
    void interludeAudioRef.current?.play().catch((cause) => {
      setPlaybackError(getDisplayErrorMessage(cause, '插话结束时钟恢复失败'));
      clearInterludePlayback({ resetSchedule: false });
    });
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
      void invokePlaybackSnapshot('restore_original_audio')
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
    void invokePlaybackSnapshot('restore_original_audio')
      .then(applyPlayerSnapshot)
      .catch(() => undefined);
  }

  function handleInterludeEnded() {
    clearInterludePlayback({
      releaseMs: interludeReleaseMsRef.current,
      resetSchedule: true,
    });
  }

  function publishInterludeRuntime(
    status: InterludeRuntimeMessage['status'],
    options?: {
      presetIds?: readonly string[];
      fileName?: string | null;
      presetSegment?: number;
      mediaPositionMs?: number;
      plan?: InterludePresetPeriodPlan | null;
      fileProgressPercent?: number;
      outputBackend?: InterludeRuntimeMessage['output_backend'];
      error?: string | null;
    },
  ) {
    const previous = interludeRuntimeRef.current;
    const sentAtMs = Date.now();
    const message: InterludeRuntimeMessage = {
      version: 1,
      type: 'interlude-runtime',
      status,
      file_cycle: interludeFileCycleRef.current,
      preset_segment: options?.presetSegment ?? (status === 'idle' ? 0 : previous.preset_segment),
      preset_ids: options?.presetIds ? [...options.presetIds] : status === 'idle' ? [] : previous.preset_ids,
      file_name: options?.fileName === undefined ? status === 'idle' ? null : previous.file_name : options.fileName,
      media_position_ms: Math.max(0, Math.round(options?.mediaPositionMs ?? (status === 'idle' ? 0 : previous.media_position_ms))),
      period_ms: options?.plan === undefined ? status === 'idle' ? null : previous.period_ms : options.plan?.periodMs ?? null,
      next_boundary_ms: options?.plan === undefined ? status === 'idle' ? null : previous.next_boundary_ms : options.plan?.nextBoundaryMs ?? null,
      file_progress_percent: Math.min(100, Math.max(0, options?.fileProgressPercent
        ?? (status === 'idle' ? 0 : previous.file_progress_percent))),
      progress_percent: options?.plan === undefined
        ? status === 'idle' ? 0 : previous.progress_percent
        : interludePresetPeriodProgress(options.plan, options?.mediaPositionMs ?? 0),
      output_backend: options?.outputBackend === undefined ? previous.output_backend : options.outputBackend,
      error: options?.error ?? null,
      started_at_ms: status === 'playing'
        ? previous.status === 'playing' && Number.isSafeInteger(previous.started_at_ms)
          ? previous.started_at_ms
          : sentAtMs
        : null,
      sent_at_ms: sentAtMs,
    };
    interludeRuntimeRef.current = message;
    try {
      playbackChannelRef.current?.postMessage(message);
    } catch {
      // 最终效果窗口关闭时保留本地清理，不阻塞插话资源释放。
    }
  }

  function handleInterludeError(event: SyntheticEvent<HTMLAudioElement>) {
    if (!interludeActiveRef.current) return;
    const detail = event.currentTarget.error?.message;
    const message = detail ? `插话音频播放失败：${detail}` : '插话音频播放失败，请检查文件格式和文件权限。';
    setPlaybackError(message);
    clearInterludePlayback({ resetSchedule: true });
    publishInterludeRuntime('failed', { error: message });
  }

  function sampleInterludeAudioCycle(interlude: InterludeSnapshot): AudioCycleSample {
    const selectionMode = interlude.audio_selection_mode === 'fixed' ? 'fixed' : 'random';
    const fixedPresetId = normalizeInterludeAudioPresetIds([
      interlude.audio_fixed_preset_id ?? 'p01',
    ])[0];
    const presetIds = normalizeInterludeAudioPresetIds(interlude.audio_preset_ids);
    if (selectionMode === 'fixed') {
      const sample = sampleAudioCycle([fixedPresetId], {
        mixEnabled: false,
        pickMin: 1,
        pickMax: 1,
      });
      lastInterludeAudioPresetIdsRef.current = [...sample.presetIds];
      return sample;
    }

    const sample = sampleAudioCycle(presetIds, {
      mixEnabled: interlude.audio_mix_enabled ?? false,
      pickMin: interlude.audio_mix_pick_min ?? DEFAULT_AUDIO_MIX_PICK_MIN,
      pickMax: interlude.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
      previousPresetIds: lastInterludeAudioPresetIdsRef.current,
    });
    lastInterludeAudioPresetIdsRef.current = [...sample.presetIds];
    return sample;
  }

  function resolveInterludeAudioCycle(interlude: InterludeSnapshot): AudioCycleSample {
    const sample = sampleInterludeAudioCycle(interlude);
    const selectionMode = interlude.audio_selection_mode === 'fixed' ? 'fixed' : 'random';
    const periodic = selectionMode === 'random';
    const period = normalizeInterludePresetPeriodRange(
      interlude.audio_variation_period_min_ms ?? 8_000,
      interlude.audio_variation_period_max_ms ?? 15_000,
    );
    interludePresetPeriodPlanRef.current = periodic
      ? createInterludePresetPeriodPlan(0, period.minMs, period.maxMs)
      : null;
    return sample;
  }

  async function startInterludePlayback(interlude: InterludeSnapshot) {
    if (interludeActiveRef.current || interludeStartingRef.current) return;
    const count = interlude.audio_files.length;
    const nextIndex = chooseInterludeIndex(count, lastInterludeIndexRef.current);
    if (nextIndex === null) return;
    const selectedPath = interlude.audio_files[nextIndex];
    const operation = interludeOperationRef.current + 1;
    interludeOperationRef.current = operation;
    interludeStartingRef.current = true;
    clearInterludeStopTimer();
    lastInterludeIndexRef.current = nextIndex;
    nextInterludeAtMsRef.current = null;
    interludeIntervalStartedAtMsRef.current = null;
    const audioCycle = resolveInterludeAudioCycle(interlude);
    const selectedFileName = interludeFileNameFromPath(selectedPath);
    interludeSelectedPathRef.current = selectedPath;
    interludeSessionSnapshotRef.current = interlude;
    publishInterludeRuntime('starting', {
      presetIds: audioCycle.presetIds,
      fileName: selectedFileName,
      presetSegment: 1,
      mediaPositionMs: 0,
      plan: interludePresetPeriodPlanRef.current,
      fileProgressPercent: 100,
      outputBackend: portAudioHardwareRef.current ? 'portaudio' : 'webview',
    });
    const audio = {
      ...audioCycle.values,
      current_formant_hz: null,
    };
    const audioVariants = interlude.audio_selection_mode !== 'fixed'
      && interlude.audio_mix_enabled
      && audioCycle.variants.length > 1
      ? buildAudioVariantsFromCycle(audio, audioCycle)
      : [];
    const usePortAudio = portAudioHardwareRef.current;
    const videoSource = isSupportedInterludeVideoSource(selectedPath);
    let playbackPath = selectedPath;
    let processedPath: string | null = null;
    if (!usePortAudio || videoSource) {
      try {
        const preparedResponse = await invoke<unknown>('prepare_webview_interlude', {
          request: {
            path: selectedPath,
            audio,
            audio_variants: audioVariants,
          },
        });
        if (!isPrepareWebViewInterludeResult(preparedResponse)) {
          throw new Error('WebView 插话处理返回了无效结果');
        }
        const prepared = preparedResponse;
        if (prepared.state === 'processed') {
          playbackPath = prepared.path;
          processedPath = prepared.path;
        } else if (prepared.state === 'original_fallback') {
          if (videoSource) throw new Error(prepared.reason ?? '视频插话音轨提取失败');
          playbackPath = prepared.path;
          if (prepared.reason) setPlaybackError(`${prepared.reason}；已回退插话原声。`);
        }
      } catch (cause) {
        if (videoSource) {
          if (operation === interludeOperationRef.current) {
            interludeStartingRef.current = false;
            const message = `${getDisplayErrorMessage(cause, '视频插话音轨提取失败')}；主音轨保持播放。`;
            setPlaybackError(message);
            publishInterludeRuntime('failed', {
              presetIds: audioCycle.presetIds,
              outputBackend: usePortAudio ? 'portaudio' : 'webview',
              error: message,
            });
          }
          return;
        }
        if (operation === interludeOperationRef.current) {
          setPlaybackError(`${getDisplayErrorMessage(cause, 'WebView 插话本地处理失败')}；已回退插话原声。`);
        }
      }
    }
    if (operation !== interludeOperationRef.current) {
      if (processedPath) releaseWebViewInterludeCache(processedPath);
      return;
    }
    if (usePortAudio) {
      try {
        const startedResponse = await invoke<unknown>('start_portaudio_interlude', {
          request: {
            path: selectedPath,
            audio,
            audio_variants: audioVariants,
          },
        });
        if (!isStartPortAudioInterludeResult(startedResponse)) {
          throw new Error('PortAudio 插话启动返回了无效会话代际');
        }
        interludePortAudioGenerationRef.current = startedResponse.generation;
      } catch (cause) {
        if (processedPath) releaseWebViewInterludeCache(processedPath);
        if (operation === interludeOperationRef.current) {
          interludeStartingRef.current = false;
          const message = `${getDisplayErrorMessage(cause, 'PortAudio 插话解码或混音失败')}；主音轨保持播放。`;
          setPlaybackError(message);
          publishInterludeRuntime('failed', { presetIds: audioCycle.presetIds, outputBackend: 'portaudio', error: message });
        }
        return;
      }
    }
    const selectedUrl = toAssetUrl(playbackPath);
    if (!selectedUrl) {
      if (usePortAudio) void invoke<void>('stop_portaudio_interlude').catch(() => undefined);
      if (processedPath) releaseWebViewInterludeCache(processedPath);
      interludeStartingRef.current = false;
      const message = '插话音频路径无法转换为安全的本地资源地址，主音轨保持播放。';
      setPlaybackError(message);
      publishInterludeRuntime('failed', {
        presetIds: audioCycle.presetIds,
        outputBackend: usePortAudio ? 'portaudio' : 'webview',
        error: message,
      });
      return;
    }
    if (operation !== interludeOperationRef.current) {
      if (usePortAudio) void invoke<void>('stop_portaudio_interlude').catch(() => undefined);
      if (processedPath) releaseWebViewInterludeCache(processedPath);
      return;
    }
    releaseCurrentWebViewInterludeCache();
    webviewInterludeCachePathRef.current = processedPath;
    interludeStartingRef.current = false;
    interludePortAudioRef.current = usePortAudio;
    interludeActiveRef.current = true;
    interludePausedRef.current = false;
    interludeGainLevelRef.current = toGainValue(interlude.volume_db);
    duckGainLevelRef.current = toGainValue(interlude.ducking_depth_db);
    interludeReleaseMsRef.current = interlude.ducking_release_ms;
    scheduleMainProgramDuck(duckGainLevelRef.current, interlude.ducking_attack_ms);
    const interludeAudio = interludeAudioRef.current;
    if (interludeAudio) {
      interludeAudio.playbackRate = resolveInterludeClockPlaybackRate(
        audio.playback_speed,
        usePortAudio && !processedPath,
      );
    }
    setInterludeAudioUrl(selectedUrl);
    interludeFileCycleRef.current += 1;
    publishInterludeRuntime('playing', {
      presetIds: audioCycle.presetIds,
      fileName: selectedFileName,
      presetSegment: 1,
      mediaPositionMs: 0,
      plan: interludePresetPeriodPlanRef.current,
      fileProgressPercent: 100,
      outputBackend: usePortAudio ? 'portaudio' : 'webview',
    });
    syncUserAudioSettings();
  }

  function currentInterludeMediaPositionMs() {
    const audio = interludeAudioRef.current;
    return audio && Number.isFinite(audio.currentTime)
      ? Math.max(0, Math.round(audio.currentTime * 1_000))
      : 0;
  }

  async function switchActiveInterludePreset(
    interlude: InterludeSnapshot,
    mediaPositionMs: number,
    nextPlan: InterludePresetPeriodPlan,
  ) {
    if (interludePresetSwitchingRef.current) return;
    const selectedPath = interludeSelectedPathRef.current;
    if (!selectedPath) return;
    interludePresetSwitchingRef.current = true;
    const operation = interludeOperationRef.current;
    const audioCycle = sampleInterludeAudioCycle(interlude);
    const audio = {
      ...audioCycle.values,
      current_formant_hz: null,
    };
    const audioVariants = interlude.audio_mix_enabled && audioCycle.variants.length > 1
      ? buildAudioVariantsFromCycle(audio, audioCycle)
      : [];
    let processedPath: string | null = null;
    let webviewPrevious: { element: HTMLAudioElement; url: string; positionSeconds: number } | null = null;
    try {
      if (interludePortAudioRef.current) {
        const generation = interludePortAudioGenerationRef.current;
        if (generation === null) throw new Error('PortAudio 插话会话代际缺失');
        await invoke<void>('switch_portaudio_interlude_preset', {
          request: {
            generation,
            audio,
            audio_variants: audioVariants,
            media_position_ms: mediaPositionMs,
          },
        });
        const interludeAudio = interludeAudioRef.current;
        if (operation === interludeOperationRef.current && interludeAudio) {
          interludeAudio.playbackRate = resolveInterludeClockPlaybackRate(
            audio.playback_speed,
            webviewInterludeCachePathRef.current === null,
          );
        }
      } else {
        const preparedResponse = await invoke<unknown>('prepare_webview_interlude', {
          request: {
            path: selectedPath,
            audio,
            audio_variants: audioVariants,
          },
        });
        if (!isPrepareWebViewInterludeResult(preparedResponse)) {
          throw new Error('WebView 插话预设处理返回了无效结果');
        }
        if (preparedResponse.state !== 'processed') {
          throw new Error(preparedResponse.reason ?? '插话预设候选处理失败');
        }
        if (preparedResponse.state === 'processed') processedPath = preparedResponse.path;
        const nextPath = preparedResponse.path;
        const nextUrl = toAssetUrl(nextPath);
        if (!nextUrl) throw new Error('插话预设候选路径无法转换为安全资源地址');
        if (operation !== interludeOperationRef.current) {
          if (processedPath) releaseWebViewInterludeCache(processedPath);
          return;
        }
        const element = interludeAudioRef.current;
        if (!element) throw new Error('插话播放元素已释放');
        const resumeAt = Math.max(mediaPositionMs / 1_000, element.currentTime);
        const candidate = document.createElement('audio');
        candidate.crossOrigin = 'anonymous';
        candidate.preload = 'auto';
        candidate.muted = true;
        candidate.src = nextUrl;
        await new Promise<void>((resolve, reject) => {
          const finish = (error?: Error) => {
            window.clearTimeout(timeout);
            candidate.removeEventListener('loadedmetadata', handleMetadata);
            candidate.removeEventListener('canplay', handleReady);
            candidate.removeEventListener('error', handleError);
            if (error) reject(error); else resolve();
          };
          const handleMetadata = () => {
            candidate.currentTime = Math.min(
              resumeAt,
              Number.isFinite(candidate.duration) ? candidate.duration : resumeAt,
            );
          };
          const handleReady = () => finish();
          const handleError = () => finish(new Error(candidate.error?.message ?? '插话预设候选预加载失败'));
          const timeout = window.setTimeout(() => finish(new Error('插话预设候选预加载超时')), 10_000);
          candidate.addEventListener('loadedmetadata', handleMetadata, { once: true });
          candidate.addEventListener('canplay', handleReady, { once: true });
          candidate.addEventListener('error', handleError, { once: true });
          candidate.load();
        });
        candidate.removeAttribute('src');
        candidate.load();
        webviewPrevious = {
          element,
          url: interludeAudioUrlRef.current ?? element.currentSrc,
          positionSeconds: resumeAt,
        };
        element.pause();
        element.src = nextUrl;
        element.load();
        await new Promise<void>((resolve, reject) => {
          const timeout = window.setTimeout(() => finish(new Error('插话预设候选加载超时')), 10_000);
          const finish = (error?: Error) => {
            window.clearTimeout(timeout);
            element.removeEventListener('loadedmetadata', handleLoaded);
            element.removeEventListener('error', handleError);
            if (error) reject(error); else resolve();
          };
          const handleLoaded = () => finish();
          const handleError = () => finish(new Error(element.error?.message ?? '插话预设候选加载失败'));
          element.addEventListener('loadedmetadata', handleLoaded, { once: true });
          element.addEventListener('error', handleError, { once: true });
        });
        element.currentTime = Math.min(resumeAt, Number.isFinite(element.duration) ? element.duration : resumeAt);
        interludeAudioUrlRef.current = nextUrl;
        setInterludeAudioUrl(nextUrl);
        syncUserAudioSettings();
        await element.play();
        const previousCache = webviewInterludeCachePathRef.current;
        webviewInterludeCachePathRef.current = processedPath;
        if (previousCache && previousCache !== processedPath) releaseWebViewInterludeCache(previousCache);
      }
      if (operation !== interludeOperationRef.current) return;
      interludePresetPeriodPlanRef.current = nextPlan;
      publishInterludeRuntime('playing', {
        presetIds: audioCycle.presetIds,
        fileName: interludeFileNameFromPath(selectedPath),
        presetSegment: nextPlan.segment,
        mediaPositionMs: currentInterludeMediaPositionMs(),
        plan: nextPlan,
        outputBackend: interludePortAudioRef.current ? 'portaudio' : 'webview',
      });
    } catch (cause) {
      if (processedPath) releaseWebViewInterludeCache(processedPath);
      if (webviewPrevious && operation === interludeOperationRef.current) {
        const { element, url, positionSeconds } = webviewPrevious;
        if (url) {
          element.src = url;
          element.load();
          const restore = () => {
            element.currentTime = Math.min(
              positionSeconds,
              Number.isFinite(element.duration) ? element.duration : positionSeconds,
            );
            syncUserAudioSettings();
            void element.play().catch(() => undefined);
          };
          if (element.readyState >= 1) restore();
          else element.addEventListener('loadedmetadata', restore, { once: true });
        }
      }
      if (operation === interludeOperationRef.current) {
        interludePresetPeriodPlanRef.current = nextPlan;
        const message = `${getDisplayErrorMessage(cause, '插话预设切换失败')}；继续使用当前预设。`;
        setPlaybackError(message);
        publishInterludeRuntime('playing', {
          mediaPositionMs,
          plan: nextPlan,
          error: message,
        });
      }
    } finally {
      interludePresetSwitchingRef.current = false;
    }
  }

  function updateActiveInterludePresetPeriod() {
    if (!interludeActiveRef.current || interludePausedRef.current) return;
    const interlude = snapshotRef.current?.interlude ?? interludeSessionSnapshotRef.current;
    const selectedPath = interludeSelectedPathRef.current;
    if (!interlude || !selectedPath || interlude.audio_selection_mode === 'fixed') return;
    const mediaPositionMs = currentInterludeMediaPositionMs();
    const period = normalizeInterludePresetPeriodRange(
      interlude.audio_variation_period_min_ms ?? 8_000,
      interlude.audio_variation_period_max_ms ?? 15_000,
    );
    let plan = interludePresetPeriodPlanRef.current;
    if (!plan) {
      plan = createInterludePresetPeriodPlan(mediaPositionMs, period.minMs, period.maxMs);
      interludePresetPeriodPlanRef.current = plan;
    }
    publishInterludeRuntime('playing', {
      fileName: interludeFileNameFromPath(selectedPath),
      presetSegment: plan.segment,
      mediaPositionMs,
      plan,
    });
    const nextPlan = advanceInterludePresetPeriodPlan(
      plan,
      mediaPositionMs,
      period.minMs,
      period.maxMs,
    );
    if (nextPlan) void switchActiveInterludePreset(interlude, mediaPositionMs, nextPlan);
  }

  function tryActivatePendingAudioArtifact(absolutePositionMs: number) {
    const pending = pendingAudioArtifactRef.current;
    const currentSnapshot = snapshotRef.current;
    const currentIdentity = pendingAudioArtifactTimeline(currentSnapshot);
    const sourceDurationMs = currentSnapshot?.source_media?.duration_ms;
    const candidateDurationMs = pending?.element && Number.isFinite(pending.element.duration)
      ? Math.max(0, Math.round(pending.element.duration * 1_000))
      : 0;
    const localPositionMs = pending && currentIdentity
      ? resolveMediaArtifactSwitchTime(
          pending.timeline,
          currentIdentity,
          absolutePositionMs,
          candidateDurationMs,
          sourceDurationMs ?? undefined,
          currentSnapshot?.source_media_pool.length === 1,
        )
      : null;
    if (
      pending
      && currentSnapshot
      && currentIdentity
      && !pending.activating
      && currentSnapshot.playback_generation === pending.timeline.playbackGeneration
      && currentSnapshot.source_media?.source_path === pending.sourcePath
      && absolutePositionMs >= pending.timeline.validUntilAbsolutePositionMs
    ) {
      pending.activating = true;
      pendingAudioArtifactRef.current = null;
      clearProcessedAudioSlot(pending.element);
      void invokePlaybackSnapshot('discard_audio_media_candidate', {
        request: {
          plan_id: pending.timeline.planId,
          sequence: pending.timeline.sequence,
          playback_generation: pending.timeline.playbackGeneration,
          source_revision: pending.timeline.sourceRevision,
          reason: '候选声音已超过有效媒体时间窗口，跳过本轮并准备下一轮',
        },
      }).then(applyPlayerSnapshot).catch(() => undefined);
      return;
    }
    if (
      !pending
      || !pending.preloaded
      || pending.activating
      || !currentSnapshot
      || !currentIdentity
      || currentSnapshot.playback_generation !== pending.timeline.playbackGeneration
      || currentSnapshot.source_media?.source_path !== pending.sourcePath
      || localPositionMs === null
    ) return;
    pending.activating = true;
    const standby = pending.element;
    const standbySlot: 0 | 1 = standby === processedAudioARef.current ? 0 : 1;
    let committing = false;
    let resyncAttempts = 0;
    let confirmationTimer: number | null = null;
    const removeConfirmationListeners = () => {
      standby.removeEventListener('playing', confirmCandidatePlayback);
      standby.removeEventListener('timeupdate', confirmCandidatePlayback);
      if (confirmationTimer !== null) {
        window.clearTimeout(confirmationTimer);
        confirmationTimer = null;
      }
    };
    const failStandby = (cause?: unknown) => {
      removeConfirmationListeners();
      if (pendingAudioArtifactRef.current !== pending) return;
      pendingAudioArtifactRef.current = null;
      clearProcessedAudioSlot(standby);
      const reason = cause
        ? getDisplayErrorMessage(cause, '候选声音无法启动，继续播放当前声音')
        : '候选声音未通过切换门禁，继续播放当前声音';
      setPlaybackError(reason);
      void invokePlaybackSnapshot('discard_audio_media_candidate', {
        request: {
          plan_id: pending.timeline.planId,
          sequence: pending.timeline.sequence,
          playback_generation: pending.timeline.playbackGeneration,
          source_revision: pending.timeline.sourceRevision,
          reason,
        },
      }).then(applyPlayerSnapshot).catch(() => undefined);
    };
    const confirmCandidatePlayback = () => {
      if (committing || standby.paused || pendingAudioArtifactRef.current !== pending) return;
      const latestSnapshot = snapshotRef.current;
      const latestIdentity = pendingAudioArtifactTimeline(latestSnapshot);
      const latestAbsolutePositionMs = latestAbsolutePositionMsRef.current;
      const actualLocalPositionMs = Math.max(0, Math.round(standby.currentTime * 1_000));
      const latestDurationMs = Number.isFinite(standby.duration)
        ? Math.max(0, Math.round(standby.duration * 1_000))
        : 0;
      const expectedLocalPositionMs = latestIdentity
        ? resolveMediaArtifactSwitchTime(
            pending.timeline,
            latestIdentity,
            latestAbsolutePositionMs,
            latestDurationMs,
            latestSnapshot?.source_media?.duration_ms ?? undefined,
            latestSnapshot?.source_media_pool.length === 1,
          )
        : null;
      if (
        latestSnapshot?.playback_state !== 'playing'
        || !latestIdentity
        || expectedLocalPositionMs === null
      ) return;
      const driftMs = Math.abs(actualLocalPositionMs - expectedLocalPositionMs);
      if (driftMs > MEDIA_ARTIFACT_SWITCH_TOLERANCE_MS) {
        if (resyncAttempts >= 2) return;
        resyncAttempts += 1;
        try {
          standby.currentTime = expectedLocalPositionMs / 1_000;
        } catch {
          // 当前 N 继续播放，等待下一次 timeupdate 再确认同步。
        }
        return;
      }
      committing = true;
      removeConfirmationListeners();
      const oldActive = processedAudioPlayingRef.current
        ? (processedActiveSlotRef.current === 0
            ? processedAudioARef.current
            : processedAudioBRef.current)
        : null;
      const oldReference = latestSnapshot.current_audio_artifact_reference;
      void invokePlaybackSnapshot('commit_audio_media_candidate', {
        request: {
          plan_id: pending.timeline.planId,
          sequence: pending.timeline.sequence,
          playback_generation: pending.timeline.playbackGeneration,
          source_revision: pending.timeline.sourceRevision,
        },
      }).then((nextSnapshot) => {
        const committedIdentity = currentAudioArtifactTimeline(nextSnapshot);
        if (
          pendingAudioArtifactRef.current !== pending
          || nextSnapshot.current_audio_artifact_reference !== pending.reference
          || !committedIdentity
          || !isMediaArtifactIdentityCurrent(pending.timeline, committedIdentity)
        ) {
          failStandby(new Error('处理后声音提交身份不一致'));
          return;
        }
        pendingAudioArtifactRef.current = null;
        const anchorAbsolutePositionMs = latestAbsolutePositionMs - actualLocalPositionMs;
        activeAudioArtifactTimelineRef.current = {
          reference: pending.reference,
          sourcePath: pending.sourcePath,
          timeline: {
            ...pending.timeline,
            targetAbsolutePositionMs: anchorAbsolutePositionMs,
            validUntilAbsolutePositionMs: anchorAbsolutePositionMs + latestDurationMs,
          },
        };
        processedActiveSlotRef.current = standbySlot;
        processedAudioUrlRef.current = toAssetUrl(pending.reference);
        setProcessedAudioUrl(processedAudioUrlRef.current);
        processedAudioPlayingRef.current = true;
        setProcessedAudioReady(true);
        scheduleProcessedAudioCrossfade(true, standbySlot);
        applyPlayerSnapshot(nextSnapshot);
        syncUserAudioSettings();
        if (oldActive && oldActive !== standby) {
          window.setTimeout(() => {
            releaseCommittedAudioSlot(oldActive, oldReference);
          }, AUDIO_PARAM_CROSSFADE_SEC * 1_000);
        }
      }).catch((cause) => failStandby(cause));
    };
    standby.addEventListener('playing', confirmCandidatePlayback);
    standby.addEventListener('timeupdate', confirmCandidatePlayback);
    confirmationTimer = window.setTimeout(() => {
      failStandby(new Error('候选声音启动后未完成同步'));
    }, MEDIA_ARTIFACT_CONFIRM_TIMEOUT_MS);
    try {
      standby.currentTime = localPositionMs / 1_000;
    } catch {
      // metadata 已通过；首次 seek 被 WebView 拒绝时从候选起点播放。
    }
    void standby.play().catch(failStandby);
  }

  function publishMediaState() {
    const channel = playbackChannelRef.current;
    const video = videoRef.current;
    if (!channel || !video) return;
    const currentSnapshot = snapshotRef.current;
    const sourceAudio = sourceAudioRef.current;
    const clockMedia = currentSnapshot?.source_media?.media_kind === 'video'
      && sourceAudio
      ? sourceAudio
      : video;
    const duration = Number.isFinite(clockMedia.duration) && clockMedia.duration >= 0
      ? clockMedia.duration
      : 0;
    const currentTime = clampMediaTime(clockMedia.currentTime, duration);
    const probedDurationMs = currentSnapshot?.source_media?.duration_ms;
    const durationMs = typeof probedDurationMs === 'number' && probedDurationMs > 0
      ? Math.round(probedDurationMs)
      : Math.max(0, Math.round(duration * 1_000));
    const localPositionMs = Math.max(0, Math.round(currentTime * 1_000));
    const playbackGeneration = currentSnapshot?.playback_generation ?? 0;
    const sourcePath = currentSnapshot?.source_media?.source_path ?? '';
    const identity = `${playbackGeneration}:${sourcePath}`;
    if (clockIdentityRef.current !== identity) {
      clockIdentityRef.current = identity;
      clockEpochRef.current += 1;
      clockSequenceRef.current = 0;
      sourceRevisionRef.current += 1;
    }
    const directLoopIndex = Math.max(loopSequenceRef.current, currentSnapshot?.loop_index ?? 0);
    const absolutePositionMs = directLoopIndex * durationMs + Math.min(durationMs, localPositionMs);
    const sourceClock = mapAbsolutePositionToSourceClock(absolutePositionMs, durationMs);
    if (!sourceClock) return;
    const positionMs = sourceClock.positionMs;
    if (sourceAudio && currentSnapshot?.source_media?.media_kind === 'video') {
      const previousAudioLoopIndex = sourceAudioLoopIndexRef.current;
      const sync = resolveSourceAudioSync(
        video.playbackRate,
        sourceAudio.currentTime * 1_000 - positionMs,
        previousAudioLoopIndex !== null && previousAudioLoopIndex !== sourceClock.loopIndex,
        sourceAudio.ended,
      );
      sourceAudioLoopIndexRef.current = sourceClock.loopIndex;
      sourceAudio.playbackRate = sync.playbackRate;
      if (sync.hardRealign) {
        sourceAudio.currentTime = positionMs / 1_000;
        if (currentSnapshot.playback_state === 'playing' && sourceAudio.paused) {
          void sourceAudio.play().catch(() => undefined);
        }
      }
    }
    loopSequenceRef.current = sourceClock.loopIndex;
    clockSequenceRef.current += 1;
    if (!Number.isSafeInteger(absolutePositionMs)) return;
    latestAbsolutePositionMsRef.current = absolutePositionMs;
    tryActivatePendingAudioArtifact(absolutePositionMs);
    const nowMs = Date.now();
    if (nowMs - lastRustPositionSyncAtRef.current >= 500) {
      lastRustPositionSyncAtRef.current = nowMs;
      void invokePlaybackSnapshot('update_playback_position', {
        request: { position_ms: positionMs },
      }).catch(() => undefined);
    }
    try {
      channel.postMessage({
        version: 2,
        type: 'playback-media-state',
        // 主窗口的进度条始终展示源媒体时钟；候选片段自己的局部时间只在
        // 最终效果窗内部用于 A/B 对时，不能把 8～15 秒片段误报成源时长。
        current_time: positionMs / 1_000,
        duration: durationMs / 1_000,
        volume: clampVolume(userVolumeRef.current),
        muted: userMutedRef.current,
        paused: clockMedia.paused,
        playback_generation: playbackGeneration,
        source_revision: sourceRevisionRef.current,
        clock_session: clockSessionRef.current,
        clock_epoch: clockEpochRef.current,
        clock_sequence: clockSequenceRef.current,
        loop_index: loopSequenceRef.current,
        position_ms: positionMs,
        duration_ms: durationMs,
        absolute_position_ms: absolutePositionMs,
        playback_rate: Number.isFinite(clockMedia.playbackRate) && clockMedia.playbackRate > 0
          ? clockMedia.playbackRate
          : 1,
        clock_health: playbackClockHealthRef.current.status,
      } satisfies PlaybackMediaStateMessage);
    } catch {
      // 播放器关闭时通道可能已失效，媒体播放不应因此失败。
    }
  }

  function applyPlaybackMediaControl(message: PlaybackMediaControlMessage) {
    const video = videoRef.current;
    if (!video) return;
    if (message.action === 'seek') {
      if (!shouldApplyPlaybackSeek(message, snapshotRef.current?.playback_generation)) return;
      clockEpochRef.current += 1;
      clockSequenceRef.current = 0;
      const sourceDurationMs = snapshotRef.current?.source_media?.duration_ms ?? 0;
      const sourcePositionSeconds = clampMediaTime(message.current_time, sourceDurationMs / 1_000);
      video.currentTime = sourcePositionSeconds;
      const sourceAudio = sourceAudioRef.current;
      if (sourceAudio) sourceAudio.currentTime = sourcePositionSeconds;
      const audio = audioRef.current;
      if (audio) {
        audio.currentTime = Math.max(0, sourcePositionSeconds - (snapshotRef.current?.current_audio_start_at_ms ?? 0) / 1000);
      }
      publishMediaState();
      return;
    }
    if (message.action === 'set-volume') {
      userVolumeRef.current = clampVolume(message.volume);
      setUserVolume(userVolumeRef.current);
      syncUserAudioSettings();
      syncPortAudioMediaVolume();
      publishMediaState();
      return;
    }
    if (message.action === 'toggle-muted') {
      userMutedRef.current = !userMutedRef.current;
      setUserMuted(userMutedRef.current);
      syncUserAudioSettings();
      syncPortAudioMediaVolume();
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
      if (isInterludeVolumeControlMessage(event.data)) {
        interludeGainLevelRef.current = toGainValue(event.data.volume_db);
        syncUserAudioSettings();
        if (interludePortAudioRef.current) {
          void invoke<void>('set_portaudio_interlude_volume', {
            request: { volume_db: event.data.volume_db },
          }).catch((cause) => {
            setPlaybackError(getDisplayErrorMessage(cause, 'PortAudio 插话音量更新失败'));
          });
        }
        return;
      }
      if (isPlaybackMediaControlMessage(event.data)) {
        void applyPlaybackMediaControl(event.data);
        return;
      }
      if (isPlaybackControlMessage(event.data)) {
        const video = videoRef.current;
        const audio = audioRef.current;
        const currentSnapshot = snapshotRef.current;
        if (currentSnapshot) {
          applyPlayerSnapshot({
            ...currentSnapshot,
            playback_state: event.data.action === 'resume' ? 'playing' : event.data.action === 'pause' ? 'paused' : 'stopped',
          });
        }
        if (event.data.action === 'stop') {
          if (currentSnapshot) releaseStoppedMediaArtifacts(currentSnapshot);
          audio?.pause();
          if (audio) audio.currentTime = 0;
          const operationId = fixedSpeechOperationRef.current?.operationId;
          if (operationId) cancelFixedSpeech(operationId);
        } else if (event.data.action === 'pause') {
          video?.pause();
          sourceAudioRef.current?.pause();
          audio?.pause();
          window.speechSynthesis?.pause();
        } else if (video) {
          if (event.data.action === 'resume') resumeAudioDiagnostics();
          window.speechSynthesis?.resume();
          void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击播放区域继续。'));
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
      runtimeAudioEnabledRef.current = false;
      runtimeAudioParamsRef.current = event.data.payload;
      syncWebAudioContextState();
      applyRealtimeVideoFx(event.data.payload, false);
      syncUserAudioSettings();
    };
    channel.addEventListener('message', handleMessage);
    const handlePageHide = () => {
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      shutdownInterludePlayback();
      setPortAudioHardwareActive(false);
      if (videoRef.current) clearVideoElement(videoRef.current);
      sourceAudioRef.current?.pause();
      pendingAudioArtifactRef.current = null;
      for (const slot of [processedAudioARef.current, processedAudioBRef.current]) {
        if (slot) clearProcessedAudioSlot(slot);
      }
      const context = audioContextRef.current;
      audioContextRef.current = null;
      audioContextTransitionRef.current = null;
      void context?.close().catch(() => undefined);
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
      shutdownInterludePlayback();
      window.removeEventListener('pagehide', handlePageHide);
      channel.removeEventListener('message', handleMessage);
      channel.close();
      if (playbackChannelRef.current === channel) playbackChannelRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!finalEffectPollingActive) return;
    const channel = playbackChannelRef.current;
    let cancelled = false;
    let diagnosticPollInFlight = false;
    const publishWebAudioFallback = () => {
      if (cancelled || !channel) return;
      const analyser = analyserRef.current;
      if (analyser) {
        const waveform = new Uint8Array(analyser.fftSize);
        const spectrum = new Uint8Array(analyser.frequencyBinCount);
        analyser.getByteTimeDomainData(waveform);
        analyser.getByteFrequencyData(spectrum);
        const waveformSamples = Array.from(waveform.slice(0, DIAGNOSTIC_SAMPLE_COUNT), (sample) => (sample - 128) / 128);
        const spectrumSamples = Array.from(spectrum.slice(0, DIAGNOSTIC_SAMPLE_COUNT), (sample) => sample / 255);
        channel.postMessage({
          version: 1,
          type: 'diagnostic',
          source: 'web-audio-analyser',
          sequence: null,
          has_pcm: false,
          sample_rate_hz: audioContextRef.current?.sampleRate ?? null,
          captured_frame_count: null,
          line: waveformSamples.slice(0, DIAGNOSTIC_LINE_SAMPLE_COUNT),
          rms_dbfs: null,
          peak_dbfs: null,
          low_band_rms_dbfs: null,
          cutoff_hz: null,
          mfcc: [],
          mfcc_available: false,
          noise_floor_dbfs: null,
          snr_db: null,
          formants_hz: [null, null, null],
          current_formant_hz: null,
          waveform: waveformSamples,
          spectrum: spectrumSamples,
          sent_at_ms: Date.now(),
          error: playbackError ?? finalEffectResizeError,
        } satisfies DiagnosticMessage);
        return;
      }
      channel.postMessage({
        version: 1,
        type: 'diagnostic',
        source: 'web-audio-analyser',
        sequence: null,
        has_pcm: false,
        sample_rate_hz: null,
        captured_frame_count: null,
        line: [],
        rms_dbfs: null,
        peak_dbfs: null,
        low_band_rms_dbfs: null,
        cutoff_hz: null,
        mfcc: [],
        mfcc_available: false,
        noise_floor_dbfs: null,
        snr_db: null,
        formants_hz: [null, null, null],
        current_formant_hz: null,
        sent_at_ms: Date.now(),
        error: playbackError ?? finalEffectResizeError ?? 'PortAudio 混音快照暂无数据，Web Audio 回退不可用',
      } satisfies DiagnosticMessage);
    };
    const publishDiagnostic = () => {
      if (cancelled || !channel || diagnosticPollInFlight) return;
      diagnosticPollInFlight = true;
      void invoke<unknown>('get_audio_cycle_diagnostic')
        .then((diagnostic) => {
          if (cancelled || !channel) return;
          if (isAudioCycleDiagnosticSnapshot(diagnostic) && diagnostic.has_pcm && diagnostic.line.length > 0) {
            channel.postMessage({
              version: 1,
              type: 'diagnostic',
              source: 'portaudio-mixed-pcm',
              sequence: diagnostic.sequence,
              has_pcm: diagnostic.has_pcm,
              sample_rate_hz: diagnostic.sample_rate_hz,
              captured_frame_count: diagnostic.captured_frame_count,
              line: diagnostic.line,
              rms_dbfs: diagnostic.rms_dbfs,
              peak_dbfs: diagnostic.peak_dbfs,
              low_band_rms_dbfs: diagnostic.low_band_rms_dbfs,
              cutoff_hz: diagnostic.cutoff_hz,
              mfcc: diagnostic.mfcc,
              mfcc_available: diagnostic.mfcc_available,
              noise_floor_dbfs: diagnostic.noise_floor_dbfs,
              snr_db: diagnostic.snr_db,
              formants_hz: diagnostic.formants_hz,
              current_formant_hz: diagnostic.current_formant_hz,
              sent_at_ms: diagnostic.captured_at_ms,
              error: playbackError ?? finalEffectResizeError,
            } satisfies DiagnosticMessage);
            return;
          }
          publishWebAudioFallback();
        })
        .catch(() => {
          publishWebAudioFallback();
        })
        .finally(() => {
          diagnosticPollInFlight = false;
        });
    };
    publishDiagnostic();
    const timer = window.setInterval(publishDiagnostic, DIAGNOSTIC_PUBLISH_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [finalEffectPollingActive, finalEffectResizeError, playbackError]);

  function resumeAudioDiagnostics() {
    syncWebAudioContextState();
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
    setPlaybackError(null);
  }, [sourceUrl]);

  useEffect(() => {
    if (!documentVisible) return;
    const refreshSnapshot = () => {
      if (snapshotPollInFlightRef.current) return;
      snapshotPollInFlightRef.current = true;
      const version = snapshotSyncVersionRef.current;
      void invokePlaybackSnapshot('get_snapshot').then((nextSnapshot) => {
        if (version === snapshotSyncVersionRef.current) applyPlayerSnapshot(nextSnapshot);
      }).catch(() => undefined).finally(() => {
        snapshotPollInFlightRef.current = false;
      });
    };
    refreshSnapshot();
    const timer = window.setInterval(refreshSnapshot, PLAYBACK_SNAPSHOT_POLL_MS);
    return () => window.clearInterval(timer);
  }, [documentVisible]);

  // 最终效果窗自行探测 PortAudio，避免只依赖 BroadcastChannel 时序。
  useEffect(() => {
    if (!finalEffectPollingActive || !snapshot?.audio_processing_enabled) return;
    let cancelled = false;
    let realignBusy = false;
    let realignAttemptedForHz: number | null = null;
    let realignFailedForHz: number | null = null;
    let avDriftPollCount = 0;
    let avDriftCooldownUntilMs = 0;
    let sourceRetryAttemptedAtMs = 0;
    let outputStatusPollInFlight = false;
    const refreshOutput = () => {
      if (outputStatusPollInFlight) return;
      outputStatusPollInFlight = true;
      void invokeAudioOutputBackendStatus('get_audio_output_backend_status')
        .then(async (status) => {
          if (cancelled) return;
          const targetRate = PORTAUDIO_SAMPLE_RATE_HZ;
          const hardware =
            PORTAUDIO_FORMAL_SOURCE_SYNC_READY
            && Boolean(status.preferred_portaudio)
            && Boolean(status.running)
            && status.selected_backend === 'portaudio';
          if (hardware || !status.preferred_portaudio) {
            sourceRetryAttemptedAtMs = 0;
          }
          const sourceRetryCoolingDown = sourceRetryAttemptedAtMs > 0
            && Date.now() - sourceRetryAttemptedAtMs < AUTO_PORTAUDIO_RETRY_COOLDOWN_MS;
          const sourceRetryMode = getPortAudioSourceRetryMode(
            status,
            snapshotRef.current?.playback_state,
            sourceRetryCoolingDown,
          );
          if (sourceRetryMode) {
            sourceRetryAttemptedAtMs = Date.now();
            setPortAudioHardwareActive(false);
            syncAudioOutputSourceLatest(sourceRetryMode === 'recover');
            return;
          }
          const avOffsetMs = status.av_offset_ms;
          if (
            hardware
            && typeof avOffsetMs === 'number'
            && Number.isFinite(avOffsetMs)
            && Math.abs(avOffsetMs) > AUDIO_VIDEO_REALIGN_THRESHOLD_MS
          ) {
            avDriftPollCount += 1;
          } else {
            avDriftPollCount = 0;
          }
          if (
            avDriftPollCount >= AUDIO_VIDEO_REALIGN_CONSECUTIVE_POLLS
            && Date.now() >= avDriftCooldownUntilMs
          ) {
            avDriftPollCount = 0;
            avDriftCooldownUntilMs = Date.now() + AUDIO_VIDEO_REALIGN_COOLDOWN_MS;
            syncAudioOutputSourceLatest();
          }
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
              await loopSyncPromiseRef.current;
              const syncClock = resolveAudioSyncClock(
                snapshotRef.current,
                null,
                videoRef.current,
                loopSequenceRef.current,
              );
              if (!syncClock) throw new Error('最终效果窗口尚未建立绝对媒体时钟');
              const next = await invokeAudioOutputBackendStatus('set_audio_output_backend', {
                request: {
                  prefer_portaudio: true,
                  device_index: status.device_index,
                  memory_buffer_kib: status.memory_buffer_kib,
                  frames_per_buffer: status.frames_per_buffer,
                  sample_rate_hz: targetRate,
                  ...syncClock,
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
  }, [finalEffectPollingActive, snapshot?.audio_processing_enabled]);

  // 图只建一次：独立源音频汇入 mainProgramGain，画面元素始终无声且 muted。
  useEffect(() => {
    if (!sourceUrl) return;
    let cancelled = false;
    let retryTimer: number | null = null;

    const ensureGraph = () => {
      if (cancelled) return;
      if (audioContextCleanupTimerRef.current !== null) {
        window.clearTimeout(audioContextCleanupTimerRef.current);
        audioContextCleanupTimerRef.current = null;
      }
      if (audioContextRef.current && videoFxGainRef.current) {
        syncWebAudioContextState();
        audioDiagnosticsReadyRef.current = true;
        setAudioDiagnosticsReady(true);
        syncUserAudioSettings();
        return;
      }
      const video = videoRef.current;
      const sourceAudio = sourceAudioRef.current;
      const audio = audioRef.current;
      const processedA = processedAudioARef.current;
      const processedB = processedAudioBRef.current;
      const interlude = interludeAudioRef.current;
      if (!video || !sourceAudio || !audio || !processedA || !processedB || !interlude) {
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
        // 画面只承担源媒体身份；原声由独立 audio 元素输出，避免双声。
        const dryGain = context.createGain();
        const slotGainA = context.createGain();
        const slotGainB = context.createGain();
        const mainProgramGain = context.createGain();
        dryGain.gain.value = 1;
        slotGainA.gain.value = 0;
        slotGainB.gain.value = 0;
        mainProgramGain.gain.value = interludeActiveRef.current ? duckGainLevelRef.current : 1;
        context.createMediaElementSource(sourceAudio).connect(dryGain);
        dryGain.connect(gain);
        gain.connect(low);
        low.connect(mid);
        mid.connect(high);
        high.connect(mainProgramGain);
        high.connect(reverbDelay);
        reverbDelay.connect(reverbFeedback);
        reverbFeedback.connect(reverbDelay);
        reverbDelay.connect(reverbWet);
        reverbWet.connect(mainProgramGain);
        noiseSource.connect(noiseGain);
        noiseGain.connect(mainProgramGain);
        noiseSource.start(0);
        context.createMediaElementSource(processedA).connect(slotGainA);
        context.createMediaElementSource(processedB).connect(slotGainB);
        slotGainA.connect(mainProgramGain);
        slotGainB.connect(mainProgramGain);
        context.createMediaElementSource(audio).connect(mainProgramGain);
        const mainMediaVolumeGain = context.createGain();
        mainProgramGain.connect(mainMediaVolumeGain);
        mainMediaVolumeGain.connect(analyser);
        context.createMediaElementSource(interlude).connect(analyser);
        videoDryGainRef.current = dryGain;
        processedSlotGainARef.current = slotGainA;
        processedSlotGainBRef.current = slotGainB;
        mainProgramGainRef.current = mainProgramGain;
        mainMediaVolumeGainRef.current = mainMediaVolumeGain;
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
        syncWebAudioContextState();
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
        mainProgramGainRef.current = null;
        mainMediaVolumeGainRef.current = null;
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
        audioContextTransitionRef.current = null;
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
        mainProgramGainRef.current = null;
        mainMediaVolumeGainRef.current = null;
        processedAudioGainTargetRef.current = null;
        speakerMuteGainRef.current = null;
        portAudioHardwareRef.current = false;
        setPortAudioHardwareEnabled(false);
        audioDiagnosticsReadyRef.current = false;
        setAudioDiagnosticsReady(false);
        void context?.close().catch(() => undefined);
      }, 100);
    };
  }, [Boolean(sourceUrl)]);

  // 换源：只 resume + 应用当前实时参数，不重建图。
  useEffect(() => {
    if (!sourceUrl) return;
    syncWebAudioContextState();
    syncUserAudioSettings();
  }, [sourceUrl]);

  function syncAudioOutputSourceLatest(
    recoverUnhealthy = false,
    reanchorLoopBoundary = false,
  ) {
    const sync = audioSourceSyncRef.current;
    sync.latestRequest += 1;
    sync.pending = true;
    sync.recoverUnhealthy ||= recoverUnhealthy;
    sync.reanchorLoopBoundary ||= reanchorLoopBoundary;
    if (sync.running) return;
    sync.running = true;
    void (async () => {
      let latestFailure: string | null = null;
      let supersededRetryUsed = false;
      try {
        while (sync.pending) {
          sync.pending = false;
          const requestId = sync.latestRequest;
          const recover_unhealthy = sync.recoverUnhealthy;
          const reanchor_loop_boundary = sync.reanchorLoopBoundary;
          sync.recoverUnhealthy = false;
          sync.reanchorLoopBoundary = false;
          try {
            await loopSyncPromiseRef.current;
            const syncClock = resolveAudioSyncClock(
              snapshotRef.current,
              null,
              videoRef.current,
              loopSequenceRef.current,
            );
            if (!syncClock) throw new Error('最终效果窗口尚未建立绝对媒体时钟');
            const status = await invokeAudioOutputBackendStatus('sync_audio_output_source', {
              request: { ...syncClock, recover_unhealthy, reanchor_loop_boundary },
            });
            if (requestId !== sync.latestRequest) continue;
            const disposition = classifyAudioOutputSync(status);
            if (disposition === 'active') {
              latestFailure = null;
              setPlaybackError(clearResolvedPortAudioSyncError);
              continue;
            }
            if (disposition === 'retryable') {
              latestFailure = null;
              if (!status.running && portAudioHardwareRef.current) {
                setPortAudioHardwareActive(false);
              }
              if (
                status.reason_code === 'audio_mixer_candidate_superseded'
                && !supersededRetryUsed
              ) {
                supersededRetryUsed = true;
                sync.pending = true;
                sync.recoverUnhealthy ||= recover_unhealthy;
                sync.reanchorLoopBoundary ||= reanchor_loop_boundary;
              }
              continue;
            }
            latestFailure = status.reason ?? '当前源音频不可用。';
          } catch (cause) {
            if (requestId === sync.latestRequest) {
              const code = getCommandErrorCode(cause);
              const effectivePlaybackState = videoRef.current?.paused
                ? 'paused'
                : snapshotRef.current?.playback_state;
              if (isExpectedAudioOutputSyncCancellation(code, effectivePlaybackState)) {
                latestFailure = null;
              } else if (isRetryableAudioOutputSyncCode(code)) {
                latestFailure = null;
                if (code === 'audio_mixer_candidate_superseded' && !supersededRetryUsed) {
                  supersededRetryUsed = true;
                  sync.pending = true;
                  sync.recoverUnhealthy ||= recover_unhealthy;
                  sync.reanchorLoopBoundary ||= reanchor_loop_boundary;
                }
              } else {
                latestFailure = getDisplayErrorMessage(cause, '当前源音频不可用。');
              }
            }
          }
        }
        if (latestFailure && sync.latestRequest === audioSourceSyncRef.current.latestRequest) {
          if (portAudioHardwareRef.current) setPortAudioHardwareActive(false);
          setPlaybackError(latestFailure);
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
      sync.recoverUnhealthy = false;
      sync.reanchorLoopBoundary = false;
    };
  }, [
    portAudioHardwareEnabled,
    portAudioSourcePath,
    snapshot?.playback_generation,
    snapshot?.audio_stream_revision,
  ]);

  useEffect(() => {
    // WebView 保留源媒体时钟和 Original 兜底；正式处理画面由 mpv 原生表面承载。
    const nextUrl = playbackVideoUrl(snapshot);
    setSourceUrl((prev) => (prev === nextUrl ? prev : nextUrl));
  }, [
    snapshot?.audio_processing_enabled,
    snapshot?.current_video_reference,
    snapshot?.current_video_source,
    snapshot?.source_media?.media_kind,
    snapshot?.source_media?.playback_reference,
    snapshot?.source_media?.source_path,
  ]);

  useEffect(() => {
    const pendingReference = snapshot?.pending_audio_artifact_reference;
    const timeline = pendingAudioArtifactTimeline(snapshot);
    const sourcePath = snapshot?.source_media?.source_path;
    if (
      snapshot?.audio_processing_status !== 'ready'
      || !pendingReference
      || !timeline
      || !sourcePath
      || timeline.playbackGeneration !== snapshot.playback_generation
    ) return;
    const standbySlot: 0 | 1 = processedAudioPlayingRef.current
      ? (processedActiveSlotRef.current === 0 ? 1 : 0)
      : 0;
    const candidate = standbySlot === 0 ? processedAudioARef.current : processedAudioBRef.current;
    if (!candidate) return;
    clearProcessedAudioSlot(candidate);
    candidate.preload = 'auto';
    let cancelled = false;
    const discard = (reason: string) => {
      if (cancelled || pendingAudioArtifactRef.current?.element !== candidate) return;
      pendingAudioArtifactRef.current = null;
      clearProcessedAudioSlot(candidate);
      setPlaybackError(reason);
      void invokePlaybackSnapshot('discard_audio_media_candidate', {
        request: {
          plan_id: timeline.planId,
          sequence: timeline.sequence,
          playback_generation: timeline.playbackGeneration,
          source_revision: timeline.sourceRevision,
          reason,
        },
      }).then(applyPlayerSnapshot).catch(() => undefined);
    };
    const markPreloaded = () => {
      if (cancelled) return;
      const pending = pendingAudioArtifactRef.current;
      if (!pending || pending.element !== candidate) return;
      pending.preloaded = true;
      tryActivatePendingAudioArtifact(latestAbsolutePositionMsRef.current);
    };
    const handleError = () => discard('处理后声音预加载失败，继续播放当前声音。');
    pendingAudioArtifactRef.current = {
      reference: pendingReference,
      sourcePath,
      timeline,
      element: candidate,
      preloaded: false,
      activating: false,
    };
    candidate.addEventListener('canplay', markPreloaded, { once: true });
    candidate.addEventListener('error', handleError, { once: true });
    candidate.src = toAssetUrl(pendingReference) ?? '';
    candidate.load();
    return () => {
      cancelled = true;
      candidate.removeEventListener('canplay', markPreloaded);
      candidate.removeEventListener('error', handleError);
      if (pendingAudioArtifactRef.current?.element === candidate) {
        pendingAudioArtifactRef.current = null;
      }
      if (
        !processedAudioPlayingRef.current
        || (processedActiveSlotRef.current === 0 ? processedAudioARef.current : processedAudioBRef.current) !== candidate
      ) clearProcessedAudioSlot(candidate);
    };
  }, [
    snapshot?.pending_audio_artifact_reference,
    snapshot?.pending_audio_media_plan_id,
    snapshot?.pending_audio_media_sequence,
    snapshot?.pending_audio_media_playback_generation,
    snapshot?.pending_audio_media_source_revision,
    snapshot?.pending_audio_media_target_absolute_position_ms,
    snapshot?.pending_audio_media_output_duration_ms,
    snapshot?.pending_audio_media_valid_until_absolute_position_ms,
    snapshot?.audio_processing_status,
  ]);

  useEffect(() => {
    const audioLive = Boolean(snapshot?.audio_processing_enabled);
    const ref = snapshot?.current_audio_artifact_reference ?? null;
    const next = audioLive && ref ? toAssetUrl(ref) : null;
    setProcessedAudioUrl((prev) => (prev === next ? prev : next));
    if (!next) {
      processedAudioPlayingRef.current = false;
      setProcessedAudioReady(false);
    }
  }, [
    snapshot?.audio_processing_enabled,
    snapshot?.current_audio_artifact_reference,
    snapshot?.audio_processing_status,
  ]);

  useEffect(() => {
    const operationId = fixedSpeechOperationRef.current?.operationId;
    if (operationId) cancelFixedSpeech(operationId);
  }, [snapshot?.playback_generation, snapshot?.source_media?.source_path]);

  useEffect(() => {
    const source = snapshot?.source_media;
    if (source?.media_kind !== 'video') {
      setFinalEffectResizeError(null);
      return;
    }
    const resizeKey = buildFinalEffectWindowResizeKey({
      playbackGeneration: snapshot?.playback_generation,
      width: source?.width,
      height: source?.height,
      sourcePath: source?.source_path,
    });
    if (!resizeKey || resizeKey === finalEffectResizeKeyRef.current) return;

    let cancelled = false;
    let retryTimer: number | null = null;
    let releaseRetryWait: (() => void) | null = null;
    const isCurrentResize = () => {
      const currentSnapshot = snapshotRef.current;
      const currentSource = currentSnapshot?.source_media;
      if (currentSource?.media_kind !== 'video') return false;
      return buildFinalEffectWindowResizeKey({
        playbackGeneration: currentSnapshot?.playback_generation,
        width: currentSource?.width,
        height: currentSource?.height,
        sourcePath: currentSource?.source_path,
      }) === resizeKey;
    };
    const waitBeforeRetry = () => new Promise<void>((resolve) => {
      releaseRetryWait = resolve;
      retryTimer = window.setTimeout(() => {
        retryTimer = null;
        releaseRetryWait = null;
        resolve();
      }, 200);
    });
    const resizeWindow = async () => {
      for (let attempts = 1; attempts <= 3; attempts += 1) {
        if (cancelled || !isCurrentResize()) return;
        try {
          await invoke('resize_final_effect_window', {
            request: { width: source?.width, height: source?.height },
          });
          if (cancelled || !isCurrentResize()) return;
          finalEffectResizeKeyRef.current = resizeKey;
          setFinalEffectResizeError(null);
          return;
        } catch (cause) {
          if (cancelled || !isCurrentResize()) return;
          if (attempts === 3) {
            setFinalEffectResizeError(cause instanceof Error ? cause.message : '最终效果窗口尺寸调整失败');
            return;
          }
          await waitBeforeRetry();
        }
      }
    };
    const queuedResize = finalEffectResizeQueueRef.current.then(resizeWindow, resizeWindow);
    finalEffectResizeQueueRef.current = queuedResize;
    return () => {
      cancelled = true;
      if (retryTimer !== null) {
        window.clearTimeout(retryTimer);
        retryTimer = null;
      }
      releaseRetryWait?.();
    };
  }, [
    snapshot?.playback_generation,
    snapshot?.source_media?.height,
    snapshot?.source_media?.media_kind,
    snapshot?.source_media?.source_path,
    snapshot?.source_media?.width,
  ]);

  useEffect(() => {
    loadedPlaybackGenerationRef.current = null;
    const video = videoRef.current;
    const expectedPlaybackGeneration = snapshot?.playback_generation;
    const expectedIdentity = finalEffectMediaIdentity;
    if (
      !video
      || !snapshot
      || !sourceUrl
      || expectedPlaybackGeneration === undefined
      || !expectedIdentity
    ) return;
    const resumeAt = resolvePlaybackResumePosition(
      playbackPositionRef.current,
      expectedPlaybackGeneration,
    );
    let cancelled = false;
    const restore = () => {
      if (cancelled || !isCurrentFinalEffectVideo(video, expectedIdentity)) return;
      video.removeEventListener('loadedmetadata', restore);
      loadedPlaybackGenerationRef.current = expectedPlaybackGeneration;
      setPlaybackError(null);
      if (resumeAt > 0.05 && Number.isFinite(video.duration) && video.duration > 0) {
        const safeEnd = Math.max(0, video.duration - 0.2);
        const target = Math.min(resumeAt, safeEnd);
        if (Math.abs(video.currentTime - target) > 0.05) {
          video.currentTime = target;
        }
        playbackPositionRef.current = {
          playbackGeneration: expectedPlaybackGeneration,
          positionSec: video.currentTime,
        };
      }
      syncWebAudioContextState();
      userMutedRef.current = false;
      syncUserAudioSettings();
      if (snapshotRef.current?.playback_state === 'playing') {
        suppressMediaEventRef.current = true;
        void video.play().catch(() => {
          if (!isCurrentFinalEffectVideo(video, expectedIdentity)) return;
          suppressMediaEventRef.current = false;
          setPlaybackError('处理结果已切换，但自动播放被拦截，请点击播放区域恢复声音。');
        });
      }
    };
    video.addEventListener('loadedmetadata', restore);
    if (video.readyState >= 1) restore();
    return () => {
      cancelled = true;
      video.removeEventListener('loadedmetadata', restore);
    };
  }, [finalEffectLoadIdentity, finalEffectMediaIdentity]);

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

  function hasUsableProcessedAudioOutput() {
    if (!processedAudioPlayingRef.current) return false;
    const active = processedActiveSlotRef.current === 0
      ? processedAudioARef.current
      : processedAudioBRef.current;
    return Boolean(active && !active.error && (active.currentSrc || active.getAttribute('src')));
  }

  function handleProcessedAudioElementError(event: SyntheticEvent<HTMLAudioElement>) {
    const active = processedActiveSlotRef.current === 0
      ? processedAudioARef.current
      : processedAudioBRef.current;
    if (!processedAudioPlayingRef.current || event.currentTarget === active) {
      restoreDryAudioOutput();
    }
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
    let onSeeked: (() => void) | null = null;
    let onPlaying: (() => void) | null = null;
    let audibleFallbackTimer: number | null = null;
    let crossfadeTimer: number | null = null;
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
        crossfadeTimer = window.setTimeout(() => {
          crossfadeTimer = null;
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
      onPlaying = () => {
        const listener = onPlaying;
        if (listener) {
          standby.removeEventListener('playing', listener);
          standby.removeEventListener('timeupdate', listener);
          onPlaying = null;
        }
        if (audibleFallbackTimer !== null) {
          window.clearTimeout(audibleFallbackTimer);
          audibleFallbackTimer = null;
        }
        cutOver();
      };
      standby.addEventListener('playing', onPlaying);
      standby.addEventListener('timeupdate', onPlaying);
      // 保险：仍无事件则 400ms 后切，避免永远不切
      audibleFallbackTimer = window.setTimeout(() => {
        audibleFallbackTimer = null;
        if (!cancelled && !cutDone && !standby.paused) {
          const listener = onPlaying;
          if (listener) {
            standby.removeEventListener('playing', listener);
            standby.removeEventListener('timeupdate', listener);
            onPlaying = null;
          }
          cutOver();
        }
      }, 400);
    };
    const activate = () => {
      if (cancelled) return;
      const t = currentProcessedAudioTimeSeconds(standby);
      const target = Number.isFinite(standby.duration) && standby.duration > 0
        ? Math.min(Math.max(0, t), Math.max(0, standby.duration - 0.05))
        : Math.max(0, t);
      const startPlay = () => {
        if (cancelled) return;
        void standby.play().then(waitAudibleThenCut).catch(failStandby);
      };
      if (Math.abs((standby.currentTime || 0) - target) > 0.05) {
        onSeeked = () => {
          const listener = onSeeked;
          if (listener) {
            standby.removeEventListener('seeked', listener);
            onSeeked = null;
          }
          startPlay();
        };
        standby.addEventListener('seeked', onSeeked);
        try {
          standby.currentTime = target;
        } catch {
          standby.removeEventListener('seeked', onSeeked);
          onSeeked = null;
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
      standby.removeEventListener('loadedmetadata', activate);
      standby.removeEventListener('error', failStandby);
      if (onSeeked) standby.removeEventListener('seeked', onSeeked);
      if (onPlaying) {
        standby.removeEventListener('playing', onPlaying);
        standby.removeEventListener('timeupdate', onPlaying);
      }
      if (audibleFallbackTimer !== null) window.clearTimeout(audibleFallbackTimer);
      if (crossfadeTimer !== null) window.clearTimeout(crossfadeTimer);
    };
  }, [processedAudioUrl]);

  // 真轨失效回干声；加载/seek 中的短暂停不算失效
  useEffect(() => {
    if (!finalEffectPollingActive || !processedAudioReady) return;
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
  }, [finalEffectPollingActive, processedAudioReady]);

  useEffect(() => {
    if (snapshot?.audio_processing_status !== 'failed') return;
    if (!snapshot.audio_processing_enabled || !hasUsableProcessedAudioOutput()) {
      restoreDryAudioOutput();
    }
  }, [snapshot?.audio_processing_enabled, snapshot?.audio_processing_status, snapshot?.fallback_reason]);

  useEffect(() => {
    const video = videoRef.current;
    const sourceAudio = sourceAudioRef.current;
    const audio = audioRef.current;
    if (!video || !sourceAudio || !sourceUrl) return;
    if (snapshot?.playback_state === 'stopped') {
      video.pause();
      sourceAudio.pause();
      playbackPositionRef.current = null;
      loadedPlaybackGenerationRef.current = null;
      video.currentTime = 0;
      sourceAudio.currentTime = 0;
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
      clearInterludePlayback({ resetSchedule: true, resetIndex: true });
      return;
    }
    if (snapshot?.playback_state === 'paused' || snapshot?.playback_state === 'ready') {
      // ready：导入后只展示首帧，等主页「播放」
      video.pause();
      sourceAudio.pause();
      audio?.pause();
      processedAudioARef.current?.pause();
      processedAudioBRef.current?.pause();
      window.speechSynthesis?.pause();
      pauseInterludePlayback();
      return;
    }
    if (snapshot?.playback_state === 'playing') {
      window.speechSynthesis?.resume();
      if (video.paused) {
        void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击播放区域继续。'));
      }
      const sourceDurationSeconds = (snapshot.source_media?.duration_ms ?? 0) / 1_000;
      const sourcePositionSeconds = sourceDurationSeconds > 0
        ? video.currentTime % sourceDurationSeconds
        : video.currentTime;
      if (Math.abs(sourceAudio.currentTime - sourcePositionSeconds) > 0.08) {
        sourceAudio.currentTime = sourcePositionSeconds;
      }
      if (sourceAudio.paused) {
        void sourceAudio.play().catch(() => setPlaybackError('原声音轨自动播放被拦截，请点击播放区域继续。'));
      }
      const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
      if (effectiveAudioSource === 'realtime_variant' && audioUrl && audioDiagnosticsReady && audio) {
        void audio.play().catch(() => undefined);
      }
      if (processedAudioPlayingRef.current) {
        const active = processedActiveSlotRef.current === 0
          ? processedAudioARef.current
          : processedAudioBRef.current;
        if (active) {
          active.currentTime = currentProcessedAudioTimeSeconds(active);
          void active.play().catch(() => undefined);
        }
      }
      resumeInterludePlayback();
    }
  }, [audioDiagnosticsReady, audioUrl, snapshot?.playback_state, snapshot?.effective_audio_source, snapshot?.current_audio_source, sourceUrl, processedAudioReady]);

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
    if (interludeAudioUrl && interludeAudioUrlRef.current === interludeAudioUrl) {
      syncUserAudioSettings();
      return;
    }
    interludeAudioUrlRef.current = interludeAudioUrl;
    if (!interludeAudioUrl) {
      interludeAudio.pause();
      interludeAudio.currentTime = 0;
      interludeAudio.removeAttribute('src');
      interludeAudio.load();
      syncUserAudioSettings();
      return;
    }
    if (!audioDiagnosticsReady && !interludePortAudioRef.current) {
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
      void interludeAudio.play().catch((cause) => {
        setPlaybackError(getDisplayErrorMessage(cause, '插话音频无法开始播放'));
        clearInterludePlayback({ resetSchedule: true });
      });
    }
  }, [audioDiagnosticsReady, interludeAudioUrl]);

  function startPlaybackClockRecovery(
    video: HTMLVideoElement,
    expectedIdentity: string,
    resumeAt: number,
  ) {
    if (clockRecoveryRef.current?.identity === expectedIdentity) return;
    let started = false;
    let settled = false;
    let timeout: number | null = null;
    let cancelRecovery = () => undefined;
    const recovery = new Promise<void>((resolve, reject) => {
      const cleanup = () => {
        video.removeEventListener('loadedmetadata', restoreAndPlay);
        video.removeEventListener('error', fail);
        if (timeout !== null) window.clearTimeout(timeout);
      };
      const settle = (cause?: unknown) => {
        if (settled) return;
        settled = true;
        cleanup();
        if (cause === undefined) resolve();
        else reject(cause);
      };
      const fail = () => settle(new Error('媒体重新加载失败'));
      cancelRecovery = () => {
        if (settled) return;
        settled = true;
        cleanup();
        resolve();
      };
      const restoreAndPlay = () => {
        if (started || !isCurrentFinalEffectVideo(video, expectedIdentity)) return;
        started = true;
        if (Number.isFinite(video.duration) && video.duration > 0) {
          const safeEnd = Math.max(0, video.duration - 0.2);
          video.currentTime = Math.min(Math.max(0, resumeAt), safeEnd);
        }
        suppressMediaEventRef.current = true;
        void video.play().then(() => {
          if (settled) return;
          if (!isCurrentFinalEffectVideo(video, expectedIdentity)) {
            settle(new Error('媒体源已切换'));
            return;
          }
          const currentSnapshot = snapshotRef.current;
          const audio = audioRef.current;
          if (
            audio
            && audioDiagnosticsReadyRef.current
            && getEffectiveAudioSource(currentSnapshot) === 'realtime_variant'
          ) {
            audio.currentTime = Math.max(
              0,
              video.currentTime - (currentSnapshot?.current_audio_start_at_ms ?? 0) / 1_000,
            );
            void audio.play().catch(() => undefined);
          }
          const processed = processedAudioPlayingRef.current
            ? (processedActiveSlotRef.current === 0
                ? processedAudioARef.current
                : processedAudioBRef.current)
            : null;
          if (processed) {
            processed.currentTime = currentProcessedAudioTimeSeconds(processed);
            void processed.play().catch(() => undefined);
          }
          if (portAudioHardwareRef.current) syncAudioOutputSourceLatest(true, true);
          settle();
        }).catch(settle);
      };

      video.addEventListener('loadedmetadata', restoreAndPlay);
      video.addEventListener('error', fail, { once: true });
      timeout = window.setTimeout(
        () => settle(new Error('媒体恢复超时')),
        PLAYBACK_CLOCK_RECOVERY_TIMEOUT_MS,
      );
      clockLoadingGraceUntilRef.current = performance.now() + PLAYBACK_CLOCK_RECOVERY_TIMEOUT_MS;
      suppressMediaEventRef.current = true;
      video.pause();
      try {
        video.load();
        if (video.readyState >= HTMLMediaElement.HAVE_METADATA) queueMicrotask(restoreAndPlay);
      } catch (cause) {
        settle(cause);
      }
    });
    clockRecoveryRef.current = {
      identity: expectedIdentity,
      promise: recovery,
      cancel: () => cancelRecovery(),
    };
    void recovery.catch((cause) => {
      handlePlaybackClockFailure(video, expectedIdentity, cause);
    }).finally(() => {
      if (clockRecoveryRef.current?.promise === recovery) clockRecoveryRef.current = null;
    });
  }

  function handlePlaybackClockFailure(
    video: HTMLVideoElement,
    expectedIdentity: string,
    cause: unknown,
  ) {
    if (
      playbackClockHealthRef.current.identity !== expectedIdentity
      || clockFailureHandledIdentityRef.current === expectedIdentity
    ) return;
    clockFailureHandledIdentityRef.current = expectedIdentity;
    playbackClockHealthRef.current = failPlaybackClockRecovery(
      playbackClockHealthRef.current,
      expectedIdentity,
    );
    publishMediaState();
    // 播放时钟恢复属于最终效果窗内部自愈，失败也不能冒充用户暂停。
    // Rust 仍保持 playing，快照轮询会继续推动本地媒体重试。
    void video.play().catch(() => undefined);
    setPlaybackError(getDisplayErrorMessage(cause, '播放时钟自动恢复失败，已保持播放状态并继续尝试。'));
  }

  useEffect(() => {
    const video = videoRef.current;
    const expectedIdentity = finalEffectMediaIdentity;
    if (!video || !expectedIdentity) return;
    const mediaEvents = [
      'loadedmetadata',
      'durationchange',
      'timeupdate',
      'play',
      'pause',
      'volumechange',
      'ratechange',
      'seeked',
      'ended',
      'enterpictureinpicture',
      'leavepictureinpicture',
      'waiting',
      'stalled',
      'canplay',
      'playing',
    ];
    const loadingEvents = new Set(['loadedmetadata', 'waiting', 'stalled', 'canplay', 'playing']);
    let loadingEpisode = false;
    const currentClockMedia = (): HTMLMediaElement => {
      const currentSnapshot = snapshotRef.current;
      return currentSnapshot?.source_media?.media_kind === 'video'
        && sourceAudioRef.current
        ? sourceAudioRef.current
        : video;
    };
    const samplePlaybackClock = (event?: Event) => {
      if (!isCurrentFinalEffectVideo(video, expectedIdentity)) return;
      const clockMedia = currentClockMedia();
      const nowMs = performance.now();
      if (event && loadingEvents.has(event.type) && !loadingEpisode) {
        loadingEpisode = true;
        clockLoadingGraceUntilRef.current = nowMs + 1_500;
      }
      const previousHealth = playbackClockHealthRef.current;
      const result = observePlaybackClock(playbackClockHealthRef.current, {
        identity: expectedIdentity,
        nowMs,
        currentTime: clockMedia.currentTime,
        expectedPlaying: snapshotRef.current?.playback_state === 'playing',
        paused: clockMedia.paused,
        seeking: clockMedia.seeking,
        ended: clockMedia.ended,
        // 媒体加载事件只提供一次短暂宽限，避免永久掩盖真实时钟停滞。
        loadingGrace: nowMs < clockLoadingGraceUntilRef.current,
      });
      playbackClockHealthRef.current = result.state;
      if (
        Math.abs(clockMedia.currentTime - previousHealth.lastPositionSec) >= 0.02
        && result.state.status === 'healthy'
      ) {
        loadingEpisode = false;
        clockLoadingGraceUntilRef.current = 0;
        clockFailureHandledIdentityRef.current = null;
      }
      publishMediaState();
      if (result.recoveryRequested) {
        startPlaybackClockRecovery(video, expectedIdentity, clockMedia.currentTime);
      } else if (previousHealth.status !== 'stalled' && result.state.status === 'stalled') {
        handlePlaybackClockFailure(video, expectedIdentity, new Error('媒体恢复后播放时钟仍未推进'));
      }
    };
    const previousIdentity = playbackClockHealthRef.current.identity;
    playbackClockHealthRef.current = resetPlaybackClockObservation(
      playbackClockHealthRef.current,
      expectedIdentity,
      currentClockMedia().currentTime,
      performance.now(),
    );
    if (previousIdentity !== expectedIdentity) {
      clockFailureHandledIdentityRef.current = null;
    }
    clockLoadingGraceUntilRef.current = performance.now() + 1_500;
    mediaEvents.forEach((eventName) => video.addEventListener(eventName, samplePlaybackClock));
    samplePlaybackClock();
    const timer = snapshot?.playback_state === 'playing'
      ? window.setInterval(samplePlaybackClock, 250)
      : null;
    return () => {
      if (timer !== null) window.clearInterval(timer);
      mediaEvents.forEach((eventName) => video.removeEventListener(eventName, samplePlaybackClock));
      if (clockRecoveryRef.current?.identity === expectedIdentity) {
        clockRecoveryRef.current.cancel();
        clockRecoveryRef.current = null;
      }
    };
  }, [finalEffectMediaIdentity, snapshot?.playback_state, sourceUrl]);

  useEffect(() => {
    if (snapshot?.playback_state !== 'playing') {
      if (snapshot?.playback_state === 'stopped') {
        shutdownInterludePlayback();
      } else {
        pauseInterludePlayback();
      }
      return;
    }
    const timer = window.setInterval(() => {
      const video = videoRef.current;
      const currentSnapshot = snapshotRef.current;
      const interlude = currentSnapshot?.interlude ?? null;
      if (
        !video
        || !currentSnapshot
        || (!audioDiagnosticsReadyRef.current && !portAudioHardwareRef.current)
      ) return;

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
        interludeIntervalStartedAtMsRef.current = null;
        lastInterludeIndexRef.current = null;
        interludePresetPeriodPlanRef.current = null;
        lastInterludeAudioPresetIdsRef.current = [];
        clearInterludePlayback({ resetSchedule: true, resetIndex: true });
      }

      if (currentSnapshot.playback_state === 'stopped') {
        if (interludeActiveRef.current || interludeStartingRef.current) {
          clearInterludePlayback({ resetSchedule: true });
        }
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

      if (
        !interlude
        || !interlude.enabled
        || interlude.status !== 'ready'
        || interlude.audio_files.length === 0
      ) {
        if (!interludeActiveRef.current && !interludeStartingRef.current) {
          nextInterludeAtMsRef.current = null;
          interludeIntervalStartedAtMsRef.current = null;
        }
        return;
      }

      if (interludeActiveRef.current) {
        updateActiveInterludePresetPeriod();
        return;
      }
      if (interludeStartingRef.current) return;

      const currentClockMs = performance.now();
      if (nextInterludeAtMsRef.current === null) {
        interludeIntervalStartedAtMsRef.current = currentClockMs;
        nextInterludeAtMsRef.current = nextInterludeAtMs(
          currentClockMs,
          lastInterludeIndexRef.current !== null,
          interlude.interval_min_ms,
          interlude.interval_max_ms,
        );
      }
      const intervalStartedAtMs = interludeIntervalStartedAtMsRef.current ?? currentClockMs;
      publishInterludeRuntime('idle', {
        fileProgressPercent: interludeIntervalProgress(
          intervalStartedAtMs,
          nextInterludeAtMsRef.current,
          currentClockMs,
        ),
      });
      if (currentClockMs < nextInterludeAtMsRef.current) return;
      void startInterludePlayback(interlude);
    }, 250);
    return () => window.clearInterval(timer);
  }, [snapshot?.playback_state, sourceUrl]);

  // N/N+1 临时文件切换不是换源；循环令牌只跟真实播放池源。
  const currentSourceKey = snapshot?.source_media?.source_path ?? null;

  useEffect(() => {
    if (
      loopSourceKeyRef.current !== currentSourceKey ||
      loopGenerationRef.current !== (snapshot?.playback_generation ?? null)
    ) {
      loopSourceKeyRef.current = currentSourceKey;
      loopGenerationRef.current = snapshot?.playback_generation ?? null;
      loopSequenceRef.current = snapshot?.loop_index ?? 0;
      sourceAudioLoopIndexRef.current = null;
      lastRestartTokenRef.current = null;
    } else if ((snapshot?.loop_index ?? 0) > loopSequenceRef.current) {
      loopSequenceRef.current = snapshot?.loop_index ?? loopSequenceRef.current;
    }

    if (snapshot?.playback_state === 'stopped') {
      sourceAudioLoopIndexRef.current = null;
      lastRestartTokenRef.current = null;
    }
  }, [currentSourceKey, snapshot?.loop_index, snapshot?.playback_generation, snapshot?.playback_state]);

  function restartCurrentPlayback(video: HTMLVideoElement, autoplayError: string) {
    // 自然结束会先触发 pause；必须先占住事件，避免误暂停 PortAudio 和候选。
    suppressMediaEventRef.current = true;
    video.currentTime = 0;
    if (sourceAudioRef.current) {
      sourceAudioRef.current.currentTime = 0;
      void sourceAudioRef.current.play().catch(() => undefined);
    }
    if (audioRef.current) audioRef.current.currentTime = 0;
    const processed = processedAudioPlayingRef.current
      ? (processedActiveSlotRef.current === 0 ? processedAudioARef.current : processedAudioBRef.current)
      : null;
    void video.play().then(() => {
      // timeupdate 可能在媒体仍处于 playing 时提前触发循环，此时不会再收到 onPlay。
      // 成功续播后主动释放抑制，避免下一次真实用户暂停被误忽略。
      suppressMediaEventRef.current = false;
      if (processed && processedAudioPlayingRef.current) {
        void processed.play().catch(() => undefined);
      }
    }).catch(() => {
      suppressMediaEventRef.current = false;
      setPlaybackError(autoplayError);
    });
    const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
    if (effectiveAudioSource === 'realtime_variant' && audioRef.current) {
      setRealtimeAudioPlaybackState(false);
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
    }
  }

  function restartToNextLoop(video: HTMLVideoElement, restartToken: string) {
    const currentSnapshot = snapshotRef.current;
    if (!currentSnapshot || currentSnapshot.source_media_index === null) return;
    lastRestartTokenRef.current = restartToken;
    playbackPositionRef.current = null;
    const sourceCount = currentSnapshot.source_media_pool?.length
      ?? (currentSnapshot.source_media ? 1 : 0);
    const restartImmediately = shouldRestartCurrentSourceImmediately(sourceCount);
    if (restartImmediately) {
      loopSequenceRef.current += 1;
      restartCurrentPlayback(video, '媒体已回到开头，但自动播放失败，请点击播放区域继续。');
    } else {
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      clearInterludePlayback({ resetSchedule: true, resetIndex: true });
    }

    const loopSyncPromise = invoke<unknown>('complete_playback_item', {
      request: {
        playback_generation: currentSnapshot.playback_generation,
        loop_index: currentSnapshot.loop_index,
        source_media_index: currentSnapshot.source_media_index,
      },
    })
      .then((result) => {
        if (!isPlaybackItemCompletionResult(result)) {
          throw new Error('播放项切换响应无效');
        }
        const nextSnapshot = result.snapshot;
        if (result.source_changed && restartImmediately) {
          const operationId = fixedSpeechOperationRef.current?.operationId;
          if (operationId) cancelFixedSpeech(operationId);
          clearInterludePlayback({ resetSchedule: true, resetIndex: true });
        }
        loopSequenceRef.current = nextSnapshot.loop_index;
        applyPlayerSnapshot(nextSnapshot);
        if (!result.source_changed && !restartImmediately) {
          restartCurrentPlayback(video, '媒体已回到开头，但自动播放失败，请点击播放区域继续。');
        }
        if (portAudioHardwareRef.current) syncAudioOutputSourceLatest(false, true);
      })
      .catch((cause) => {
        lastRestartTokenRef.current = null;
        loopSequenceRef.current = currentSnapshot.loop_index;
        if (!restartImmediately) {
          restartCurrentPlayback(video, '当前媒体重新播放失败，请点击播放区域恢复播放。');
        }
        setPlaybackError(getDisplayErrorMessage(cause, '播放项切换失败，已重播当前媒体。'));
      });
    const completion = loopSyncPromise.then(() => undefined);
    loopSyncPromiseRef.current = completion;
    void completion.finally(() => {
      if (loopSyncPromiseRef.current === completion) loopSyncPromiseRef.current = null;
    });
  }

  function restartAtBoundary(event: SyntheticEvent<HTMLVideoElement>) {
    const video = event.currentTarget;
    const currentSnapshot = snapshotRef.current;
    if (
      !currentSnapshot
      || currentSnapshot.playback_state !== 'playing'
      || !currentSourceKey
      || !isCurrentFinalEffectVideo(video, finalEffectMediaIdentity)
    ) return;
    const currentTime = Number.isFinite(video.currentTime) ? video.currentTime : 0;
    if (currentTime > 0) {
      playbackPositionRef.current = capturePlaybackPosition(playbackPositionRef.current, {
        playbackGeneration: currentSnapshot.playback_generation,
        loadedPlaybackGeneration: loadedPlaybackGenerationRef.current,
        positionSec: currentTime,
        transitionInFlight: loopSyncPromiseRef.current !== null,
      });
    }
    // ponytail: 不在 timeupdate 里 seek 真轨，频繁 seek 会卡顿；只在循环边界对齐
    const sourceDurationMs = currentSnapshot.source_media?.duration_ms;
    const duration = resolvePlaybackBoundaryDuration({
      mediaDurationSeconds: video.duration,
      sourceDurationMs,
      realtimeVideoStreamActive: false,
    });
    const restartToken = `${currentSourceKey}:${currentSnapshot.playback_generation}:${loopSequenceRef.current}`;
    if (
      !shouldRestartPlayback({
        restartToken,
        lastRestartToken: lastRestartTokenRef.current,
        syncInFlight: loopSyncPromiseRef.current !== null,
        ended: event.type === 'ended',
        endedRequiresDurationBoundary: false,
        currentTime,
        duration,
      })
    ) {
      return;
    }
    restartToNextLoop(video, restartToken);
  }

  return (
    <Layout style={{ position: 'fixed', inset: 0, width: '100vw', height: '100vh', minWidth: 0, minHeight: 0, overflow: 'hidden', background: '#000' }}>
      <Layout.Content style={{ position: 'relative', width: '100%', height: '100%', minWidth: 0, minHeight: 0, padding: 0, overflow: 'hidden' }}>
        <div style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', overflow: 'hidden', background: '#000' }}>
          {sourceUrl ? (
            <>
              <video
                ref={videoRef}
                crossOrigin="anonymous"
                src={sourceUrl ?? undefined}
                playsInline
                preload="auto"
                muted
                onClick={() => {
                  resumeAudioDiagnostics();
                  void sourceAudioRef.current?.play().catch(() => undefined);
                }}
                onPlay={(event) => {
                  if (!isCurrentFinalEffectVideo(event.currentTarget, finalEffectMediaIdentity)) return;
                  if (snapshotRef.current?.playback_state !== 'playing') {
                    suppressMediaEventRef.current = true;
                    event.currentTarget.pause();
                    return;
                  }
                  resumeAudioDiagnostics();
                  if (suppressMediaEventRef.current) suppressMediaEventRef.current = false;
                }}
                onTimeUpdate={restartAtBoundary}
                onEnded={restartAtBoundary}
                style={{
                  position: 'absolute',
                  inset: 0,
                  width: '100%',
                  height: '100%',
                  maxWidth: 'none',
                  maxHeight: 'none',
                  objectFit: 'contain',
                  display: 'block',
                  opacity: currentMediaIsAudio ? 0 : 1,
                  pointerEvents: currentMediaIsAudio ? 'none' : 'auto',
                  background: '#000',
                }}
                onError={(event) => {
                  if (isCurrentFinalEffectVideo(event.currentTarget, finalEffectMediaIdentity)) {
                    setPlaybackError('独立播放器加载媒体失败，请回到主页重新导入。');
                  }
                }}
              />
              <audio
                ref={sourceAudioRef}
                crossOrigin="anonymous"
                src={sourceUrl ?? undefined}
                preload="auto"
                loop={(snapshot?.source_media_pool.length ?? 0) === 1}
                hidden
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
                onError={handleProcessedAudioElementError}
                hidden
              />
              <audio
                ref={processedAudioBRef}
                crossOrigin="anonymous"
                preload="auto"
                onError={handleProcessedAudioElementError}
                hidden
              />
              <audio
                ref={interludeAudioRef}
                crossOrigin="anonymous"
                src={interludeAudioUrl ?? undefined}
                preload="auto"
                muted={portAudioHardwareEnabled}
                onEnded={handleInterludeEnded}
                onError={handleInterludeError}
                hidden
              />
            </>
          ) : null}
        </div>
      </Layout.Content>
    </Layout>
  );
}

function DesktopApp() {
  const { message: messageApi } = AntApp.useApp();
  const location = useLocation();
  const documentVisible = useDocumentVisibility();
  const initialRoute = location.pathname === '/settings' ? 'settings' : 'home';
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
  const savedInterludeConfigRef = useRef<PersistedInterludeConfig | null | undefined>(undefined);
  if (savedInterludeConfigRef.current === undefined) {
    savedInterludeConfigRef.current = loadInterludeConfig();
  }
  const [interludeDraft, setInterludeDraft] = useState<InterludeConfigDraft>(() => (
    buildInterludeDraft(savedInterludeConfigRef.current ?? null)
  ));
  const interludeRestoreAttemptedRef = useRef(false);
  const [interludeDirty, setInterludeDirty] = useState(false);
  const [interludeSaving, setInterludeSaving] = useState(false);
  const [interludeSaveError, setInterludeSaveError] = useState<string | null>(null);
  const [interludeRuntime, setInterludeRuntime] = useState<InterludeRuntimeMessage | null>(null);
  const [mediaEngineCapabilities, setMediaEngineCapabilities] = useState<MediaEngineCapabilities | null>(null);
  const [mediaVideoBackendStatus, setMediaVideoBackendStatus] = useState<MediaVideoBackendStatus | null>(null);
  const [audioOutputBackend, setAudioOutputBackend] = useState<AudioOutputBackendStatus | null>(null);
  const audioOutputBackendRef = useRef<AudioOutputBackendStatus | null>(null);
  const [audioOutputDevices, setAudioOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [audioOutputDevicesError, setAudioOutputDevicesError] = useState<string | null>(null);
  const [audioOutputDevicesRefreshing, setAudioOutputDevicesRefreshing] = useState(false);
  const audioOutputDevicesRefreshInFlightRef = useRef(false);
  const [audioOutputBusy, setAudioOutputBusy] = useState(false);
  const [audioOutputDeviceId, setAudioOutputDeviceId] = useState<string | null>(null);
  const [audioOutputDeviceIdInput, setAudioOutputDeviceIdInput] = useState<string | null>(null);
  const [audioOutputMemoryKib, setAudioOutputMemoryKib] = useState(PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB);
  const [audioOutputMemoryKibInput, setAudioOutputMemoryKibInput] = useState<number | null>(
    PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB,
  );
  const [audioOutputHostApiFilter, setAudioOutputHostApiFilter] = useState<string>('all');
  const audioOutputDraftTouchedRef = useRef(false);
  const audioOutputDraftDirty = audioOutputDeviceIdInput !== audioOutputDeviceId
    || audioOutputMemoryKibInput !== audioOutputMemoryKib
    || (audioOutputHostApiFilter !== 'all' && audioOutputDeviceIdInput === null);
  const hasAsioOutputDevice = audioOutputDevices.some(
    (device) => device.host_api.trim().toLowerCase() === 'asio',
  );
  const autoPortAudioAttemptRef = useRef({ key: null as string | null, startedAtMs: 0, inFlight: false });
  const autoPortAudioRetryTimerRef = useRef<number | null>(null);
  const [autoPortAudioRetryRevision, setAutoPortAudioRetryRevision] = useState(0);

  useLayoutEffect(() => {
    audioOutputBackendRef.current = audioOutputBackend;
  }, [audioOutputBackend]);

  useLayoutEffect(() => {
    audioOutputDraftTouchedRef.current = audioOutputDraftDirty;
  }, [audioOutputDraftDirty]);

  async function refreshAudioOutputDevices(isCancelled: () => boolean = () => false) {
    if (audioOutputDevicesRefreshInFlightRef.current) return;
    audioOutputDevicesRefreshInFlightRef.current = true;
    if (!isCancelled()) setAudioOutputDevicesRefreshing(true);
    try {
      const devices = await invoke<unknown>('list_audio_output_devices');
      if (isCancelled()) return;
      if (!isAudioOutputDeviceList(devices)) {
        setAudioOutputDevicesError('PortAudio 输出设备列表响应无效');
        return;
      }
      setAudioOutputDevices(devices);
      setAudioOutputDevicesError(null);
    } catch (cause) {
      if (!isCancelled()) {
        setAudioOutputDevicesError(getDisplayErrorMessage(cause, 'PortAudio 输出设备枚举失败'));
      }
    } finally {
      audioOutputDevicesRefreshInFlightRef.current = false;
      if (!isCancelled()) setAudioOutputDevicesRefreshing(false);
    }
  }

  function syncAudioOutputConfiguration(status: AudioOutputBackendStatus, forceDraft = false) {
    const deviceId = typeof status.device_index === 'number' ? String(status.device_index) : null;
    setAudioOutputDeviceId(deviceId);
    setAudioOutputMemoryKib(status.memory_buffer_kib);
    if (forceDraft || !audioOutputDraftTouchedRef.current) {
      setAudioOutputDeviceIdInput(deviceId);
      setAudioOutputMemoryKibInput(status.memory_buffer_kib);
    }
  }

  function publishAudioOutputBackend(status: AudioOutputBackendStatus) {
    if (getActualAudioOutputLabel(status) === 'PortAudio') {
      syncAudioOutputConfiguration(status);
    }
    setAudioOutputBackend(status);
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
  ): Promise<AudioOutputBackendStatus | null> {
    setAudioOutputBusy(true);
    try {
      const syncClock = resolveAudioSyncClock(snapshot, mediaState, null);
      if (preferPortaudio && !syncClock) throw new Error('当前播放尚未建立绝对媒体时钟');
      const sampleRateHz = PORTAUDIO_SAMPLE_RATE_HZ;
      const status = await invokeAudioOutputBackendStatus('set_audio_output_backend', {
        request: {
          prefer_portaudio: preferPortaudio,
          device_index: deviceId !== null && deviceId !== '' ? Number(deviceId) : null,
          memory_buffer_kib: memoryBufferKib,
          sample_rate_hz: sampleRateHz,
          ...(syncClock ?? {}),
        },
      });
      publishAudioOutputBackend(status);
      if (
        classifyAudioOutputSync(status) === 'fallback'
        && status.preferred_portaudio
        && status.selected_backend !== 'portaudio'
      ) {
        setError(status.reason ?? 'PortAudio 启动失败，已回退 WebView');
      }
      return status;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '切换音频出口失败');
      return null;
    } finally {
      setAudioOutputBusy(false);
    }
  }
  const [mediaEffectParams, setMediaEffectParams] = useState<MediaEffectParams | null>(null);
  const [ambientSoundPath, setAmbientSoundPath] = useState<string | null>(null);
  const [cacheCleanup, setCacheCleanup] = useState<CacheCleanupResult | null>(null);
  const [cacheCleanupError, setCacheCleanupError] = useState<string | null>(null);
  const [mediaProcessingBusy, setMediaProcessingBusy] = useState<'video' | null>(null);
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
  const processingSwitchRequestRef = useRef(0);
  const processingSwitchQueueRef = useRef<Promise<void>>(Promise.resolve());
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const mediaStateRef = useRef<PlaybackMediaStateMessage | null>(null);
  const pictureInPictureVideoRef = useRef<HTMLVideoElement | null>(null);
  const pictureInPictureOperationRef = useRef(0);
  const pictureInPictureLoadedSourceRef = useRef<string | null>(null);
  const runtimeMessageRef = useRef<RuntimeParameterMessage | null>(null);
  const runtimeSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const audioSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const mediaCyclePlanIdRef = useRef(0);
  const mediaCandidateSequenceRef = useRef(0);
  const mediaCycleClockIdentityRef = useRef<string | null>(null);
  const audioCycleRetryRef = useRef(createAudioCycleRetryState());
  const videoCycleRetryRef = useRef(createCycleRetryState());
  const audioFuturePlansRef = useRef<MediaCycleQueue<PlannedAudioCyclePayload> | null>(null);
  const videoFuturePlansRef = useRef<MediaCycleQueue<PlannedVideoCyclePayload> | null>(null);
  const pendingMediaApplyRef = useRef<{
    params: MediaEffectParams;
    mediaCandidate: PreparedVideoMediaCandidate;
  } | null>(null);
  const videoPrepareRetryRef = useRef(createCycleRetryState());
  const videoPrepareRetryTimerRef = useRef<number | null>(null);
  const videoPrepareRetryCandidateRef = useRef<PreparedVideoMediaCandidate | null>(null);
  const activeVideoRenderRef = useRef<PreparedVideoMediaCandidate | null>(null);
  const activeAudioRenderRef = useRef<PreparedAudioMediaCandidate | null>(null);
  const audioCandidatePrepareInFlightRef = useRef(false);
  const mediaApplyInFlightRef = useRef(false);
  const realtimeVideoCommitInFlightRef = useRef(false);
  const mediaEffectParamsRef = useRef<MediaEffectParams | null>(null);
  const mediaEffectParamsMutationVersionRef = useRef(0);
  const audioProcessingEnabledRef = useRef(false);
  const videoProcessingEnabledRef = useRef(false);
  const snapshotRefHome = useRef<PlaybackSnapshot | null>(null);
  const audioDiagnosticDisplayRef = useRef<AudioDiagnosticDisplayHandle | null>(null);

  const refreshRuntimeResourceCapabilities = useCallback((
    components: readonly RuntimeResourceComponent[],
    expectedActionToken: number,
  ) => {
    if (components.includes('media')) {
      void invoke<unknown>('get_media_engine_capabilities')
        .then((capabilities) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setMediaEngineCapabilities(
              isMediaEngineCapabilities(capabilities) ? capabilities : null,
            );
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
      void invoke<unknown>('get_media_video_backend_status')
        .then((status) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setMediaVideoBackendStatus(parseMediaVideoBackendStatus(status));
          }
        })
        .catch((cause) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setMediaVideoBackendStatus(null);
            setError(getDisplayErrorMessage(cause, '读取视频实际运行后端失败'));
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
    const pendingCancelled = pending !== null
      && pending.component === nextStatus.component
      && pending.token === runtimeResourceActionTokenRef.current
      && (nextStatus.state === 'failed' || nextStatus.state === 'cancelled');
    pendingRuntimeActionRef.current = pendingCancelled ? null : resolution.pending;
    if (pendingCancelled) {
      pending.cancel?.(nextStatus);
      return;
    }
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
    cancel?: (status: RuntimeResourceStatus) => void,
  ) => {
    const token = runtimeResourceActionTokenRef.current + 1;
    runtimeResourceActionTokenRef.current = token;
    pendingRuntimeActionRef.current = { component, resume, cancel, token };
    try {
      const currentStatus = await invokeRuntimeResourceStatus('get_runtime_resource_status', { component });
      if (runtimeResourceActionTokenRef.current !== token) return;
      await applyRuntimeResourceStatus(currentStatus, token);
      const currentDecision = runtimeResourceEnsureDecision(component, currentStatus);
      if (currentDecision === 'resume' || currentDecision === 'wait') return;
      if (currentDecision === 'conflict') {
        pendingRuntimeActionRef.current = null;
        throw new RuntimeResourceConflictError('另一项媒体能力操作正在执行，请稍后再试');
      }
      const installingStatus = await invokeRuntimeResourceStatus('install_runtime_resources', { component });
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
    void invokeRuntimeResourceStatus('get_runtime_resource_status', { component: 'media' })
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
      void invokeRuntimeResourceStatus('get_runtime_resource_status', { component })
        .then((status) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourcePollGenerationRef.current === pollGeneration
          ) {
            return applyRuntimeResourceStatus(status, pending?.token);
          }
          return undefined;
        })
        .catch(() => {
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
  const [, setRuntimeLastChangeMs] = useState<number | null>(null);
  const [audioVariationCycle, setAudioVariationCycle] = useState(0);
  const [, setAudioLastChangeMs] = useState<number | null>(null);
  // 每个预设显式包含 35 个可写字段；默认勾选 p01–p20；mix 默认关（单轨）；会话可持久化
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
  const [audioPresetDrawerOpen, setAudioPresetDrawerOpen] = useState(false);
  const [interludePresetDrawerOpen, setInterludePresetDrawerOpen] = useState(false);
  const [audioSettingsDrawerOpen, setAudioSettingsDrawerOpen] = useState(false);
  const [interludeDrawerOpen, setInterludeDrawerOpen] = useState(false);
  const [fixedSpeechDrawerOpen, setFixedSpeechDrawerOpen] = useState(false);

  useEffect(() => {
    setAudioSettingsDrawerOpen(initialRoute === 'settings');
  }, [initialRoute]);
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
    refreshAudioFutureMediaCyclePlansAfterSettingsChange();
  }, [audioValuePresetIds, audioMixEnabled, audioMixPickMin, audioMixPickMax]);

  useEffect(() => {
    mediaEffectParamsRef.current = mediaEffectParams;
  }, [mediaEffectParams]);
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
      applyAudioToMediaEffectParams?: boolean;
      baseParams?: MediaEffectParams;
      randomChangePeriodMs?: number;
    } = {},
  ) {
    const {
      applyAudioToMediaEffectParams = true,
      baseParams,
      randomChangePeriodMs,
    } = options;
    audioCycleSampleRef.current = sample;
    setAudioActivePresetIds(sample.presetIds);
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
        // 历史快照结构只存数值；字符串、布尔和 null 可由稳定预设 ID 复原。
        values: Object.fromEntries(
          Object.entries(sample.values).filter(
            (entry): entry is [string, number] => typeof entry[1] === 'number',
          ),
        ),
      });
      lastAudioCycleSnapshotSignatureRef.current = snapshotSignature;
    }
    if (!applyAudioToMediaEffectParams) return sample;
    const source = baseParams ?? mediaEffectParamsRef.current;
    if (!source) return sample;
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
    mediaEffectParamsRef.current = next;
    setMediaEffectParams(next);
    return sample;
  }

  function sampleAndCommitAudioCycle(options: {
    applyAudioToMediaEffectParams?: boolean;
    baseParams?: MediaEffectParams;
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

  function nextMediaCyclePlanId(kind: 'audio' | 'video'): string {
    mediaCyclePlanIdRef.current += 1;
    return `${kind}-${mediaCyclePlanIdRef.current}`;
  }

  function buildAudioCycleSeed(
    periodMs: number,
    previousPresetIds: readonly string[] | undefined,
    planId = nextMediaCyclePlanId('audio'),
  ): MediaCycleSeed<PlannedAudioCyclePayload> | null {
    const base = mediaEffectParamsRef.current;
    if (!base) return null;
    const sample = sampleAudioCycle(audioValuePresetIdsRef.current, {
      mixEnabled: audioMixEnabledRef.current,
      pickMin: audioMixPickMinRef.current,
      pickMax: audioMixPickMaxRef.current,
      previousPresetIds,
    });
    const audio = {
      ...base.audio,
      ...sample.values,
      random_change_period_ms: periodMs,
    };
    const audioVariants = audioMixEnabledRef.current && sample.variants.length > 1
      ? buildAudioVariantsFromCycle(audio, sample)
      : [];
    return {
      planId,
      periodMediaMs: periodMs,
      payload: { sample, audio, audioVariants, periodMs },
    };
  }

  function buildVideoCycleSeed(
    periodMs: number,
    planId = nextMediaCyclePlanId('video'),
  ): MediaCycleSeed<PlannedVideoCyclePayload> {
    const base = mediaEffectParamsRef.current;
    if (!base) {
      throw new Error('生成视频周期计划前必须先加载媒体参数');
    }
    const videoEffectsEnabled = videoProcessingEnabledRef.current;
    const sample = videoEffectsEnabled ? sampleAutomaticVideoParameters() : null;
    return {
      planId,
      periodMediaMs: periodMs,
      payload: {
        seed: sample?.seed ?? 0,
        videoEffectsEnabled,
        skipVideoProcessing: false,
        video: sample ? { ...base.video, ...sample.video } : base.video,
        advanced: sample ? { ...base.advanced, ...sample.advanced } : base.advanced,
        periodMs,
      },
    };
  }

  function createVideoMediaCycleQueue(
    currentAbsolutePositionMs: number,
    seeds: readonly [
      MediaCycleSeed<PlannedVideoCyclePayload>,
      MediaCycleSeed<PlannedVideoCyclePayload>,
    ],
    startSequence: number,
    currentSnapshot: PlaybackSnapshot | null,
  ): MediaCycleQueue<PlannedVideoCyclePayload> {
    const source = currentSnapshot?.source_media;
    const sourceDurationMs = source?.duration_ms;
    const alignTarget = (targetAbsolutePositionMs: number) => alignMediaPositionToVideoFrame(
      targetAbsolutePositionMs,
      source?.frame_rate_fps,
    );
    const singleSourceLoop = currentSnapshot?.source_media_pool.length === 1
      && source?.media_kind === 'video'
      && typeof sourceDurationMs === 'number'
      && Number.isSafeInteger(sourceDurationMs)
      && sourceDurationMs > 0;
    if (!singleSourceLoop || !source || typeof sourceDurationMs !== 'number') {
      return createMediaCycleQueue(currentAbsolutePositionMs, seeds, startSequence, alignTarget);
    }
    const targets = resolveSourceBoundedVideoCycleQueueTargets(
      currentAbsolutePositionMs,
      seeds[0].periodMediaMs,
      seeds[1].periodMediaMs,
      sourceDurationMs,
      alignTarget,
    );
    const boundSeed = (
      startAbsolutePositionMs: number,
      seed: MediaCycleSeed<PlannedVideoCyclePayload>,
      target: (typeof targets)[number],
    ): MediaCycleSeed<PlannedVideoCyclePayload> => {
      const periodMediaMs = target.targetAbsolutePositionMs - startAbsolutePositionMs;
      return {
        ...seed,
        periodMediaMs,
        payload: {
          ...seed.payload,
          periodMs: periodMediaMs,
          skipVideoProcessing: target.skipVideoProcessing,
        },
      };
    };
    const first = boundSeed(currentAbsolutePositionMs, seeds[0], targets[0]);
    const second = boundSeed(targets[0].targetAbsolutePositionMs, seeds[1], targets[1]);
    return createMediaCycleQueue(currentAbsolutePositionMs, [first, second], startSequence);
  }

  function clearAudioFutureMediaCyclePlans() {
    audioFuturePlansRef.current = null;
    audioCycleRetryRef.current = clearAudioCycleRetry();
  }

  function discardAudioFutureMediaCyclePlans() {
    clearAudioFutureMediaCyclePlans();
    activeAudioRenderRef.current = null;
  }

  function refreshAudioFutureMediaCyclePlansAfterSettingsChange() {
    audioCycleRetryRef.current = clearAudioCycleRetry();
    const queue = audioFuturePlansRef.current;
    const active = activeAudioRenderRef.current;
    if (
      !queue
      || !active
      || active.timeline.planId !== queue[0].planId
      || active.timeline.sequence !== queue[0].sequence
    ) {
      audioFuturePlansRef.current = null;
      return;
    }
    const next = queue[1];
    const refreshed = buildAudioCycleSeed(
      next.periodMediaMs,
      queue[0].payload.sample.presetIds,
      next.planId,
    );
    audioFuturePlansRef.current = refreshed
      ? [queue[0], { ...next, payload: refreshed.payload }]
      : null;
  }

  function scheduleAudioCycleRetry() {
    const identity = mediaCycleClockIdentityRef.current;
    audioFuturePlansRef.current = null;
    if (!identity) return;
    audioCycleRetryRef.current = recordAudioCycleRetryFailure(
      audioCycleRetryRef.current,
      identity,
      Date.now(),
    );
  }

  function scheduleVideoCycleRetry() {
    const identity = mediaCycleClockIdentityRef.current;
    videoFuturePlansRef.current = null;
    if (!identity) return;
    videoCycleRetryRef.current = recordCycleRetryFailure(
      videoCycleRetryRef.current,
      identity,
      Date.now(),
    );
  }

  function wakeMediaCycleScheduling(_reason: 'healthy' | 'switch' | 'retry' = 'healthy') {
    const clock = mediaStateRef.current;
    const currentSnapshot = snapshotRefHome.current;
    if (!clock || !currentSnapshot) return;
    if (
      currentSnapshot.playback_state?.toLowerCase() !== 'playing'
      || clock.playback_generation !== currentSnapshot.playback_generation
      || clock.paused
      || clock.clock_health !== 'healthy'
    ) return;
    const audioActive = audioProcessingEnabledRef.current;
    const videoActive = currentSnapshot.source_media?.media_kind === 'video';
    if (!audioActive && !videoActive) return;
    if (!initializeFutureMediaCyclePlans(clock, audioActive, videoActive)) return;
    void flushPendingMediaApply();
    void prepareNextAudioMediaCandidate();
    prepareNextVideoMediaCandidate();
  }

  function resetVideoPrepareRetry(candidate?: PreparedVideoMediaCandidate) {
    if (
      candidate
      && videoPrepareRetryCandidateRef.current
      && videoPrepareRetryCandidateRef.current !== candidate
    ) return;
    if (videoPrepareRetryTimerRef.current !== null) {
      window.clearTimeout(videoPrepareRetryTimerRef.current);
      videoPrepareRetryTimerRef.current = null;
    }
    videoPrepareRetryRef.current = clearCycleRetry();
    videoPrepareRetryCandidateRef.current = null;
  }

  function cancelVideoPrepareRetry() {
    const candidate = videoPrepareRetryCandidateRef.current;
    if (candidate && pendingMediaApplyRef.current?.mediaCandidate === candidate) {
      pendingMediaApplyRef.current = null;
    }
    if (candidate && activeVideoRenderRef.current === candidate) {
      activeVideoRenderRef.current = null;
    }
    resetVideoPrepareRetry();
  }

  function scheduleVideoPrepareRetry(
    params: MediaEffectParams,
    candidate: PreparedVideoMediaCandidate,
  ) {
    const clockIdentity = mediaCycleClockIdentityRef.current;
    if (!clockIdentity) {
      cancelVideoPrepareRetry();
      return;
    }
    const retryIdentity = `${clockIdentity}:${candidate.timeline.planId}:${candidate.timeline.sequence}`;
    const retry = recordCycleRetryFailure(
      videoPrepareRetryRef.current,
      retryIdentity,
      Date.now(),
    );
    videoPrepareRetryRef.current = retry;
    videoPrepareRetryCandidateRef.current = candidate;
    pendingMediaApplyRef.current = { params, mediaCandidate: candidate };
    if (retry.exhausted || retry.retryAtMs === null) {
      pendingMediaApplyRef.current = null;
      if (activeVideoRenderRef.current === candidate) activeVideoRenderRef.current = null;
      videoCycleRetryRef.current = recordCycleRetryFailure(
        videoCycleRetryRef.current,
        clockIdentity,
        Date.now(),
      );
      videoFuturePlansRef.current = null;
      setError(videoCycleRetryRef.current.exhausted
        ? '视频处理连续失败，已停止自动重试；可继续播放源视频并手动重试。'
        : '视频 prepare 连续瞬时失败，稍后自动重建候选。');
      return;
    }
    if (videoPrepareRetryTimerRef.current !== null) {
      window.clearTimeout(videoPrepareRetryTimerRef.current);
    }
    videoPrepareRetryTimerRef.current = window.setTimeout(() => {
      videoPrepareRetryTimerRef.current = null;
      if (
        videoPrepareRetryRef.current.identity !== retryIdentity
        || mediaCycleClockIdentityRef.current !== clockIdentity
        || !isVideoCandidatePlaybackActive(candidate)
      ) {
        cancelVideoPrepareRetry();
        return;
      }
      void flushPendingMediaApply();
    }, Math.max(0, retry.retryAtMs - Date.now()));
  }

  function clearVideoFutureMediaCyclePlans() {
    resetVideoPrepareRetry();
    videoFuturePlansRef.current = null;
    activeVideoRenderRef.current = null;
    pendingMediaApplyRef.current = null;
    videoCycleRetryRef.current = clearCycleRetry();
  }

  function videoRenderIdentity(
    planId: string,
    sequence: number,
    playbackGeneration: number,
  ): string {
    return `${playbackGeneration}:${sequence}:${planId}`;
  }

  function logVideoPeriodStage(
    candidate: PreparedVideoMediaCandidate,
    stage: 'plan' | 'prepare',
    result: 'scheduled' | 'queued' | 'accepted' | 'not_started',
  ) {
    console.info('[video-period]', {
      plan: candidate.timeline.planId,
      sequence: candidate.timeline.sequence,
      generation: candidate.playbackGeneration,
      stage,
      result,
    });
  }

  function clearFutureMediaCyclePlans() {
    discardAudioFutureMediaCyclePlans();
    clearVideoFutureMediaCyclePlans();
    mediaCycleClockIdentityRef.current = null;
  }

  function isMediaProcessingPlaybackActive(playbackGeneration: number) {
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    if (!currentSnapshot || !clock) return false;
    return currentSnapshot?.playback_state?.toLowerCase() === 'playing'
      && currentSnapshot.playback_generation === playbackGeneration
      && clock.playback_generation === playbackGeneration
      && !clock.paused
      && clock.clock_health === 'healthy';
  }

  function isVideoCandidatePlaybackActive(candidate: PreparedVideoMediaCandidate) {
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    if (
      !currentSnapshot
      || !clock
      || currentSnapshot.playback_state?.toLowerCase() !== 'playing'
      || clock.playback_generation !== currentSnapshot.playback_generation
      || clock.paused
      || clock.clock_health !== 'healthy'
    ) return false;
    return isVideoCandidateBoundToCurrentSource(candidate, currentSnapshot);
  }

  function isVideoCandidateBoundToCurrentSource(
    candidate: PreparedVideoMediaCandidate,
    currentSnapshot: PlaybackSnapshot,
  ) {
    const currentIndex = currentSnapshot.source_media_index;
    if (currentIndex === null) return false;
    return candidate.playbackGeneration === currentSnapshot.playback_generation
      && candidate.sourceMediaIndex === currentIndex
      && currentSnapshot.source_media?.source_path === candidate.sourcePath;
  }

  function initializeFutureMediaCyclePlans(
    clock: PlaybackMediaStateMessage,
    audioActive: boolean,
    videoActive: boolean,
  ): boolean {
    const currentSnapshot = snapshotRefHome.current;
    const identity = `${clock.playback_generation}:${clock.source_revision}:${clock.clock_epoch}:${currentSnapshot?.loop_index ?? clock.loop_index}`;
    if (mediaCycleClockIdentityRef.current !== identity) {
      clearFutureMediaCyclePlans();
      mediaCycleClockIdentityRef.current = identity;
    }
    const audioCandidateActive = audioActive && isAudioCycleRetryReady(
      audioCycleRetryRef.current,
      identity,
      Date.now(),
    );
    const videoCandidateActive = videoActive && isCycleRetryReady(
      videoCycleRetryRef.current,
      identity,
      Date.now(),
    );
    const nextAudioSequence = Math.max(
      1,
      (currentSnapshot?.current_audio_media_sequence ?? 0) + 1,
      (currentSnapshot?.pending_audio_media_sequence ?? 0) + 1,
    );
    const nextVideoSequence = Math.max(
      1,
      mediaCandidateSequenceRef.current + 1,
    );
    if (!audioCandidateActive) audioFuturePlansRef.current = null;
    if (!videoCandidateActive) videoFuturePlansRef.current = null;
    if (audioCandidateActive && videoCandidateActive && !audioFuturePlansRef.current && !videoFuturePlansRef.current) {
      const firstAudioPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      const secondAudioPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      const firstAudio = buildAudioCycleSeed(firstAudioPeriod, audioCycleSampleRef.current?.presetIds);
      const secondAudio = buildAudioCycleSeed(secondAudioPeriod, firstAudio?.payload.sample.presetIds);
      if (!firstAudio || !secondAudio) return false;
      const firstVideoPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
      const secondVideoPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
      const startSequence = Math.max(nextAudioSequence, nextVideoSequence);
      audioFuturePlansRef.current = createMediaCycleQueue(
        clock.absolute_position_ms,
        [firstAudio, secondAudio],
        startSequence,
      );
      videoFuturePlansRef.current = createVideoMediaCycleQueue(
        clock.absolute_position_ms,
        [buildVideoCycleSeed(firstVideoPeriod), buildVideoCycleSeed(secondVideoPeriod)],
        startSequence,
        currentSnapshot,
      );
    } else {
      if (audioCandidateActive && !audioFuturePlansRef.current) {
        const firstPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
        const secondPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
        const first = buildAudioCycleSeed(firstPeriod, audioCycleSampleRef.current?.presetIds);
        const second = buildAudioCycleSeed(secondPeriod, first?.payload.sample.presetIds);
        if (!first || !second) return false;
        audioFuturePlansRef.current = createMediaCycleQueue(
          clock.absolute_position_ms,
          [first, second],
          nextAudioSequence,
        );
      }
      if (videoCandidateActive && !videoFuturePlansRef.current) {
        const firstPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
        const secondPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
        videoFuturePlansRef.current = createVideoMediaCycleQueue(
          clock.absolute_position_ms,
          [buildVideoCycleSeed(firstPeriod), buildVideoCycleSeed(secondPeriod)],
          nextVideoSequence,
          currentSnapshot,
        );
      }
    }
    const nextAudio = audioFuturePlansRef.current?.[0];
    const nextVideo = videoFuturePlansRef.current?.[0];
    if (nextAudio) {
      nextAudioPeriodMsRef.current = nextAudio.periodMediaMs;
      setAudioPeriodMs(nextAudio.periodMediaMs);
    }
    if (nextVideo) {
      mediaCandidateSequenceRef.current = Math.max(
        mediaCandidateSequenceRef.current,
        videoFuturePlansRef.current?.[1].sequence ?? nextVideo.sequence,
      );
      nextVideoPeriodMsRef.current = nextVideo.periodMediaMs;
      setVideoPeriodMs(nextVideo.periodMediaMs);
    }
    queueMicrotask(() => {
      void prepareNextAudioMediaCandidate();
      prepareNextVideoMediaCandidate();
    });
    return true;
  }

  function createPlannedArtifactTimeline(
    plan: MediaCyclePlan<unknown>,
    nextTargetAbsolutePositionMs: number,
    currentSnapshot: PlaybackSnapshot,
    sourceRevision: number,
    safetyTailMs = MEDIA_CANDIDATE_SAFETY_TAIL_MS,
  ): { timeline: MediaArtifactTimeline; loopSource: boolean } | null {
    const sourceDurationMs = currentSnapshot.source_media?.duration_ms;
    if (
      typeof sourceDurationMs !== 'number'
      || !Number.isSafeInteger(sourceDurationMs)
      || sourceDurationMs <= 0
    ) return null;
    const loopSource = currentSnapshot.source_media_pool.length === 1;
    const sourceWindowEndMs = plan.targetAbsolutePositionMs
      - (plan.targetAbsolutePositionMs % sourceDurationMs)
      + sourceDurationMs;
    const validUntilAbsolutePositionMs = loopSource
      ? nextTargetAbsolutePositionMs
      : Math.min(nextTargetAbsolutePositionMs, sourceWindowEndMs);
    if (validUntilAbsolutePositionMs <= plan.targetAbsolutePositionMs) return null;
    const boundedSafetyTailMs = loopSource
      ? safetyTailMs
      : Math.min(safetyTailMs, sourceWindowEndMs - validUntilAbsolutePositionMs);
    return {
      loopSource,
      timeline: createMediaArtifactTimeline({
        planId: plan.planId,
        sequence: plan.sequence,
        playbackGeneration: currentSnapshot.playback_generation,
        sourceRevision,
        targetAbsolutePositionMs: plan.targetAbsolutePositionMs,
        validUntilAbsolutePositionMs,
        sourceDurationMs,
        safetyTailMs: boundedSafetyTailMs,
      }),
    };
  }

  async function prepareNextAudioMediaCandidate() {
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    const source = currentSnapshot?.source_media;
    const queue = audioFuturePlansRef.current;
    if (
      audioCandidatePrepareInFlightRef.current
      || !currentSnapshot
      || !clock
      || !source
      || !queue
      || clock.playback_generation !== currentSnapshot.playback_generation
    ) return;
    const plan = queue[0];
    if (activeAudioRenderRef.current) return;
    const sourceDurationMs = source.duration_ms;
    if (!Number.isSafeInteger(sourceDurationMs) || sourceDurationMs === null || sourceDurationMs <= 0) return;
    const planned = createPlannedArtifactTimeline(
      plan,
      queue[1].targetAbsolutePositionMs,
      currentSnapshot,
      currentSnapshot.audio_stream_revision,
    );
    if (!planned) return;
    if (!doesAudioWindowOverlapArtifact({
      sourceStartMs: planned.timeline.sourceStartMs,
      outputDurationMs: planned.timeline.outputDurationMs,
      sourceDurationMs,
      audioStartMs: source.audio_start_ms,
      audioEndMs: source.audio_end_ms,
      loopSource: planned.loopSource,
    })) {
      // 多项池等待真实换源重建队列，不能把当前素材误当成下一轮自循环。
      if (planned.loopSource) {
        advanceIndependentAudioQueue(planned.timeline.validUntilAbsolutePositionMs, false);
      }
      return;
    }
    const candidate: PreparedAudioMediaCandidate = {
      audioCyclePlan: plan,
      timeline: planned.timeline,
      sourcePath: source.source_path,
      playbackGeneration: currentSnapshot.playback_generation,
      loopSource: planned.loopSource,
      artifactReference: null,
    };
    activeAudioRenderRef.current = candidate;
    await prepareAudioMediaCandidate(candidate);
  }

  async function prepareAudioMediaCandidate(candidate: PreparedAudioMediaCandidate) {
    audioCandidatePrepareInFlightRef.current = true;
    try {
      const nextSnapshot = await invokePlaybackSnapshot('prepare_audio_media_candidate', {
        request: {
          params: candidate.audioCyclePlan.payload.audio,
          audio_variants: candidate.audioCyclePlan.payload.audioVariants,
          ambient_sound_path: ambientSoundPath,
          plan_id: candidate.timeline.planId,
          sequence: candidate.timeline.sequence,
          playback_generation: candidate.timeline.playbackGeneration,
          source_revision: candidate.timeline.sourceRevision,
          source_start_ms: candidate.timeline.sourceStartMs,
          output_duration_ms: candidate.timeline.outputDurationMs,
          loop_source: candidate.loopSource,
          target_absolute_position_ms: candidate.timeline.targetAbsolutePositionMs,
          valid_until_absolute_position_ms: candidate.timeline.validUntilAbsolutePositionMs,
        },
      });
      if (!isMediaProcessingPlaybackActive(candidate.playbackGeneration)) return;
      const pendingRevision = nextSnapshot.pending_audio_media_source_revision;
      if (
        nextSnapshot.pending_audio_media_plan_id === candidate.timeline.planId
        && nextSnapshot.pending_audio_media_sequence === candidate.timeline.sequence
        && nextSnapshot.pending_audio_media_playback_generation === candidate.timeline.playbackGeneration
        && typeof pendingRevision === 'number'
        && Number.isSafeInteger(pendingRevision)
        && pendingRevision >= 0
      ) {
        candidate.timeline = { ...candidate.timeline, sourceRevision: pendingRevision };
      }
      snapshotRefHome.current = nextSnapshot;
      setSnapshot(nextSnapshot);
      if (nextSnapshot.audio_processing_status === 'failed') {
        activeAudioRenderRef.current = null;
        scheduleAudioCycleRetry();
        if (nextSnapshot.fallback_reason) setError(nextSnapshot.fallback_reason);
      }
    } catch (cause) {
      if (activeAudioRenderRef.current === candidate) activeAudioRenderRef.current = null;
      if (isMediaWorkerBusyError(cause)) return;
      if (!isMediaProcessingPlaybackActive(candidate.playbackGeneration)) return;
      if (getCommandErrorCode(cause) === AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE) {
        // Rust 的同源元数据防线可能在前端快照更新竞态中先命中；这是静默跳过，不是处理失败。
        if (candidate.loopSource) {
          advanceIndependentAudioQueue(candidate.timeline.validUntilAbsolutePositionMs, false);
        }
        return;
      }
      scheduleAudioCycleRetry();
      setError(getDisplayErrorMessage(cause, '准备声音周期候选失败'));
    } finally {
      audioCandidatePrepareInFlightRef.current = false;
    }
  }

  function prepareNextVideoMediaCandidate() {
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    const source = currentSnapshot?.source_media;
    const base = mediaEffectParamsRef.current;
    const queue = videoFuturePlansRef.current;
    if (
      !currentSnapshot
      || !clock
      || !source
      || source.media_kind !== 'video'
      || !base
      || !queue
      || clock.playback_generation !== currentSnapshot.playback_generation
    ) return;
    if (
      activeVideoRenderRef.current
      || mediaApplyInFlightRef.current
      || pendingRuntimeActionRef.current?.component === 'media'
    ) return;
    const plan = queue[0];
    if (plan.payload.videoEffectsEnabled !== videoProcessingEnabledRef.current) {
      clearVideoFutureMediaCyclePlans();
      return;
    }
    const planned = createPlannedArtifactTimeline(
      plan,
      queue[1].targetAbsolutePositionMs,
      currentSnapshot,
      clock.source_revision,
      0,
    );
    if (!planned) return;
    const params = {
      ...base,
      video: plan.payload.video,
      advanced: plan.payload.advanced,
    };
    const candidate: PreparedVideoMediaCandidate = {
      params,
      videoCyclePlan: plan,
      videoEffectsEnabled: plan.payload.videoEffectsEnabled,
      realtimePrepared: false,
      // N+1 只覆盖本轮目标到 N+2 的窗口；N+2 仍只保存参数计划。
      timeline: planned.timeline,
      sourcePath: source.source_path,
      sourceMediaIndex: currentSnapshot.source_media_index ?? 0,
      playbackGeneration: currentSnapshot.playback_generation,
    };
    logVideoPeriodStage(candidate, 'plan', 'scheduled');
    void applyVideoProcessing(params, candidate);
  }

  function applyPlannedVideoCycle(plan: MediaCyclePlan<PlannedVideoCyclePayload>) {
    const currentParams = mediaEffectParamsRef.current;
    if (!currentParams) return;
    const nextParams = {
      ...currentParams,
      video: plan.payload.video,
      advanced: plan.payload.advanced,
    };
    mediaEffectParamsMutationVersionRef.current += 1;
    mediaEffectParamsRef.current = nextParams;
    setMediaEffectParams(nextParams);
    publishRuntimeParameterMessage(nextParams);
    const current = runtimeSchedulerRef.current;
    current.cycle += 1;
    current.lastChangeMs = Date.now();
    setRuntimeCycle(current.cycle);
    setRuntimeLastChangeMs(current.lastChangeMs);
    setVideoPeriodMs(plan.periodMediaMs);
    nextVideoPeriodMsRef.current = plan.periodMediaMs;
  }

  function applyPlannedAudioCycle(plan: MediaCyclePlan<PlannedAudioCyclePayload>) {
    commitAudioCycleSample(plan.payload.sample, {
      randomChangePeriodMs: plan.periodMediaMs,
    });
    const current = audioSchedulerRef.current;
    current.cycle += 1;
    current.lastChangeMs = Date.now();
    setAudioVariationCycle(current.cycle);
    setAudioLastChangeMs(current.lastChangeMs);
    setAudioPeriodMs(plan.periodMediaMs);
    nextAudioPeriodMsRef.current = plan.periodMediaMs;
  }

  function advanceIndependentAudioQueue(
    committedAtAbsolutePositionMs: number,
    prepareCandidate = true,
  ) {
    const queue = audioFuturePlansRef.current;
    if (!queue) return;
    const period = samplePeriodMsInRange(audioPeriodRangeRef.current);
    const seed = buildAudioCycleSeed(period, queue[1].payload.sample.presetIds);
    if (!seed) {
      audioFuturePlansRef.current = null;
      return;
    }
    audioFuturePlansRef.current = createMediaCycleQueue(
      committedAtAbsolutePositionMs,
      [queue[1], seed],
      queue[1].sequence,
    );
    if (prepareCandidate) queueMicrotask(() => void prepareNextAudioMediaCandidate());
  }

  function advanceIndependentVideoQueue(
    committedAtAbsolutePositionMs: number,
    prepareCandidate = true,
  ) {
    const queue = videoFuturePlansRef.current;
    if (!queue) return;
    const period = samplePeriodMsInRange(videoPeriodRangeRef.current);
    videoFuturePlansRef.current = createVideoMediaCycleQueue(
      committedAtAbsolutePositionMs,
      [queue[1], buildVideoCycleSeed(period)],
      queue[1].sequence,
      snapshotRefHome.current,
    );
    if (prepareCandidate) queueMicrotask(() => prepareNextVideoMediaCandidate());
  }

  const [runtimeChannelError, setRuntimeChannelError] = useState<string | null>(null);
  const [diagnosticSummary, setDiagnosticSummary] = useState<AudioDiagnosticSummary>({
    fresh: false,
    message: null,
  });
  const [mediaState, setMediaState] = useState<PlaybackMediaStateMessage | null>(null);
  const [mediaSeekDraft, setMediaSeekDraft] = useState<{
    value: number;
    playbackGeneration: number;
    committed: boolean;
  } | null>(null);
  const [pictureInPictureActive, setPictureInPictureActive] = useState(false);
  const currentMediaIsVideo = snapshot?.source_media?.media_kind === 'video';
  const pictureInPictureSourceUrl = currentMediaIsVideo
    ? playbackVideoUrl(snapshot)
    : null;
  const runtimeResourceBusy = runtimeResourceStatus !== null && isRuntimeResourceBusy(runtimeResourceStatus);

  useLayoutEffect(() => {
    runtimeResourceBusyRef.current = runtimeResourceBusy;
  }, [runtimeResourceBusy]);

  useEffect(() => {
    if (typeof BroadcastChannel === 'undefined') {
      setRuntimeChannelError('当前桌面运行时不支持跨窗口预览通信，媒体播放不受影响。');
      return;
    }
    let channel: BroadcastChannel;
    try {
      channel = new BroadcastChannel(PLAYBACK_CHANNEL_NAME);
    } catch {
      setRuntimeChannelError('预览通信通道创建失败，媒体播放不受影响。');
      return;
    }
    playbackChannelRef.current = channel;
    const handleMessage = (event: MessageEvent<unknown>) => {
      if (isPlaybackMediaStateMessage(event.data)) {
        const previous = mediaStateRef.current;
        if (!shouldAcceptPlaybackMediaState(previous, event.data)) return;
        mediaStateRef.current = event.data;
        setMediaState(event.data);
        commitPreparedRealtimeVideoCandidate(event.data);
        if (
          event.data.clock_health === 'healthy'
          && previous?.clock_health !== 'healthy'
        ) {
          queueMicrotask(() => wakeMediaCycleScheduling('healthy'));
        }
        return;
      }
      if (isInterludeRuntimeMessage(event.data)) {
        setInterludeRuntime(event.data);
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
                playback_watermark_ms: 0,
                actual_sample_rate_hz: null,
                audio_timeline_position_ms: null,
                av_offset_ms: null,
                callback_stalled_ms: null,
                pcm_stalled_ms: null,
                recovery_required: false,
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
                playback_watermark_ms: 0,
                actual_sample_rate_hz: null,
                audio_timeline_position_ms: null,
                av_offset_ms: null,
                callback_stalled_ms: null,
                pcm_stalled_ms: null,
                recovery_required: false,
                ring_len_samples: 0,
                ring_capacity_samples: 0,
                preferred_portaudio: false,
                running: false,
                reason: '最终效果窗已关闭',
                reason_code: null,
                retryable: false,
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
      audioDiagnosticDisplayRef.current?.acceptDiagnostic(event.data);
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
    const nextTime = clampMediaTime(mediaState.current_time, duration);
    if (!Number.isFinite(video.currentTime) || Math.abs(video.currentTime - nextTime) > 0.25) {
      video.currentTime = nextTime;
    }
    video.volume = mediaState.volume;
    video.muted = true;
    if (mediaState.paused) {
      video.pause();
    } else if (video.paused) {
      void video.play().catch(() => undefined);
    }
  }

  function releasePictureInPictureVideo(
    video = pictureInPictureVideoRef.current,
    updateState = true,
  ) {
    pictureInPictureOperationRef.current += 1;
    pictureInPictureLoadedSourceRef.current = null;
    if (video) {
      video.pause();
      video.removeAttribute('src');
      video.preload = 'none';
      video.load();
    }
    if (updateState) setPictureInPictureActive(false);
  }

  function waitForPictureInPictureMetadata(
    video: HTMLVideoElement,
    operation: number,
  ): Promise<void> {
    if (video.readyState >= 1) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        window.clearTimeout(timer);
        video.removeEventListener('loadedmetadata', handleLoaded);
        video.removeEventListener('error', handleError);
      };
      const handleLoaded = () => {
        cleanup();
        if (pictureInPictureOperationRef.current !== operation) {
          reject(new Error('画中画加载已取消'));
          return;
        }
        resolve();
      };
      const handleError = () => {
        cleanup();
        reject(new Error('画中画视频加载失败'));
      };
      const timer = window.setTimeout(() => {
        cleanup();
        reject(new Error('画中画视频加载超时'));
      }, 5_000);
      video.addEventListener('loadedmetadata', handleLoaded);
      video.addEventListener('error', handleError);
    });
  }

  useEffect(() => {
    const video = pictureInPictureVideoRef.current;
    if (!video) return;
    const handleEnter = () => setPictureInPictureActive(true);
    const handleLeave = () => releasePictureInPictureVideo(video);
    video.addEventListener('enterpictureinpicture', handleEnter);
    video.addEventListener('leavepictureinpicture', handleLeave);
    return () => {
      video.removeEventListener('enterpictureinpicture', handleEnter);
      video.removeEventListener('leavepictureinpicture', handleLeave);
      releasePictureInPictureVideo(video, false);
    };
  }, []);

  useEffect(() => {
    if (!pictureInPictureActive) return;
    syncPictureInPictureVideo();
  }, [pictureInPictureActive, mediaState?.current_time, mediaState?.duration, mediaState?.volume]);

  useEffect(() => {
    const video = pictureInPictureVideoRef.current;
    const loadedSource = pictureInPictureLoadedSourceRef.current;
    if (!video || !loadedSource || loadedSource === pictureInPictureSourceUrl) return;
    const pipDocument = document as PictureInPictureDocument;
    if (pipDocument.pictureInPictureElement === video && typeof pipDocument.exitPictureInPicture === 'function') {
      void pipDocument.exitPictureInPicture().catch(() => undefined).finally(() => {
        releasePictureInPictureVideo(video);
      });
      return;
    }
    releasePictureInPictureVideo(video);
  }, [pictureInPictureSourceUrl]);

  const interludePlaybackActive = Boolean(
    interludeDraft.enabled
    && snapshot?.interlude?.enabled
    && interludeRuntime?.status === 'playing'
  );

  // 关最终效果窗会清 PortAudio preferred；主窗轮询对齐开关状态。
  useEffect(() => {
    if (!documentVisible) return;
    let cancelled = false;
    let outputStatusPollInFlight = false;
    const refreshOutputStatus = () => {
      if (outputStatusPollInFlight) return;
      outputStatusPollInFlight = true;
      void invokeAudioOutputBackendStatus('get_audio_output_backend_status')
        .then((status) => {
          if (cancelled) return;
          if (getActualAudioOutputLabel(status) === 'PortAudio') {
            syncAudioOutputConfiguration(status);
          }
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
              && current.playback_watermark_ms === status.playback_watermark_ms
              && current.actual_sample_rate_hz === status.actual_sample_rate_hz
              && current.audio_timeline_position_ms === status.audio_timeline_position_ms
              && current.av_offset_ms === status.av_offset_ms
              && current.callback_stalled_ms === status.callback_stalled_ms
              && current.pcm_stalled_ms === status.pcm_stalled_ms
              && current.recovery_required === status.recovery_required
              && current.ring_len_samples === status.ring_len_samples
              && current.ring_capacity_samples === status.ring_capacity_samples
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
    if (snapshot?.playback_state?.toLowerCase() !== 'playing') return;
    const timer = window.setInterval(refreshOutputStatus, AUDIO_OUTPUT_STATUS_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [documentVisible, snapshot?.playback_state]);

  const mediaWasProcessingRef = useRef(false);

  useEffect(() => {
    if (!documentVisible) return;
    let cancelled = false;
    const refreshSnapshot = async () => {
      if (snapshotPollInFlightRef.current) return;
      snapshotPollInFlightRef.current = true;
      const requestId = ++snapshotRequestRef.current;
      setSnapshotLoading(true);
      try {
        const nextSnapshot = await invokePlaybackSnapshot('get_snapshot');
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
  }, [documentVisible]);

  useEffect(() => {
    if (!snapshot) return;
    setVideoProcessingEnabled(snapshot.video_processing_enabled);
    setAudioProcessingEnabled(snapshot.audio_processing_enabled);
  }, [snapshot?.audio_processing_enabled, snapshot?.video_processing_enabled]);

  useEffect(() => {
    if (!snapshot || interludeRestoreAttemptedRef.current) return;
    interludeRestoreAttemptedRef.current = true;
    const savedInterludeConfig = savedInterludeConfigRef.current;
    if (!savedInterludeConfig) return;
    setInterludeSaving(true);
    void invokePlaybackSnapshot('set_interlude_config', { request: savedInterludeConfig })
      .then((nextSnapshot) => {
        setSnapshot(nextSnapshot);
        setInterludeDraft(buildInterludeDraft(nextSnapshot.interlude));
        setInterludeDirty(false);
        setInterludeSaveError(null);
      })
      .catch((cause) => {
        setInterludeDraft(buildInterludeDraft(savedInterludeConfig));
        setInterludeDirty(true);
        setInterludeSaveError(getDisplayErrorMessage(cause, '恢复已保存的随机插话配置失败'));
      })
      .finally(() => setInterludeSaving(false));
  }, [snapshot]);

  useEffect(() => {
    if (!interludeDirty) {
      setInterludeDraft(buildInterludeDraft(snapshot?.interlude));
    }
  }, [
    interludeDirty,
    snapshot?.interlude?.audio_fixed_preset_id,
    snapshot?.interlude?.audio_mix_enabled,
    snapshot?.interlude?.audio_mix_pick_max,
    snapshot?.interlude?.audio_mix_pick_min,
    snapshot?.interlude?.audio_preset_ids?.join('\0'),
    snapshot?.interlude?.audio_selection_mode,
    snapshot?.interlude?.audio_variation_mode,
    snapshot?.interlude?.audio_variation_period_max_ms,
    snapshot?.interlude?.audio_variation_period_min_ms,
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
    const initialMutationVersion = mediaEffectParamsMutationVersionRef.current;
    const cancelIdleWork = scheduleAfterInitialPaint(() => {
      if (cancelled) return;
      const capabilityToken = runtimeResourceActionTokenRef.current;
      if (
        !runtimeResourceBusyRef.current
        && pendingRuntimeActionRef.current === null
      ) {
        refreshRuntimeResourceCapabilities(['media'], capabilityToken);
      }
      void refreshAudioOutputDevices(() => cancelled);
      void invoke<unknown>('get_default_media_effect_params')
        .then((params) => {
          if (
            !cancelled
            && mediaEffectParamsMutationVersionRef.current === initialMutationVersion
            && isMediaEffectParams(params)
          ) {
            // 声音处理参数默认从预设抽样；采样率/码率等结构字段保留默认。
            sampleAndCommitAudioCycle({
              baseParams: params,
              randomChangePeriodMs: samplePeriodMsInRange(audioPeriodRangeRef.current),
            });
          }
        })
        .catch(() => {
          if (!cancelled) setMediaEffectParams(null);
        });
    });
    return () => {
      cancelled = true;
      cancelIdleWork();
    };
  }, [refreshRuntimeResourceCapabilities]);

  useEffect(() => {
    if (selectedFixedSpeechPresetId && !fixedSpeechPresets.some((preset) => preset.id === selectedFixedSpeechPresetId)) {
      setSelectedFixedSpeechPresetId(null);
    }
  }, [fixedSpeechPresets, selectedFixedSpeechPresetId]);

  const runtimeBaseParameters = useMemo(
    () => mediaEffectParams ? toRuntimePreviewParameters(mediaEffectParams) : null,
    [
    mediaEffectParams?.audio.input_gain_db,
    mediaEffectParams?.audio.loudness_adjustment_db,
    mediaEffectParams?.audio.pitch_shift_semitones,
    mediaEffectParams?.audio.playback_speed,
    mediaEffectParams?.audio.fade_in_ms,
    mediaEffectParams?.audio.fade_out_ms,
    mediaEffectParams?.audio.reverb_wet_percent,
    mediaEffectParams?.audio.noise_reduction_percent,
    mediaEffectParams?.audio.phase_perturbation_percent,
    mediaEffectParams?.audio.vibrato_frequency_hz,
    mediaEffectParams?.audio.vibrato_depth_percent,
    mediaEffectParams?.audio.environment_noise_percent,
    mediaEffectParams?.audio.environment_noise_dbfs,
    mediaEffectParams?.audio.filter_q,
    mediaEffectParams?.audio.sample_rate_hz,
    mediaEffectParams?.audio.output_bitrate_kbps,
    mediaEffectParams?.audio.low_eq_db,
    mediaEffectParams?.audio.mid_eq_db,
    mediaEffectParams?.audio.high_eq_db,
    mediaEffectParams?.audio.output_gain_db,
    mediaEffectParams?.video.brightness_percent,
    mediaEffectParams?.video.contrast_percent,
    mediaEffectParams?.video.saturation_percent,
    mediaEffectParams?.video.hue_rotation_degrees,
    mediaEffectParams?.video.blur_radius_px,
    mediaEffectParams?.video.pixel_scale_percent,
    mediaEffectParams?.video.space_x_offset_px,
    mediaEffectParams?.video.space_y_offset_px,
  ]);
  const [audioPeriodRange, setAudioPeriodRange] = useState<PeriodRangeMs>(() => loadAudioPeriodRange());
  const [videoPeriodRange, setVideoPeriodRange] = useState<PeriodRangeMs>(() => loadVideoPeriodRange());
  const audioPeriodRangeRef = useRef(audioPeriodRange);
  const videoPeriodRangeRef = useRef(videoPeriodRange);
  const nextAudioPeriodMsRef = useRef(samplePeriodMsInRange(audioPeriodRange));
  const nextVideoPeriodMsRef = useRef(samplePeriodMsInRange(videoPeriodRange));
  const [, setAudioPeriodMs] = useState(() => nextAudioPeriodMsRef.current);
  const [, setVideoPeriodMs] = useState(() => nextVideoPeriodMsRef.current);
  const playbackRequested = snapshot?.playback_state?.toLowerCase() === 'playing';
  const playbackActive = playbackRequested
    && mediaState?.playback_generation === snapshot?.playback_generation
    && mediaState.clock_health === 'healthy'
    && !mediaState.paused;
  const playbackClockBlocked = playbackRequested && !playbackActive;
  // 暂停、停止或真实媒体时钟不健康时冻结周期，进度不使用墙钟补偿。
  const runtimeActive = MPV_REALTIME_VIDEO_ENABLED
    && playbackActive
    && currentMediaIsVideo
    && videoProcessingEnabled;
  const videoStreamActive = MPV_REALTIME_VIDEO_ENABLED
    && playbackActive
    && currentMediaIsVideo
    && videoProcessingEnabled
    && Boolean(mediaEffectParams);
  const audioPeriodActive = playbackActive && audioProcessingEnabled && Boolean(mediaEffectParams);

  useEffect(() => {
    if (!AUTO_PORTAUDIO_ENABLED || !PORTAUDIO_FORMAL_SOURCE_SYNC_READY) {
      if (
        (audioOutputBackend?.preferred_portaudio || audioOutputBackend?.running)
        && !audioOutputBusy
      ) {
        void applyAudioOutputBackend(false);
      }
      return;
    }
    const attempt = autoPortAudioAttemptRef.current;
    const clearRetryTimer = () => {
      if (autoPortAudioRetryTimerRef.current !== null) {
        window.clearTimeout(autoPortAudioRetryTimerRef.current);
        autoPortAudioRetryTimerRef.current = null;
      }
    };
    if (!playbackRequested || !audioProcessingEnabled) {
      clearRetryTimer();
      attempt.key = null;
      attempt.startedAtMs = 0;
      if (audioOutputBackend?.running && !attempt.inFlight && !audioOutputBusy) {
        attempt.inFlight = true;
        void applyAudioOutputBackend(false).finally(() => {
          attempt.inFlight = false;
        });
      }
      return;
    }
    if (!snapshot?.source_media?.audio_sample_rate_hz || !mediaState || audioOutputBusy) return;
    if (audioOutputBackend?.running && audioOutputBackend.selected_backend === 'portaudio') {
      clearRetryTimer();
      return;
    }
    if (
      audioOutputBackend?.preferred_portaudio
      && audioOutputBackend.hardware_state === 'active'
    ) {
      clearRetryTimer();
      return;
    }
    if (audioOutputBackend && !audioOutputBackend.available) return;
    const syncClock = resolveAudioSyncClock(snapshot, mediaState, null);
    if (!syncClock || attempt.inFlight) return;
    const attemptKey = [
      syncClock.playback_generation,
      snapshot.audio_stream_revision,
      snapshot.source_media.source_path,
      audioOutputDeviceId ?? 'default',
      audioOutputMemoryKib,
    ].join(':');
    const now = Date.now();
    if (
      attempt.key === attemptKey
      && (
        !audioOutputBackend?.preferred_portaudio
        || now - attempt.startedAtMs < AUTO_PORTAUDIO_RETRY_COOLDOWN_MS
      )
    ) {
      return;
    }
    clearRetryTimer();
    attempt.key = attemptKey;
    attempt.startedAtMs = now;
    attempt.inFlight = true;
    void applyAudioOutputBackend(
      true,
      audioOutputDeviceId,
      audioOutputMemoryKib,
    )
      .then((status) => {
        if (status?.preferred_portaudio && !status.running) {
          autoPortAudioRetryTimerRef.current = window.setTimeout(() => {
            autoPortAudioRetryTimerRef.current = null;
            setAutoPortAudioRetryRevision((current) => current + 1);
          }, AUTO_PORTAUDIO_RETRY_COOLDOWN_MS);
        }
      })
      .finally(() => {
        attempt.inFlight = false;
      });
  }, [
    audioOutputBackend,
    audioOutputBusy,
    audioOutputDeviceId,
    audioOutputMemoryKib,
    audioProcessingEnabled,
    autoPortAudioRetryRevision,
    mediaState,
    playbackRequested,
    snapshot?.audio_stream_revision,
    snapshot?.playback_generation,
    snapshot?.source_media?.audio_sample_rate_hz,
    snapshot?.source_media?.source_path,
  ]);

  useEffect(() => () => {
    if (autoPortAudioRetryTimerRef.current !== null) {
      window.clearTimeout(autoPortAudioRetryTimerRef.current);
    }
  }, []);

  useEffect(() => {
    audioPeriodRangeRef.current = audioPeriodRange;
    saveAudioPeriodRange(audioPeriodRange);
    refreshAudioFutureMediaCyclePlansAfterSettingsChange();
  }, [audioPeriodRange]);
  useEffect(() => {
    videoPeriodRangeRef.current = videoPeriodRange;
    saveVideoPeriodRange(videoPeriodRange);
    clearVideoFutureMediaCyclePlans();
  }, [videoPeriodRange]);
  useEffect(() => {
    const current = activeVideoRenderRef.current;
    const source = snapshot?.source_media;
    if (
      source?.media_kind !== 'video'
      || (current !== null
        && (!snapshot || !isVideoCandidateBoundToCurrentSource(current, snapshot)))
    ) {
      clearVideoFutureMediaCyclePlans();
      pendingMediaApplyRef.current = null;
    }
  }, [
    snapshot?.playback_generation,
    snapshot?.source_media?.media_kind,
    snapshot?.source_media?.source_path,
  ]);
  useEffect(() => {
    if (!playbackActive) cancelVideoPrepareRetry();
  }, [playbackActive]);
  useEffect(() => () => {
    if (videoPrepareRetryTimerRef.current !== null) {
      window.clearTimeout(videoPrepareRetryTimerRef.current);
    }
  }, []);
  useEffect(() => {
    if (!runtimeActive || !runtimeBaseParameters) {
      if (!videoProcessingEnabled) {
        runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
        setRuntimeCycle(0);
        setRuntimeLastChangeMs(null);
      }
      return;
    }
  }, [runtimeActive, runtimeBaseParameters, videoProcessingEnabled]);

  useEffect(() => {
    if (!audioPeriodActive) {
      if (playbackClockBlocked) return;
      clearAudioFutureMediaCyclePlans();
      if (!audioProcessingEnabled || !snapshot?.source_media) {
        audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
        audioCycleSampleRef.current = null;
        setAudioActivePresetIds([]);
        setAudioVariationCycle(0);
        setAudioLastChangeMs(null);
      }
      return;
    }

  }, [audioPeriodActive, audioProcessingEnabled, playbackClockBlocked, snapshot?.source_media]);

  // 定时器只负责唤醒；prepare/commit/apply 全部以最终播放窗的绝对媒体时间为准。
  useEffect(() => {
    if (!audioPeriodActive && !videoStreamActive) {
      if (playbackClockBlocked) return;
      clearFutureMediaCyclePlans();
      return;
    }
    let cancelled = false;
    const timer = window.setInterval(() => {
      if (cancelled) return;
      const clock = mediaStateRef.current;
      if (!clock || clock.paused || clock.playback_generation !== snapshotRefHome.current?.playback_generation) return;
      if (!initializeFutureMediaCyclePlans(clock, audioPeriodActive, videoStreamActive)) return;

      void prepareNextAudioMediaCandidate();
      prepareNextVideoMediaCandidate();
    }, 100);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [
    ambientSoundPath,
    audioPeriodActive,
    playbackClockBlocked,
    videoProcessingEnabled,
    videoStreamActive,
  ]);

  function publishRuntimeParameterMessage(params: MediaEffectParams | null) {
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    const livePlayback = currentSnapshot?.playback_state?.toLowerCase() === 'playing'
      && clock?.playback_generation === currentSnapshot.playback_generation
      && clock.clock_health === 'healthy'
      && !clock.paused;
    const liveAudio = Boolean(livePlayback && audioProcessingEnabledRef.current);
    // 最终效果窗口不叠加 CSS 视频效果；画面参数统一由 FFmpeg 候选处理。
    const liveVideo = false;
    const payload = params && (liveAudio || liveVideo)
      ? toRuntimePreviewParameters(params)
      : null;
    const message: RuntimeParameterMessage = {
      version: 1,
      type: 'runtime-parameters',
      payload,
      audio_processing_enabled: liveAudio,
      video_processing_enabled: liveVideo,
      playback_generation: currentSnapshot?.playback_generation ?? null,
    };
    runtimeMessageRef.current = message;
    playbackChannelRef.current?.postMessage(message);
  }

  // 仅播放中才推送实时参数；暂停时冻结播放窗效果。
  useEffect(() => {
    publishRuntimeParameterMessage(mediaEffectParams);
  }, [
    audioProcessingEnabled,
    playbackActive,
    runtimeActive,
    runtimeBaseParameters,
    snapshot?.playback_generation,
    snapshot?.playback_state,
    videoProcessingEnabled,
  ]);

  useEffect(() => {
    if (!documentVisible || !playbackActive) return;
    const timer = window.setInterval(() => {
      const message = runtimeMessageRef.current;
      if (message) playbackChannelRef.current?.postMessage(message);
    }, 500);
    return () => window.clearInterval(timer);
  }, [documentVisible, playbackActive]);

  function updateMediaEffectParam(section: 'audio' | 'video' | 'advanced', field: PropertyKey, value: unknown) {
    if (!mediaEffectParams) return;
    mediaEffectParamsMutationVersionRef.current += 1;
    if (section === 'audio') refreshAudioFutureMediaCyclePlansAfterSettingsChange();
    else clearVideoFutureMediaCyclePlans();
    const next = {
      ...mediaEffectParams,
      [section]: { ...mediaEffectParams[section], [field]: value },
    };
    mediaEffectParamsRef.current = next;
    setMediaEffectParams(next);
  }

  async function resetMediaEffectParams() {
    const mutationVersion = ++mediaEffectParamsMutationVersionRef.current;
    clearFutureMediaCyclePlans();
    try {
      const defaults = await invoke<unknown>('get_default_media_effect_params');
      if (!isMediaEffectParams(defaults)) throw new Error('默认媒体参数响应无效');
      if (mutationVersion !== mediaEffectParamsMutationVersionRef.current) return;
      sampleAndCommitAudioCycle({ baseParams: defaults });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '恢复媒体效果参数默认值失败');
    }
  }

  function rerollSubtleAudioParams() {
    if (!mediaEffectParams) return;
    refreshAudioFutureMediaCyclePlansAfterSettingsChange();
    sampleAndCommitAudioCycle();
  }

  const sourceMediaPool = snapshot?.source_media_pool?.length
    ? snapshot.source_media_pool
    : snapshot?.source_media
      ? [snapshot.source_media]
      : [];
  const currentSource = snapshot?.source_media ?? null;
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
  const canPause = playbackDisplayState === 'playing';
  const canResume = playbackDisplayState === 'paused';
  // ponytail: 有源即可反复点播放（再开窗+开播）
  const canStartPlayback = Boolean(currentSource);
  const canStop = ['ready', 'playing', 'paused'].includes(playbackDisplayState);
  const trimmedFixedSpeechText = fixedSpeechText.trim();
  const trimmedFixedSpeechPresetTitle = fixedSpeechPresetTitle.trim();
  const selectedFixedSpeechPreset =
    selectedFixedSpeechPresetId === null
      ? null
      : fixedSpeechPresets.find((preset) => preset.id === selectedFixedSpeechPresetId) ?? null;
  const fixedSpeechBusy = fixedSpeechState.status === 'starting' || fixedSpeechState.status === 'playing';
  const fixedSpeechPlayDisabledReason =
    !currentSource
      ? '请先导入媒体'
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
    sourceOverride?: PlaybackPoolSource | null,
  ): Promise<boolean> {
    setPlayerWindowBusy(true);
    setError(null);
    try {
      const source = sourceOverride ?? snapshot?.source_media ?? null;
      const request =
        source?.media_kind === 'video'
        && typeof source.width === 'number'
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
        releasePictureInPictureVideo(video);
        return;
      }
      if (
        !currentMediaIsVideo
        || snapshotRefHome.current?.source_media?.media_kind !== 'video'
        || !pictureInPictureSourceUrl
        || pipDocument.pictureInPictureEnabled !== true
        || typeof video.requestPictureInPicture !== 'function'
      ) {
        setError('当前桌面运行时不支持画中画。');
        return;
      }
      const operation = pictureInPictureOperationRef.current + 1;
      pictureInPictureOperationRef.current = operation;
      pictureInPictureLoadedSourceRef.current = pictureInPictureSourceUrl;
      video.pause();
      video.preload = 'metadata';
      video.src = pictureInPictureSourceUrl;
      video.load();
      await waitForPictureInPictureMetadata(video, operation);
      syncPictureInPictureVideo();
      if (!mediaState?.paused) await video.play().catch(() => undefined);
      await video.requestPictureInPicture();
      if (pictureInPictureOperationRef.current === operation) setPictureInPictureActive(true);
    } catch {
      releasePictureInPictureVideo(video);
      setError('画中画操作失败，视频播放不受影响。');
    }
  }

  async function runPlaybackAction(
    action: 'pause' | 'resume' | 'stop',
    command: 'pause_playback' | 'resume_playback' | 'stop_playback' | 'start_playback',
  ) {
    if (!shouldIssuePlaybackCommand(command, snapshotRefHome.current?.playback_state)) return;
    if (action === 'stop') {
      clearFutureMediaCyclePlans();
      pendingMediaApplyRef.current = null;
    }
    const requestId = ++playbackActionRequestRef.current;
    setPlaybackActionBusy(action);
    setError(null);
    try {
      const nextSnapshot = await invokePlaybackSnapshot(command);
      if (requestId !== playbackActionRequestRef.current) return;
      setSnapshot(nextSnapshot);
      playbackChannelRef.current?.postMessage({ version: 1, type: 'playback-control', action } satisfies PlaybackControlMessage);
    } catch (cause) {
      if (requestId !== playbackActionRequestRef.current) return;
      setError(getDisplayErrorMessage(cause, '更新播放状态失败'));
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
    const requestId = ++processingSwitchRequestRef.current;
    if (next.audio_processing_enabled !== audioProcessingEnabledRef.current) {
      discardAudioFutureMediaCyclePlans();
    }
    if (next.video_processing_enabled !== videoProcessingEnabledRef.current) {
      clearVideoFutureMediaCyclePlans();
    }
    mediaEffectParamsMutationVersionRef.current += 1;
    pendingMediaApplyRef.current = null;
    const videoJustEnabled = next.video_processing_enabled
      && !videoProcessingEnabledRef.current
      && snapshotRefHome.current?.source_media?.media_kind === 'video';
    const audioJustEnabled = next.audio_processing_enabled && !audioProcessingEnabledRef.current;
    audioProcessingEnabledRef.current = next.audio_processing_enabled;
    videoProcessingEnabledRef.current = next.video_processing_enabled;
    setVideoProcessingEnabled(next.video_processing_enabled);
    setAudioProcessingEnabled(next.audio_processing_enabled);
    if (audioJustEnabled) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setAudioVariationCycle(0);
      setAudioLastChangeMs(null);
    }
    if (videoJustEnabled) {
      runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setRuntimeCycle(0);
      setRuntimeLastChangeMs(null);
    }
    if (!next.audio_processing_enabled) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setAudioVariationCycle(0);
      setAudioLastChangeMs(null);
    }
    if (!next.video_processing_enabled) {
      activeVideoRenderRef.current = null;
      void invoke<unknown>('stop_realtime_video_renderer')
        .then((status) => setMediaVideoBackendStatus(parseMediaVideoBackendStatus(status)))
        .catch(() => setMediaVideoBackendStatus(null));
      runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setRuntimeCycle(0);
      setRuntimeLastChangeMs(null);
    }
    const request = processingSwitchQueueRef.current.then(() => invokePlaybackSnapshot(
      'set_processing_switches',
      { request: next },
    ));
    processingSwitchQueueRef.current = request.then(() => undefined, () => undefined);
    try {
      const nextSnapshot = await request;
      if (requestId !== processingSwitchRequestRef.current) return;
      snapshotRefHome.current = nextSnapshot;
      setSnapshot(nextSnapshot);
      if (
        audioJustEnabled
        || videoJustEnabled
        || nextSnapshot.source_media?.media_kind === 'video'
      ) {
        queueMicrotask(() => wakeMediaCycleScheduling('switch'));
      }
    } catch (cause) {
      if (requestId !== processingSwitchRequestRef.current) return;
      setError(cause instanceof Error ? cause.message : '更新处理开关失败');
    }
  }

  async function resetVideoEffectParams() {
    if (!mediaEffectParams) return;
    const mutationVersion = ++mediaEffectParamsMutationVersionRef.current;
    clearVideoFutureMediaCyclePlans();
    try {
      const defaults = await invoke<unknown>('get_default_media_effect_params');
      if (!isMediaEffectParams(defaults)) throw new Error('默认媒体参数响应无效');
      if (mutationVersion !== mediaEffectParamsMutationVersionRef.current) return;
      const next = {
        ...mediaEffectParams,
        video: defaults.video,
        advanced: defaults.advanced,
      };
      mediaEffectParamsRef.current = next;
      setMediaEffectParams(next);
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '恢复视频参数默认值失败'));
    }
  }

  function retryAudioProcessing() {
    if (!audioPeriodActive) return;
    discardAudioFutureMediaCyclePlans();
    wakeMediaCycleScheduling('retry');
  }

  function retryVideoProcessing() {
    if (!videoStreamActive) return;
    clearVideoFutureMediaCyclePlans();
    wakeMediaCycleScheduling('retry');
  }

  async function applyVideoProcessing(
    paramsOverride?: MediaEffectParams,
    mediaCandidateOverride?: PreparedVideoMediaCandidate | null,
  ) {
    function releaseUnstartedCandidate(candidate: PreparedVideoMediaCandidate | null | undefined) {
      if (!candidate) return;
      if (pendingMediaApplyRef.current?.mediaCandidate === candidate) {
        pendingMediaApplyRef.current = null;
      }
      if (activeVideoRenderRef.current === candidate) activeVideoRenderRef.current = null;
      logVideoPeriodStage(candidate, 'prepare', 'not_started');
    }

    const currentSnapshot = snapshotRefHome.current ?? snapshot;
    if (!currentSnapshot) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      return;
    }
    const sourceMediaIndex = mediaCandidateOverride?.sourceMediaIndex
      ?? currentSnapshot.source_media_index;
    const source = sourceMediaIndex === null || sourceMediaIndex === undefined
      ? null
      : currentSnapshot.source_media_pool[sourceMediaIndex] ?? null;
    if (
      !source
      || source.media_kind !== 'video'
      || (mediaCandidateOverride && mediaCandidateOverride.sourcePath !== source.source_path)
    ) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      return;
    }
    const rawParams = paramsOverride ?? mediaEffectParams;
    const params = rawParams
      ? {
          ...rawParams,
          // 保留完整正式参数；Rust 媒体边界会再次校验并拒绝尚无真实执行条件的字段。
          audio: { ...rawParams.audio },
          video: { ...rawParams.video },
          advanced: { ...rawParams.advanced },
        }
      : null;
    if (!params) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      return;
    }
    if (mediaApplyInFlightRef.current) {
      if (mediaCandidateOverride) {
        const previous = pendingMediaApplyRef.current?.mediaCandidate;
        pendingMediaApplyRef.current = { params, mediaCandidate: mediaCandidateOverride };
        if (
          !previous
          || videoRenderIdentity(
            previous.timeline.planId,
            previous.timeline.sequence,
            previous.playbackGeneration,
          ) !== videoRenderIdentity(
            mediaCandidateOverride.timeline.planId,
            mediaCandidateOverride.timeline.sequence,
            mediaCandidateOverride.playbackGeneration,
          )
        ) {
          logVideoPeriodStage(mediaCandidateOverride, 'prepare', 'queued');
        }
      }
      return;
    }
    // 资源等待态已经持有唯一恢复回调；禁止周期调度用新 token 覆盖它。
    if (pendingRuntimeActionRef.current?.component === 'media') return;
    mediaApplyInFlightRef.current = true;
    const releasePreAcceptSlot = () => {
      mediaApplyInFlightRef.current = false;
      setMediaProcessingBusy(null);
      queueMicrotask(() => void flushPendingMediaApply());
    };
    let validation: MediaParameterValidationResult;
    try {
      const validationResponse = await invoke<unknown>('validate_media_effect_params', {
        request: params,
      });
      if (!isMediaParameterValidationResult(validationResponse)) {
        throw new Error('媒体效果参数校验响应无效');
      }
      validation = validationResponse;
    } catch (cause) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      releasePreAcceptSlot();
      if (!isMediaProcessingPlaybackActive(currentSnapshot.playback_generation)) return;
      setError(getDisplayErrorMessage(cause, '媒体效果参数校验失败'));
      return;
    }
    if (!isMediaProcessingPlaybackActive(currentSnapshot.playback_generation)) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      releasePreAcceptSlot();
      return;
    }
    if (!validation.valid) {
      releaseUnstartedCandidate(mediaCandidateOverride);
      releasePreAcceptSlot();
      setError(validation.errors[0]?.message ?? '媒体效果参数校验失败');
      return;
    }
    let mediaCandidate = mediaCandidateOverride ?? null;
    if (!mediaCandidate) {
      const clock = mediaStateRef.current;
      const sourceDurationMs = source.duration_ms;
      if (
        !clock
        || clock.playback_generation !== currentSnapshot.playback_generation
        || typeof sourceDurationMs !== 'number'
        || !Number.isSafeInteger(sourceDurationMs)
        || sourceDurationMs <= 0
      ) {
        releasePreAcceptSlot();
        return;
      }
      mediaCandidateSequenceRef.current += 1;
      const targetAbsolutePositionMs = alignMediaPositionToVideoFrame(
        clock.absolute_position_ms + 500,
        source.frame_rate_fps,
      );
      const requestedValidUntilAbsolutePositionMs = targetAbsolutePositionMs + Math.max(
        nextVideoPeriodMsRef.current,
        1_000,
      );
      const sourceWindowEndMs = targetAbsolutePositionMs
        - (targetAbsolutePositionMs % sourceDurationMs)
        + sourceDurationMs;
      const validUntilAbsolutePositionMs = currentSnapshot.source_media_pool.length === 1
        ? requestedValidUntilAbsolutePositionMs
        : Math.min(requestedValidUntilAbsolutePositionMs, sourceWindowEndMs);
      if (validUntilAbsolutePositionMs <= targetAbsolutePositionMs) {
        releasePreAcceptSlot();
        return;
      }
      mediaCandidate = {
        params,
        videoCyclePlan: null,
        videoEffectsEnabled: videoProcessingEnabledRef.current,
        realtimePrepared: false,
        timeline: createMediaArtifactTimeline({
            planId: `video-manual-${mediaCandidateSequenceRef.current}`,
            sequence: mediaCandidateSequenceRef.current,
            playbackGeneration: currentSnapshot.playback_generation,
            sourceRevision: clock.source_revision,
            targetAbsolutePositionMs,
            validUntilAbsolutePositionMs,
            sourceDurationMs,
            safetyTailMs: 0,
          }),
        sourcePath: source.source_path,
        sourceMediaIndex: currentSnapshot.source_media_index ?? 0,
        playbackGeneration: currentSnapshot.playback_generation,
      };
    }
    if (
      videoPrepareRetryCandidateRef.current
      && videoPrepareRetryCandidateRef.current !== mediaCandidate
    ) {
      cancelVideoPrepareRetry();
    }
    setError(null);
    setMediaProcessingBusy('video');
    let retryScheduled = false;
    try {
      await ensureRuntimeResources('media', async () => {
        if (!isVideoCandidatePlaybackActive(mediaCandidate)) {
          releaseUnstartedCandidate(mediaCandidate);
          return;
        }
        setMediaProcessingBusy('video');
        try {
          const clock = mediaStateRef.current;
          if (!clock || !isVideoCandidatePlaybackActive(mediaCandidate)) {
            releaseUnstartedCandidate(mediaCandidate);
            return;
          }
          if (
            activeVideoRenderRef.current
            && activeVideoRenderRef.current !== mediaCandidate
          ) {
            releaseUnstartedCandidate(mediaCandidate);
            return;
          }
          activeVideoRenderRef.current = mediaCandidate;
          if (!MPV_REALTIME_VIDEO_ENABLED || !mediaCandidate.videoEffectsEnabled) {
            releaseUnstartedCandidate(mediaCandidate);
            return;
          }
          const queue = videoFuturePlansRef.current;
          const nextPlan = queue?.[0].sequence === mediaCandidate.timeline.sequence
            ? queue[1]
            : null;
          try {
            const response = await invoke<unknown>('prepare_realtime_video_plan', {
              request: {
                params,
                sequence: mediaCandidate.timeline.sequence,
                playback_generation: mediaCandidate.timeline.playbackGeneration,
                source_revision: VIDEO_BACKEND_SOURCE_REVISION,
                target_absolute_position_ms: mediaCandidate.timeline.targetAbsolutePositionMs,
                period_ms: mediaCandidate.videoCyclePlan?.periodMediaMs
                  ?? mediaCandidate.timeline.outputDurationMs,
                seed: mediaCandidate.videoCyclePlan?.payload.seed ?? 0,
                next_sequence: nextPlan?.sequence ?? null,
                next_target_absolute_position_ms: nextPlan?.targetAbsolutePositionMs ?? null,
              },
            });
            const status = parseMediaVideoBackendStatus(response);
            setMediaVideoBackendStatus(status);
            if (canUseRealtimeVideoBackend(status)) {
              mediaCandidate.realtimePrepared = true;
              resetVideoPrepareRetry(mediaCandidate);
              videoCycleRetryRef.current = clearCycleRetry();
              logVideoPeriodStage(mediaCandidate, 'prepare', 'accepted');
              return;
            }
          } catch (cause) {
            setError(getDisplayErrorMessage(cause, '实时画面准备失败，保持 Original 并稍后重试'));
            await invoke('stop_realtime_video_renderer').catch(() => undefined);
            scheduleVideoPrepareRetry(params, mediaCandidate);
            retryScheduled = true;
            return;
          }
          await invoke('stop_realtime_video_renderer').catch(() => undefined);
          releaseUnstartedCandidate(mediaCandidate);
          if (mediaCandidate.videoCyclePlan) scheduleVideoCycleRetry();
        } finally {
          setMediaProcessingBusy(null);
        }
      }, (status) => {
        releaseUnstartedCandidate(mediaCandidate);
        setError(status.error || '实时画面资源准备已取消，当前保持 Original。');
      });
    } catch (cause) {
      resetVideoPrepareRetry(mediaCandidate);
      releaseUnstartedCandidate(mediaCandidate);
      if (!isVideoCandidatePlaybackActive(mediaCandidate)) return;
      setError(getDisplayErrorMessage(cause, '启动本地媒体处理失败'));
    } finally {
      mediaApplyInFlightRef.current = false;
      setMediaProcessingBusy(null);
      if (!retryScheduled) void flushPendingMediaApply();
    }
  }

  function commitPreparedRealtimeVideoCandidate(clock: PlaybackMediaStateMessage) {
    const candidate = activeVideoRenderRef.current;
    if (
      !candidate
      || !candidate.realtimePrepared
      || realtimeVideoCommitInFlightRef.current
      || clock.playback_generation !== candidate.playbackGeneration
      || clock.absolute_position_ms < candidate.timeline.targetAbsolutePositionMs
    ) return;
    realtimeVideoCommitInFlightRef.current = true;
    void invoke<unknown>('commit_realtime_video_plan', {
      request: {
        sequence: candidate.timeline.sequence,
        playback_generation: candidate.timeline.playbackGeneration,
        source_revision: VIDEO_BACKEND_SOURCE_REVISION,
        media_pts_ms: clock.absolute_position_ms,
      },
    })
      .then((response) => {
        const status = parseMediaVideoBackendStatus(response);
        setMediaVideoBackendStatus(status);
        if (status?.backend !== 'realtime_gpu' || status.activation !== 'active') {
          candidate.realtimePrepared = false;
          void invoke('stop_realtime_video_renderer').catch(() => undefined);
          scheduleVideoPrepareRetry(candidate.params, candidate);
          return;
        }
        if (candidate.videoCyclePlan) applyPlannedVideoCycle(candidate.videoCyclePlan);
        else {
          mediaEffectParamsRef.current = candidate.params;
          setMediaEffectParams(candidate.params);
        }
        activeVideoRenderRef.current = null;
        if (candidate.videoCyclePlan) {
          advanceIndependentVideoQueue(clock.absolute_position_ms);
        }
      })
      .catch((cause) => {
        candidate.realtimePrepared = false;
        setError(getDisplayErrorMessage(cause, '实时画面周期提交失败，保持 Original 并稍后重试'));
        void invoke('stop_realtime_video_renderer').catch(() => undefined);
        scheduleVideoPrepareRetry(candidate.params, candidate);
      })
      .finally(() => {
        realtimeVideoCommitInFlightRef.current = false;
        void flushPendingMediaApply();
      });
  }

  useEffect(() => {
    if (mediaState) commitPreparedRealtimeVideoCandidate(mediaState);
  }, [
    mediaState?.absolute_position_ms,
    mediaState?.playback_generation,
    mediaVideoBackendStatus?.activation,
    mediaVideoBackendStatus?.backend,
    ],
  );

  useEffect(() => {
    if (
      !mediaState
      || mediaVideoBackendStatus?.backend !== 'realtime_gpu'
      || !['available', 'active'].includes(mediaVideoBackendStatus.activation)
    ) return;
    void invoke('sync_realtime_video_renderer', {
      request: {
        playback_generation: mediaState.playback_generation,
        position_ms: mediaState.position_ms,
        paused: playbackDisplayState !== 'playing',
      },
    }).catch(() => {
      // 连续播放期间不轮询 IPC；仅在代次或播放/暂停状态变化时同步，失败由下轮提交降级。
    });
  }, [
    mediaState?.playback_generation,
    playbackDisplayState,
    mediaVideoBackendStatus?.activation,
    mediaVideoBackendStatus?.backend,
  ]);

  function commitCompletedAudioRender(currentSnapshot: PlaybackSnapshot) {
    const completed = activeAudioRenderRef.current;
    if (!completed) return;
    if (currentSnapshot.audio_processing_status === 'failed') {
      activeAudioRenderRef.current = null;
      if (
        currentSnapshot.fallback_reason?.includes('候选声音已超过有效媒体时间窗口')
        && audioFuturePlansRef.current
      ) {
        advanceIndependentAudioQueue(
          mediaStateRef.current?.absolute_position_ms ?? completed.timeline.validUntilAbsolutePositionMs,
        );
        return;
      }
      if (!isMediaProcessingPlaybackActive(completed.playbackGeneration)) return;
      scheduleAudioCycleRetry();
      return;
    }
    if (currentSnapshot.pending_audio_artifact_reference !== null) {
      if (
        currentSnapshot.pending_audio_media_plan_id !== completed.timeline.planId
        || currentSnapshot.pending_audio_media_sequence !== completed.timeline.sequence
        || currentSnapshot.pending_audio_media_playback_generation !== completed.timeline.playbackGeneration
        || currentSnapshot.pending_audio_media_source_revision !== completed.timeline.sourceRevision
      ) return;
      completed.artifactReference = currentSnapshot.pending_audio_artifact_reference;
      audioCycleRetryRef.current = clearAudioCycleRetry();
      return;
    }
    const currentTimeline = currentAudioArtifactTimeline(currentSnapshot);
    if (
      !currentTimeline
      || currentTimeline.planId !== completed.timeline.planId
      || currentTimeline.sequence !== completed.timeline.sequence
      || currentTimeline.playbackGeneration !== completed.timeline.playbackGeneration
      || currentTimeline.sourceRevision !== completed.timeline.sourceRevision
    ) return;
    completed.artifactReference ??= currentSnapshot.current_audio_artifact_reference;
    if (
      completed.artifactReference === null
      || currentSnapshot.current_audio_artifact_reference !== completed.artifactReference
    ) return;
    if (
      currentSnapshot.source_media?.source_path !== completed.sourcePath
      || currentSnapshot.playback_generation !== completed.playbackGeneration
    ) {
      activeAudioRenderRef.current = null;
      return;
    }
    applyPlannedAudioCycle(completed.audioCyclePlan);
    audioCycleRetryRef.current = clearAudioCycleRetry();
    activeAudioRenderRef.current = null;
    advanceIndependentAudioQueue(
      mediaStateRef.current?.absolute_position_ms ?? completed.timeline.targetAbsolutePositionMs,
    );
  }

  useEffect(() => {
    if (snapshot) commitCompletedAudioRender(snapshot);
  }, [
    snapshot?.playback_generation,
    snapshot?.source_media?.source_path,
    snapshot?.current_audio_artifact_reference,
    snapshot?.pending_audio_artifact_reference,
    snapshot?.pending_audio_media_plan_id,
    snapshot?.pending_audio_media_sequence,
    snapshot?.pending_audio_media_playback_generation,
    snapshot?.pending_audio_media_source_revision,
    snapshot?.current_audio_media_plan_id,
    snapshot?.current_audio_media_sequence,
    snapshot?.current_audio_media_playback_generation,
    snapshot?.current_audio_media_source_revision,
    snapshot?.audio_processing_status,
  ]);

  async function flushPendingMediaApply() {
    const pending = pendingMediaApplyRef.current;
    const currentSnapshot = snapshotRefHome.current;
    const clock = mediaStateRef.current;
    if (!pending || !currentSnapshot?.source_media || !clock) return;
    if (
      currentSnapshot.playback_state?.toLowerCase() !== 'playing'
      || clock.playback_generation !== currentSnapshot.playback_generation
      || clock.paused
      || clock.clock_health !== 'healthy'
    ) return;
    await flushLatestPendingApply(
      pendingMediaApplyRef,
      () => (
        mediaApplyInFlightRef.current
        || pendingRuntimeActionRef.current?.component === 'media'
        || (activeVideoRenderRef.current !== null
          && pending.mediaCandidate !== activeVideoRenderRef.current)
      ),
      (latest) => applyVideoProcessing(latest.params, latest.mediaCandidate),
    );
  }

  // 声音 Worker 结束后稍等再续跑排队任务，避免连续重渲打断听感。
  useEffect(() => {
    const processing = snapshot?.audio_processing_status === 'processing';
    if (mediaWasProcessingRef.current && !processing) {
      const timer = window.setTimeout(() => {
        void flushPendingMediaApply();
      }, 2_500);
      mediaWasProcessingRef.current = false;
      return () => window.clearTimeout(timer);
    }
    mediaWasProcessingRef.current = Boolean(processing);
    if (playbackActive) void flushPendingMediaApply();
  }, [playbackActive, snapshot?.audio_processing_status]);

  async function cleanupLocalCaches() {
    if (cacheCleanupBusy) return;
    setCacheCleanupError(null);
    setCacheCleanup(null);
    setError(null);
    setCacheCleanupBusy(true);
    try {
      const result = await invoke<unknown>('cleanup_local_caches_command');
      if (!isCacheCleanupResult(result)) throw new Error('缓存清理响应无效');
      setCacheCleanup(result);
    } catch (cause) {
      const message = getDisplayErrorMessage(cause, '删除已生成缓存失败');
      setCacheCleanupError(message);
      setError(message);
    } finally {
      setCacheCleanupBusy(false);
    }
  }

  function applyPlaybackPoolSnapshot(nextSnapshot: PlaybackSnapshot) {
    snapshotRequestRef.current += 1;
    snapshotRefHome.current = nextSnapshot;
    setSnapshot(nextSnapshot);
    setSnapshotLoading(false);
    setSnapshotFetchError(null);
  }

  async function resumePlaybackPoolMutation(
    command: string,
    args: Record<string, unknown> | undefined,
    fallbackMessage: string,
  ) {
    importVideoInFlightRef.current = true;
    setImportVideoBusy(true);
    try {
      applyPlaybackPoolSnapshot(await invokePlaybackSnapshot(command, args));
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, fallbackMessage));
    } finally {
      importVideoInFlightRef.current = false;
      setImportVideoBusy(false);
    }
  }

  async function runPlaybackPoolMutation(
    command: string,
    args: Record<string, unknown> | undefined,
    fallbackMessage: string,
  ) {
    if (importVideoInFlightRef.current) return;
    setFixedSpeechFormError(null);
    setError(null);
    await resumePlaybackPoolMutation(command, args, fallbackMessage);
  }

  async function importVideo() {
    if (importVideoInFlightRef.current) return;
    importVideoInFlightRef.current = true;
    setImportVideoBusy(true);
    try {
      const selection = await open({
        multiple: true,
        filters: [{ name: '媒体文件', extensions: [...SUPPORTED_MEDIA_EXTENSIONS] }],
      });
      const paths = Array.isArray(selection)
        ? selection
        : typeof selection === 'string'
          ? [selection]
          : [];
      if (paths.length === 0) return;
      setFixedSpeechFormError(null);
      setError(null);
      await ensureRuntimeResources('media', async () => {
        await resumePlaybackPoolMutation('probe_local_videos', { request: { paths } }, '导入媒体失败');
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '导入媒体失败'));
    } finally {
      importVideoInFlightRef.current = false;
      setImportVideoBusy(false);
    }
  }

  async function appendVideosToPlaybackPool(droppedPaths?: string[]) {
    if (importVideoInFlightRef.current) return;
    importVideoInFlightRef.current = true;
    setImportVideoBusy(true);
    try {
      const selection = droppedPaths ?? await open({
        multiple: true,
        filters: [{ name: '媒体文件', extensions: [...SUPPORTED_MEDIA_EXTENSIONS] }],
      });
      const paths = Array.isArray(selection)
        ? selection
        : typeof selection === 'string'
          ? [selection]
          : [];
      if (paths.length === 0) return;
      if (sourceMediaPool.length + paths.length > PLAYBACK_POOL_LIMIT) {
        setError(`播放池最多包含 ${PLAYBACK_POOL_LIMIT} 项；当前 ${sourceMediaPool.length} 项，本次选择 ${paths.length} 项。`);
        return;
      }
      setFixedSpeechFormError(null);
      setError(null);
      await ensureRuntimeResources('media', async () => {
        await resumePlaybackPoolMutation('append_local_videos', { request: { paths } }, '追加媒体失败');
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '追加媒体失败'));
    } finally {
      importVideoInFlightRef.current = false;
      setImportVideoBusy(false);
    }
  }

  async function replacePlaybackPoolItem(sourcePath: string) {
    if (importVideoInFlightRef.current) return;
    importVideoInFlightRef.current = true;
    setImportVideoBusy(true);
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: '媒体文件', extensions: [...SUPPORTED_MEDIA_EXTENSIONS] }],
      });
      if (typeof selected !== 'string') return;
      setFixedSpeechFormError(null);
      setError(null);
      await ensureRuntimeResources('media', async () => {
        await resumePlaybackPoolMutation(
          'replace_playback_pool_item',
          { request: { source_path: sourcePath, path: selected } },
          '替换媒体失败',
        );
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '替换媒体失败'));
    } finally {
      importVideoInFlightRef.current = false;
      setImportVideoBusy(false);
    }
  }

  async function reorderPlaybackPool(sourcePaths: string[]) {
    await runPlaybackPoolMutation(
      'reorder_playback_pool_items',
      { request: { source_paths: sourcePaths } },
      '调整播放顺序失败',
    );
  }

  async function removePlaybackPoolItem(sourcePath: string) {
    await runPlaybackPoolMutation(
      'remove_playback_pool_item',
      { request: { source_path: sourcePath } },
      '删除媒体失败',
    );
  }

  async function clearPlaybackPool() {
    await runPlaybackPoolMutation('clear_playback_pool', undefined, '清空播放池失败');
  }

  function updateInterludeDraft(patch: Partial<InterludeConfigDraft>) {
    setInterludeDirty(true);
    setInterludeSaveError(null);
    setInterludeDraft((current) => ({ ...current, ...patch }));
  }

  function postInterludeVolumeControl(volumeDb: number) {
    playbackChannelRef.current?.postMessage({
      version: 1,
      type: 'interlude-volume-control',
      volume_db: volumeDb,
    } satisfies InterludeVolumeControlMessage);
  }

  async function persistInterludeVolume(volumeDb: number) {
    const interlude = snapshot?.interlude;
    if (!interlude) return;
    try {
      const request: PersistedInterludeConfig = {
        enabled: interlude.enabled,
        directory: interlude.directory,
        audio_selection_mode: interlude.audio_selection_mode ?? 'random',
        audio_fixed_preset_id: interlude.audio_fixed_preset_id ?? 'p01',
        audio_preset_ids: normalizeInterludeAudioPresetIds(interlude.audio_preset_ids),
        audio_mix_enabled: interlude.audio_mix_enabled ?? false,
        audio_mix_pick_min: interlude.audio_mix_pick_min ?? DEFAULT_AUDIO_MIX_PICK_MIN,
        audio_mix_pick_max: interlude.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
        audio_variation_mode: 'periodic',
        audio_variation_period_min_ms: interlude.audio_variation_period_min_ms ?? 8_000,
        audio_variation_period_max_ms: interlude.audio_variation_period_max_ms ?? 15_000,
        interval_min_ms: interlude.interval_min_ms,
        interval_max_ms: interlude.interval_max_ms,
        volume_db: volumeDb,
        ducking_depth_db: interlude.ducking_depth_db,
        ducking_attack_ms: interlude.ducking_attack_ms,
        ducking_release_ms: interlude.ducking_release_ms,
      };
      const nextSnapshot = await invokePlaybackSnapshot('set_interlude_config', {
        request,
      });
      saveInterludeConfigToStorage(request);
      savedInterludeConfigRef.current = request;
      setSnapshot(nextSnapshot);
      setInterludeDraft((current) => ({
        ...current,
        volumeDb: nextSnapshot.interlude?.volume_db ?? volumeDb,
      }));
    } catch (cause) {
      const message = getDisplayErrorMessage(cause, '保存插话音量失败');
      setInterludeSaveError(message);
      setError(message);
    }
  }

  async function chooseAmbientSoundFile() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: '环境声音频', extensions: ['mp3', 'wav', 'm4a', 'aac', 'ogg', 'flac'] }],
      });
      if (typeof selected === 'string') {
        setAmbientSoundPath(selected);
        setError(null);
      }
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '选择环境声素材失败'));
    }
  }

  async function chooseInterludeDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== 'string') return;
    updateInterludeDraft({ directory: selected });
  }

  async function saveInterludeConfig() {
    const validationErrors = getInterludeValidationErrors(interludeDraft);
    if (validationErrors.length > 0) {
      setInterludeSaveError(validationErrors.join('；'));
      return;
    }
    setInterludeSaving(true);
    setInterludeSaveError(null);
    setError(null);
    try {
      const audioFixedPresetId = normalizeInterludeAudioPresetIds([
        interludeDraft.audioFixedPresetId,
      ])[0];
      const audioPresetIds = normalizeInterludeAudioPresetIds(interludeDraft.audioPresetIds);
      const audioMixPickMax = normalizeAudioMixPickMax(interludeDraft.audioMixPickMax);
      const audioMixPickMin = normalizeAudioMixPickMin(
        interludeDraft.audioMixPickMin,
        audioMixPickMax,
      );
      const audioVariationPeriod = normalizeInterludePresetPeriodRange(
        interludeDraft.audioVariationPeriodMinMs,
        interludeDraft.audioVariationPeriodMaxMs,
      );
      const request: PersistedInterludeConfig = {
        enabled: interludeDraft.enabled,
        directory: interludeDraft.directory,
        audio_selection_mode: interludeDraft.audioSelectionMode,
        audio_fixed_preset_id: audioFixedPresetId,
        audio_preset_ids: audioPresetIds,
        audio_mix_enabled: interludeDraft.audioMixEnabled,
        audio_mix_pick_min: audioMixPickMin,
        audio_mix_pick_max: audioMixPickMax,
        audio_variation_mode: 'periodic',
        audio_variation_period_min_ms: audioVariationPeriod.minMs,
        audio_variation_period_max_ms: audioVariationPeriod.maxMs,
        interval_min_ms: interludeDraft.intervalMinMs,
        interval_max_ms: interludeDraft.intervalMaxMs,
        volume_db: interludeDraft.volumeDb,
        ducking_depth_db: interludeDraft.duckingDepthDb,
        ducking_attack_ms: interludeDraft.duckingAttackMs,
        ducking_release_ms: interludeDraft.duckingReleaseMs,
      };
      const nextSnapshot = await invokePlaybackSnapshot('set_interlude_config', {
        request,
      });
      saveInterludeConfigToStorage(request);
      savedInterludeConfigRef.current = request;
      setSnapshot(nextSnapshot);
      setInterludeDraft(buildInterludeDraft(nextSnapshot.interlude));
      setInterludeDirty(false);
      void messageApi.success('保存成功');
    } catch (cause) {
      const message = getDisplayErrorMessage(cause, '保存插话配置失败');
      setInterludeSaveError(message);
      setError(message);
    } finally {
      setInterludeSaving(false);
    }
  }

  const nextVideoPlan = videoFuturePlansRef.current?.[0];
  const nextAudioPlan = audioFuturePlansRef.current?.[0];
  const mediaCycleClockIdentity = mediaState && snapshot
    ? `${mediaState.playback_generation}:${mediaState.source_revision}:${mediaState.clock_epoch}:${snapshot.loop_index}`
    : null;
  const mediaCycleAbsolutePositionMs = mediaState
    && snapshot
    && mediaState.playback_generation === snapshot.playback_generation
    && mediaCycleClockIdentityRef.current === mediaCycleClockIdentity
    && ['playing', 'paused'].includes(snapshot.playback_state.toLowerCase())
      ? mediaState.absolute_position_ms
      : null;
  const scheduledVideoProgressPercent = currentMediaIsVideo
    ? getMediaCycleProgressPercent(nextVideoPlan, mediaCycleAbsolutePositionMs)
    : 0;
  const runtimeProgressPercent = scheduledVideoProgressPercent;
  const activeVideoCandidate = activeVideoRenderRef.current;
  const activeVideoCandidateMatchesPlan = Boolean(
    nextVideoPlan
    && snapshot
    && activeVideoCandidate?.videoCyclePlan?.planId === nextVideoPlan.planId
    && activeVideoCandidate.videoCyclePlan.sequence === nextVideoPlan.sequence
    && activeVideoCandidate.playbackGeneration === snapshot.playback_generation,
  );
  const currentVideoCandidatePreparing = activeVideoCandidateMatchesPlan
    && !activeVideoCandidate?.realtimePrepared;
  const currentVideoCandidateReady = activeVideoCandidateMatchesPlan
    && activeVideoCandidate?.realtimePrepared === true;
  const currentVideoCandidatePresented = mediaVideoBackendStatus?.backend === 'realtime_gpu'
    && mediaVideoBackendStatus.activation === 'active';
  const currentVideoCandidateFailed = mediaVideoBackendStatus?.activation === 'failed';
  const audioProgressPercent = audioProcessingEnabled && currentSource
    ? getMediaCycleProgressPercent(nextAudioPlan, mediaCycleAbsolutePositionMs)
    : 0;
  const diagnosticMessage = diagnosticSummary.message;
  const diagnosticFresh = diagnosticSummary.fresh;
  const interludePlaybackNotice = interludeDraft.enabled
    ? playbackDisplayState !== 'playing'
      ? '当前媒体已暂停或未开始，插话不会播放，请点击“继续”后再试听。'
      : !diagnosticFresh
        ? '最终效果播放器尚未连接，插话不会播放，请点击“打开/聚焦播放器”。'
        : null
    : null;
  const mediaDuration = mediaState?.duration ?? 0;
  const mediaCurrentTime = clampMediaTime(mediaState?.current_time ?? 0, mediaDuration);
  const mediaDisplayedTime = mediaSeekDraft?.value ?? mediaCurrentTime;
  useEffect(() => {
    if (
      !mediaSeekDraft?.committed
      || !mediaState
      || mediaSeekDraft.playbackGeneration !== mediaState.playback_generation
      || Math.abs(mediaState.current_time - mediaSeekDraft.value) > 0.25
    ) return;
    setMediaSeekDraft(null);
  }, [mediaSeekDraft, mediaState]);
  useEffect(() => {
    if (!mediaSeekDraft?.committed) return;
    const timeout = window.setTimeout(() => setMediaSeekDraft((current) => (
      current === mediaSeekDraft ? null : current
    )), 3_000);
    return () => window.clearTimeout(timeout);
  }, [mediaSeekDraft]);
  const playbackClockNotice = !mediaState
    || mediaState.playback_generation !== snapshot?.playback_generation
    || mediaState.clock_health === 'healthy'
    ? null
    : mediaState.clock_health === 'buffering'
      ? { type: 'info' as const, message: '播放器正在缓冲，声音和画面周期已冻结。' }
      : mediaState.clock_health === 'recovering'
        ? { type: 'warning' as const, message: '播放时钟停滞，正在自动恢复。' }
        : { type: 'error' as const, message: '播放时钟停滞且自动恢复失败，已保持播放状态并继续尝试；如画面仍不推进，请停止后重新播放。' };
  const playbackClockCycleStatus = playbackClockNotice === null
    ? null
    : mediaState?.clock_health === 'buffering'
      ? { label: '缓冲中', color: 'warning' }
      : mediaState?.clock_health === 'recovering'
        ? { label: '播放时钟恢复中', color: 'warning' }
        : { label: '播放时钟停滞', color: 'error' };
  const pictureInPictureDocument = document as PictureInPictureDocument;
  const pictureInPictureSupported =
    Boolean(pictureInPictureSourceUrl) &&
    pictureInPictureDocument.pictureInPictureEnabled === true &&
    typeof (pictureInPictureVideoRef.current as PictureInPictureVideo | null)?.requestPictureInPicture === 'function';
  const videoProcessingStatus: ProcessingStatusKey = currentMediaIsVideo
    ? !videoProcessingEnabled
      ? 'disabled'
      : currentVideoCandidateFailed
      ? 'failed'
      : currentVideoCandidatePreparing
        ? 'processing'
        : currentVideoCandidateReady || currentVideoCandidatePresented
          ? 'ready'
          : mediaEngineCapabilities?.available
            ? 'configured'
            : 'unavailable'
    : 'not_applicable';
  const videoCycleStatus = !currentSource
    ? '等待导入'
    : !currentMediaIsVideo
      ? '当前音频素材不适用'
      : !videoProcessingEnabled
        ? '视频滤镜关闭，保持 Original'
        : currentVideoCandidateFailed
          ? '当前视频候选失败，等待重试'
          : currentVideoCandidateReady
            ? '候选已准备，等待周期提交'
            : currentVideoCandidatePreparing
              ? '候选处理中'
              : currentVideoCandidatePresented
                ? '候选已呈现，准备下一周期'
                : nextVideoPlan
                  ? '等待候选准备'
                  : getProcessingStatusLabel(videoProcessingStatus);
  const videoCycleStatusColor = !currentSource || !currentMediaIsVideo || !videoProcessingEnabled
    ? 'default'
    : currentVideoCandidateFailed
      ? 'error'
      : currentVideoCandidateReady || currentVideoCandidatePresented
        ? 'warning'
        : currentVideoCandidatePreparing
          ? 'processing'
          : getProcessingStatusColor(videoProcessingStatus);
  const audioProcessingStatus = getProcessingStatusKey(
    snapshot?.audio_processing_status,
    audioProcessingEnabled,
    Boolean(audioProcessingEnabled && snapshot?.source_media && mediaEngineCapabilities?.available),
  );
  const actualAudioOutputLabel = getActualAudioOutputLabel(audioOutputBackend);
  const actualAudioStreamVariantCount = getActualAudioStreamVariantCount(snapshot);
  const videoBackendStatusView = useMemo(
    () => projectMediaVideoBackendStatus(mediaVideoBackendStatus),
    [mediaVideoBackendStatus],
  );
  const actualAudioMixLabel = actualAudioStreamVariantCount === null
    ? '未上报（兼容旧快照）'
    : actualAudioOutputLabel === 'PortAudio'
      ? `${actualAudioStreamVariantCount} 条支路`
      : `未进入正式输出（配置 ${actualAudioStreamVariantCount} 条）`;
  const interludeValidationErrors = getInterludeValidationErrors(interludeDraft);
  const portAudioFallback = audioProcessingEnabled
    && Boolean(audioOutputBackend?.preferred_portaudio)
    && actualAudioOutputLabel === 'WebView';
  const activeAudioPresets = useMemo(
    () => audioActivePresetIds
      .map((id) => AUDIO_VALUE_PRESETS.find((preset) => preset.id === id))
      .filter((preset): preset is (typeof AUDIO_VALUE_PRESETS)[number] => Boolean(preset)),
    [audioActivePresetIds],
  );
  const activeInterludePresets = useMemo(
    () => (interludeRuntime?.preset_ids ?? [])
      .map((id) => AUDIO_VALUE_PRESETS.find((preset) => preset.id === id))
      .filter((preset): preset is (typeof AUDIO_VALUE_PRESETS)[number] => Boolean(preset)),
    [interludeRuntime?.preset_ids],
  );
  const configuredInterludePresetId = snapshot?.interlude?.audio_selection_mode === 'fixed'
    ? snapshot.interlude.audio_fixed_preset_id ?? 'p01'
    : snapshot?.interlude?.audio_preset_ids?.[0] ?? 'p01';
  const displayedInterludePreset = activeInterludePresets[0]
    ?? AUDIO_VALUE_PRESETS.find((preset) => preset.id === configuredInterludePresetId)
    ?? AUDIO_VALUE_PRESETS[0];
  const currentInterludeFileName = interludeRuntime?.status === 'playing' && interludeRuntime.file_name
    ? interludeRuntime.file_name
    : '无';
  const currentInterludePresetLabel = interludeRuntime?.status === 'playing' && activeInterludePresets.length > 0
    ? activeInterludePresets.map((preset) => `${preset.label}（${preset.id}）`).join('、')
    : `${displayedInterludePreset.label}（${displayedInterludePreset.id}）`;
  const interludePeriodRange = {
    minMs: snapshot?.interlude?.interval_min_ms ?? 8_000,
    maxMs: snapshot?.interlude?.interval_max_ms ?? 13_000,
  };
  const interludePresetPeriodRange = {
    minMs: snapshot?.interlude?.audio_variation_period_min_ms ?? 8_000,
    maxMs: snapshot?.interlude?.audio_variation_period_max_ms ?? 15_000,
  };
  const interludePresetPeriodActive = Boolean(
    snapshot?.interlude && snapshot.interlude.audio_selection_mode !== 'fixed',
  );
  const interludeVolumePercent = interludeVolumeDbToPercent(interludeDraft.volumeDb);
  const interludeStatusLabel = !snapshot?.interlude?.enabled
    ? '未启用'
    : snapshot.interlude.error || interludeRuntime?.status === 'failed'
      ? '本次已跳过'
      : interludeRuntime?.status === 'playing'
        ? '处理中'
        : snapshot.interlude.status === 'ready'
          ? '已就绪'
          : '等待音频';
  const interludeWaveformActive = Boolean(
    interludePlaybackActive
    && diagnosticFresh
    && diagnosticMessage
    && interludeRuntime
    && interludeRuntime.started_at_ms !== null
    && diagnosticMessage.sent_at_ms >= interludeRuntime.started_at_ms
  );
  const interludeWaveform = interludeWaveformActive
    ? diagnosticMessage?.line ?? []
    : [];
  const actualAudioBranches = useMemo(
    () => resolveAudioStreamBranches(
      snapshot?.audio_stream_params,
      snapshot?.audio_stream_variants,
      Boolean(
        audioProcessingEnabled
        && snapshot?.audio_processing_runtime
        && snapshot.audio_processing_status !== 'failed'
      ),
    ),
    [
      audioProcessingEnabled,
      snapshot?.audio_processing_runtime,
      snapshot?.audio_processing_status,
      snapshot?.audio_stream_params,
      snapshot?.audio_stream_variants,
    ],
  );
  const audioDisplayParams = actualAudioBranches[0] ?? mediaEffectParams?.audio ?? null;
  const displayedAudioEffectParams = useMemo(() => {
    if (!mediaEffectParams) return null;
    const audio = {
      ...mediaEffectParams.audio,
      current_formant_hz: diagnosticFresh
        && diagnosticMessage?.source === 'portaudio-mixed-pcm'
        && diagnosticMessage.current_formant_hz !== null
        ? diagnosticMessage.current_formant_hz
        : null,
    };
    return {
      ...mediaEffectParams,
      audio: mergeAudioDisplayParams(audio, audioDisplayParams),
    };
  }, [
    audioDisplayParams,
    diagnosticFresh,
    diagnosticMessage?.current_formant_hz,
    diagnosticMessage?.source,
    mediaEffectParams,
  ]);
  const audioCapabilityRows = useMemo(
    () => buildAudioCapabilityRows(
      audioDisplayParams,
      audioProcessingEnabled,
      actualAudioBranches.length > 0,
    ),
    [actualAudioBranches.length, audioDisplayParams, audioProcessingEnabled],
  );

  const audioParameterControls = mediaEffectParams ? (
    <AudioParameterControls
      value={mediaEffectParams.audio}
      disabled={!audioProcessingEnabled}
      showHeading={false}
      ambientSoundPath={ambientSoundPath}
      onChange={(field, value) => updateMediaEffectParam('audio', field, value)}
      onChooseAmbientSound={chooseAmbientSoundFile}
      onClearAmbientSound={() => setAmbientSoundPath(null)}
    />
  ) : null;

  return (
    <>
      <video
        ref={pictureInPictureVideoRef}
        muted
        playsInline
        preload="none"
        aria-hidden="true"
        style={{ position: 'fixed', width: 1, height: 1, opacity: 0, pointerEvents: 'none' }}
      />
      <DesktopShell>
        <DesktopColumn area="source" ariaLabel="媒体素材与播放控制">
          <PlaybackPoolPanel
            sources={sourceMediaPool}
            currentIndex={snapshot?.source_media_index ?? null}
            importBusy={importVideoBusy}
            importDisabled={importVideoBusy || runtimeResourceBusy}
            error={error}
            onImport={() => void importVideo()}
            onAppend={() => void appendVideosToPlaybackPool()}
            onDropFiles={(paths) => void appendVideosToPlaybackPool(paths)}
            onReplace={(sourcePath) => void replacePlaybackPoolItem(sourcePath)}
            onReorder={(sourcePaths) => void reorderPlaybackPool(sourcePaths)}
            onRemove={(sourcePath) => void removePlaybackPoolItem(sourcePath)}
            onClear={() => void clearPlaybackPool()}
          />

          <DesktopPanel
            title="播放控制"
            extra={<Typography.Text className="desktop-muted">{formatMediaTime(mediaDisplayedTime)} / {formatMediaTime(mediaDuration)}</Typography.Text>}
          >
            <dl className="desktop-playback-facts">
              <div><dt>播放进度</dt><dd>{mediaDuration > 0 ? Math.round(mediaDisplayedTime / mediaDuration * 100) : 0}%</dd></div>
              <div><dt>循环次数</dt><dd>{snapshot?.playback_pool_cycle ?? 0} · 第 {currentSource ? (snapshot?.source_media_index ?? 0) + 1 : 0}/{sourceMediaPool.length} 项</dd></div>
            </dl>
            {playbackClockNotice ? <Alert showIcon type={playbackClockNotice.type} message={playbackClockNotice.message} style={{ marginBottom: 10 }} /> : null}
            <Slider
              aria-label="播放进度"
              min={0}
              max={Math.max(mediaDuration, 1)}
              value={mediaDisplayedTime}
              disabled={!mediaState || mediaDuration <= 0}
              tooltip={{ formatter: (value) => formatMediaTime(value ?? 0) }}
              onChange={(value) => {
                if (!mediaState) return;
                setMediaSeekDraft({
                  value: clampMediaTime(value, mediaDuration),
                  playbackGeneration: mediaState.playback_generation,
                  committed: false,
                });
              }}
              onChangeComplete={(value) => {
                if (!mediaState) return;
                const currentTime = clampMediaTime(value, mediaDuration);
                setMediaSeekDraft({
                  value: currentTime,
                  playbackGeneration: mediaState.playback_generation,
                  committed: true,
                });
                clearFutureMediaCyclePlans();
                postPlaybackMediaControl({
                  version: 1,
                  type: 'playback-media-control',
                  action: 'seek',
                  current_time: currentTime,
                  playback_generation: mediaState.playback_generation,
                });
              }}
            />
            <div className="desktop-control-row desktop-playback-actions">
              <Button
                className="desktop-playback-action"
                type="primary"
                icon={<PlayCircleOutlined />}
                onClick={() => void startPlaybackFromHome()}
                disabled={!canStartPlayback}
                loading={playbackActionBusy === 'resume' || playerWindowBusy}
              >
                播放
              </Button>
              <Button
                className="desktop-playback-action"
                icon={<PauseCircleOutlined />}
                onClick={() => void runPlaybackAction('pause', 'pause_playback')}
                disabled={!canPause}
                loading={playbackActionBusy === 'pause'}
              >
                暂停
              </Button>
              <Button
                className="desktop-playback-action"
                icon={<PlayCircleOutlined />}
                onClick={() => void runPlaybackAction('resume', 'resume_playback')}
                disabled={!canResume}
                loading={playbackActionBusy === 'resume'}
              >
                继续
              </Button>
              <Button
                className="desktop-playback-action"
                danger
                aria-label="停止播放"
                icon={<StopOutlined />}
                onClick={() => void runPlaybackAction('stop', 'stop_playback')}
                disabled={!canStop}
                loading={playbackActionBusy === 'stop'}
              >
                停止
              </Button>
              <Button
                aria-label="切换画中画"
                icon={<PictureOutlined />}
                disabled={!mediaState || !pictureInPictureSupported}
                onClick={() => void togglePictureInPicture()}
              />
            </div>
            <div className="desktop-control-row">
              <Typography.Text className="desktop-muted">媒体音量</Typography.Text>
              <Button
                aria-label={mediaState?.muted ? '取消媒体静音' : '媒体静音'}
                icon={mediaState?.muted ? <MutedOutlined /> : <SoundOutlined />}
                disabled={!mediaState}
                onClick={() => postPlaybackMediaControl({ version: 1, type: 'playback-media-control', action: 'toggle-muted' })}
              />
              <Slider
                aria-label="媒体音量"
                min={0}
                max={1}
                step={0.01}
                value={mediaState?.volume ?? 1}
                disabled={!mediaState}
                style={{ flex: 1 }}
                onChange={(volume) => postPlaybackMediaControl({
                  version: 1,
                  type: 'playback-media-control',
                  action: 'set-volume',
                  volume,
                })}
              />
              <Typography.Text className="desktop-volume-value">{Math.round((mediaState?.muted ? 0 : mediaState?.volume ?? 1) * 100)}%</Typography.Text>
            </div>
          </DesktopPanel>

        </DesktopColumn>

        <DesktopColumn area="media" ariaLabel="声音、插话与视频处理参数">
          <div className="desktop-media-domain-lane desktop-media-domain-lane--audio">
            <DesktopPanel
              title="音频"
              titleIcon={<SoundOutlined />}
              subtitle="插话与普通声音参数"
              className="desktop-media-domain-panel desktop-audio-domain"
              extra={(
                <Space size={6} className="desktop-domain-title-actions">
                  <Typography.Text>普通声音处理</Typography.Text>
                  <Switch
                    size="small"
                    aria-label="声音处理"
                    checked={audioProcessingEnabled}
                    onChange={(checked) => void updateProcessingSwitches({
                      video_processing_enabled: videoProcessingEnabledRef.current,
                      audio_processing_enabled: checked,
                      realtime_audio_variant_enabled: false,
                    })}
                  />
                </Space>
              )}
            >
              <section className="desktop-interlude-status" aria-label="插话状态与周期">
                <InlineRecoveryAlert
                  message={snapshot?.interlude?.error ?? interludeRuntime?.error}
                  description="主视频与普通声音继续运行，本次插话已跳过。"
                  actionLabel="打开插话设置"
                  onAction={() => setInterludeDrawerOpen(true)}
                />
                <MediaCycleCard
                  title="插话声音周期"
                  icon={<AudioOutlined />}
                  range={interludePeriodRange}
                  changes={interludeRuntime?.file_cycle ?? 0}
                  progress={interludeRuntime?.file_progress_percent ?? 0}
                  status={interludeStatusLabel}
                  statusColor={interludeRuntime?.status === 'failed' ? 'error' : snapshot?.interlude?.enabled ? 'processing' : 'default'}
                  accent="#f0bd3e"
                  footer={(
                    <div className="desktop-control-row" style={{ marginTop: 6 }}>
                      <Typography.Text className="desktop-muted">插话音量</Typography.Text>
                      <Slider
                        aria-label="插话音量"
                        aria-valuetext={`${interludeVolumePercent}%`}
                        min={0}
                        max={100}
                        step={1}
                        value={interludeVolumePercent}
                        disabled={!snapshot?.interlude || interludeSaving}
                        style={{ flex: 1 }}
                        tooltip={{ formatter: (volumePercent) => `${volumePercent ?? interludeVolumePercent}%` }}
                        onChange={(volumePercent) => {
                          const volumeDb = interludeVolumePercentToDb(volumePercent);
                          setInterludeDraft((current) => ({ ...current, volumeDb }));
                          postInterludeVolumeControl(volumeDb);
                        }}
                        onChangeComplete={(volumePercent) => void persistInterludeVolume(interludeVolumePercentToDb(volumePercent))}
                      />
                      <Typography.Text className="desktop-volume-value" style={{ width: 52 }}>{interludeVolumePercent}%</Typography.Text>
                    </div>
                  )}
                />
                <MediaCycleCard
                  title="插话预设变化周期"
                  icon={<SoundOutlined />}
                  range={interludePresetPeriodRange}
                  changes={Math.max(0, (interludeRuntime?.preset_segment ?? 1) - 1)}
                  progress={interludeRuntime?.progress_percent ?? 0}
                  status={!interludePresetPeriodActive
                    ? '每次插话抽样'
                    : interludeRuntime?.status === 'playing'
                      ? `第 ${interludeRuntime.preset_segment} 段`
                      : '等待插话'}
                  statusColor={interludePresetPeriodActive ? 'processing' : 'default'}
                  accent="#31d7aa"
                />
                <AudioProcessingPanel
                  ariaLabel="插话音频状态"
                  actualOutput={<Tag color={interludeRuntime?.output_backend === 'portaudio' ? 'success' : 'processing'}>{interludeRuntime?.output_backend === 'portaudio' ? 'PortAudio' : interludeRuntime?.output_backend === 'webview' ? 'WebView' : '等待输出'}</Tag>}
                  processingStatus={<Tag color={interludeRuntime?.status === 'failed' ? 'error' : 'processing'}>{interludeStatusLabel}</Tag>}
                  currentPreset={activeInterludePresets.length > 0 ? <Button type="link" size="small" onClick={() => setInterludePresetDrawerOpen(true)}>查看本段 {activeInterludePresets.length} 套</Button> : <Tag>等待本段抽样</Tag>}
                >
                  <svg
                    className="desktop-interlude-waveform"
                    viewBox="0 0 320 48"
                    role="img"
                    aria-label={interludeWaveformActive
                      ? '插话混入后的最终音频诊断波形'
                      : '当前没有可用的插话最终混音诊断波形'}
                  >
                    <polyline fill="none" stroke="#31d7aa" strokeWidth="1.5" points={interludeWaveform.map((value, index, values) => `${values.length <= 1 ? 0 : index / (values.length - 1) * 320},${24 - value * 20}`).join(' ')} />
                  </svg>
                  <Typography.Text className="desktop-muted">
                    {interludeWaveformActive
                      ? '实时 · 插话混音后 PCM（最终混音）'
                      : snapshot?.interlude?.enabled ? '等待插话播放' : '插话未启用'}
                  </Typography.Text>
                </AudioProcessingPanel>
              </section>

              <InlineRecoveryAlert
                message={!mediaEngineCapabilities?.available ? mediaEngineCapabilities?.reason ?? '本地媒体引擎不可用' : null}
                actionLabel="重新检查"
                onAction={() => refreshRuntimeResourceCapabilities(['media'], runtimeResourceActionTokenRef.current)}
              />
              <InlineRecoveryAlert
                message={audioProcessingStatus === 'failed' ? snapshot?.fallback_reason ?? '普通声音处理失败，已保留原声' : null}
                description={audioPeriodActive ? '已回退源音轨，可重新准备下一声音周期。' : '已回退源音轨，继续播放后可重试。'}
                actionLabel="重试声音处理"
                busy={audioProcessingStatus === 'processing'}
                onAction={audioPeriodActive ? retryAudioProcessing : undefined}
              />
              <MediaCycleCard
                title="声音周期"
                icon={<SoundOutlined />}
                range={audioPeriodRange}
                changes={audioVariationCycle}
                progress={audioProgressPercent}
                status={!audioProcessingEnabled ? '已关闭' : playbackClockCycleStatus?.label ?? getProcessingStatusLabel(audioProcessingStatus)}
                statusColor={!audioProcessingEnabled ? 'default' : playbackClockCycleStatus?.color ?? getProcessingStatusColor(audioProcessingStatus)}
                accent="#df66ed"
                editable
                onRangeChange={(range) => setAudioPeriodRange(normalizeAudioPeriodRange(range.minMs, range.maxMs))}
              />
              <AudioProcessingPanel
                ariaLabel="普通声音处理状态"
                actualOutput={<Tag color={actualAudioOutputLabel === 'PortAudio' ? 'success' : 'processing'}>{actualAudioOutputLabel}</Tag>}
                processingStatus={<Tag color={getProcessingStatusColor(audioProcessingStatus)}>{getProcessingStatusLabel(audioProcessingStatus)}</Tag>}
                currentPreset={activeAudioPresets.length > 0 ? <Button type="link" size="small" onClick={() => setAudioPresetDrawerOpen(true)}>查看本周期 {activeAudioPresets.length} 套</Button> : <Tag>等待首轮</Tag>}
              >
                <AudioDiagnosticDisplay
                  ref={audioDiagnosticDisplayRef}
                  active={documentVisible && snapshot?.playback_state?.toLowerCase() === 'playing'}
                  interludePlaybackActive={interludePlaybackActive}
                  onSummaryChange={setDiagnosticSummary}
                />
                {portAudioFallback ? <Alert type="warning" showIcon message="PortAudio 已回退 WebView" style={{ marginTop: 8 }} /> : null}
              </AudioProcessingPanel>
              {displayedAudioEffectParams ? (
                <MediaParameterPanels value={displayedAudioEffectParams} sections={AUDIO_PARAMETER_SECTIONS} />
              ) : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="正在读取声音参数" />}
            </DesktopPanel>

            <DesktopPanel
              title="插话声音预设"
              className="desktop-media-domain-panel desktop-interlude-preset-domain"
            >
              <div className="desktop-interlude-current-summary" aria-label="当前插话文件与预设" aria-live="polite">
                <div>
                  <Typography.Text className="desktop-domain-subtitle">当前文件</Typography.Text>
                  <Typography.Text strong ellipsis={{ tooltip: currentInterludeFileName }}>{currentInterludeFileName}</Typography.Text>
                </div>
                <div>
                  <Typography.Text className="desktop-domain-subtitle">当前预设</Typography.Text>
                  <Typography.Text strong ellipsis={{ tooltip: currentInterludePresetLabel }}>{currentInterludePresetLabel}</Typography.Text>
                </div>
              </div>
              <section aria-label="插话声音预设当前参数">
                <div className="desktop-preset-parameter-grid">
                  {AUDIO_PRESET_FIELD_DEFINITIONS.map((field) => (
                    <InterludePresetParameterRow
                      key={field.key}
                      label={field.label}
                      value={formatAudioPresetFieldValue(field.key, displayedInterludePreset.values[field.key], field.unit, field.digits)}
                    />
                  ))}
                </div>
              </section>
            </DesktopPanel>
          </div>

          <div className="desktop-media-domain-lane desktop-media-domain-lane--video">
            <DesktopPanel
              title="画面"
              titleIcon={<PictureOutlined />}
              subtitle="视频周期与视觉参数"
              className="desktop-media-domain-panel desktop-video-domain"
              extra={(
                <Space size={4} className="desktop-domain-title-actions">
                  <Typography.Text>视频处理</Typography.Text>
                  <Switch
                    size="small"
                    aria-label="视频处理"
                    checked={videoProcessingEnabled}
                    onChange={(checked) => void updateProcessingSwitches({
                      video_processing_enabled: checked,
                      audio_processing_enabled: audioProcessingEnabledRef.current,
                      realtime_audio_variant_enabled: false,
                    })}
                  />
                  <Button icon={<ReloadOutlined />} size="small" disabled={!currentMediaIsVideo} onClick={() => void resetVideoEffectParams()}>恢复默认</Button>
                </Space>
              )}
            >
            <InlineRecoveryAlert
              message={currentMediaIsVideo && !mediaEngineCapabilities?.available ? mediaEngineCapabilities?.reason ?? '本地媒体引擎不可用' : null}
              actionLabel="重新检查"
              onAction={() => refreshRuntimeResourceCapabilities(['media'], runtimeResourceActionTokenRef.current)}
            />
            <InlineRecoveryAlert
              message={currentMediaIsVideo && videoProcessingStatus === 'failed' ? mediaVideoBackendStatus?.demotion_reason ?? '视频处理失败，已保留源视频' : null}
              description={videoStreamActive ? '已回退源视频，可重新准备下一视频周期。' : '已回退源视频，继续播放后可重试。'}
              actionLabel="重试视频处理"
              busy={mediaProcessingBusy === 'video'}
              onAction={videoStreamActive ? retryVideoProcessing : undefined}
            />
            <div className="desktop-status-line" aria-label="视频实际运行后端">
              <span>实际画面后端</span>
              <Tag color={videoBackendStatusView.effects_applied ? 'success' : videoBackendStatusView.valid ? 'warning' : 'default'}>
                {videoBackendStatusView.label}
              </Tag>
            </div>
            <Typography.Text className="desktop-muted">{videoBackendStatusView.detail}</Typography.Text>
            <MediaCycleCard
              title="视频周期"
              icon={<PictureOutlined />}
              range={videoPeriodRange}
              changes={runtimeCycle}
              progress={runtimeProgressPercent}
              status={videoCycleStatus}
              statusColor={videoCycleStatusColor}
              accent="#38b8f8"
              editable={currentMediaIsVideo}
              onRangeChange={(range) => setVideoPeriodRange(normalizeVideoPeriodRange(range.minMs, range.maxMs))}
            />
            {mediaEffectParams ? (
              <MediaParameterPanels
                value={mediaEffectParams}
                loading={false}
                error={null}
                sections={VIDEO_PARAMETER_SECTIONS}
              />
            ) : (
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="正在读取正式媒体参数" />
            )}
            </DesktopPanel>
          </div>
        </DesktopColumn>

        <DesktopColumn area="output" ariaLabel="最终输出、话术与本地运行状态">
          <DesktopPanel
            title="最终效果窗口"
            className="desktop-output-panel desktop-compact-panel"
            extra={<Button size="small" type="primary" icon={<FolderOpenOutlined />} loading={playerWindowBusy} onClick={() => void openFinalEffectWindowFromHome()}>{diagnosticFresh ? '聚焦' : '打开'}</Button>}
          >
            <Typography.Paragraph className="desktop-muted" style={{ marginBottom: 8 }}>{diagnosticFresh ? '窗口已连接' : '窗口尚未打开'} · 单实例输出</Typography.Paragraph>
            <Segmented
              block
              aria-label="输出查看模式"
              options={[
                { label: '画中画', value: '画中画', disabled: !mediaState || !pictureInPictureSupported },
                { label: '独立窗口', value: '独立窗口' },
              ]}
              value={pictureInPictureActive ? '画中画' : '独立窗口'}
              onChange={(mode) => {
                if (mode === '画中画') {
                  if (!mediaState || !pictureInPictureSupported) return;
                  void togglePictureInPicture();
                  return;
                }
                if (pictureInPictureActive) void togglePictureInPicture();
                void openFinalEffectWindowFromHome();
              }}
            />
          </DesktopPanel>

          <DesktopPanel title="声音功能" className="desktop-compact-panel">
            <div className="desktop-feature-actions">
              <Button icon={<SettingOutlined />} onClick={() => setAudioSettingsDrawerOpen(true)}>模式修改</Button>
              <Button icon={<ThunderboltOutlined />} onClick={() => setInterludeDrawerOpen(true)}>随机插话</Button>
            </div>
          </DesktopPanel>

          <PortAudioDevicePanel
            available={Boolean(audioOutputBackend?.available)}
            running={Boolean(audioOutputBackend?.running)}
            actualOutput={actualAudioOutputLabel}
            processingStatus={audioOutputDevicesError ?? audioOutputBackend?.reason ?? (!currentSource ? '等待媒体' : audioOutputBackend?.running ? '处理中' : '等待播放')}
            hostApi={audioOutputHostApiFilter}
            hasAsioDevice={hasAsioOutputDevice}
            devices={audioOutputDevices}
            deviceId={audioOutputDeviceIdInput}
            memoryBufferKib={audioOutputMemoryKibInput}
            appliedDeviceId={audioOutputDeviceId}
            appliedMemoryBufferKib={audioOutputMemoryKib}
            busy={audioOutputBusy}
            devicesRefreshing={audioOutputDevicesRefreshing}
            onHostApiChange={(hostApi) => {
              audioOutputDraftTouchedRef.current = true;
              setAudioOutputHostApiFilter(hostApi);
              setAudioOutputDeviceIdInput((current) => {
                if (hostApi === 'all' || current === null) return current;
                return audioOutputDevices.some(
                  (device) => device.id === current && device.host_api.trim().toLowerCase() === hostApi,
                ) ? current : null;
              });
            }}
            onDeviceChange={(deviceId) => {
              audioOutputDraftTouchedRef.current = true;
              setAudioOutputDeviceIdInput(deviceId);
            }}
            onMemoryBufferChange={(value) => {
              audioOutputDraftTouchedRef.current = true;
              setAudioOutputMemoryKibInput(value);
            }}
            onMemoryBufferBlur={() => {
              const value = audioOutputMemoryKibInput;
              if (typeof value !== 'number' || !Number.isInteger(value) || value < PORTAUDIO_MIN_MEMORY_BUFFER_KIB || value > PORTAUDIO_MAX_MEMORY_BUFFER_KIB) {
                setAudioOutputMemoryKibInput(audioOutputMemoryKib);
              }
            }}
            onRefreshDevices={() => void refreshAudioOutputDevices()}
            onApply={() => {
              const value = audioOutputMemoryKibInput;
              if (typeof value !== 'number' || !Number.isInteger(value) || value < PORTAUDIO_MIN_MEMORY_BUFFER_KIB || value > PORTAUDIO_MAX_MEMORY_BUFFER_KIB) return;
              void applyAudioOutputBackend(true, audioOutputDeviceIdInput, value).then((status) => {
                if (!status || getActualAudioOutputLabel(status) !== 'PortAudio') return;
                audioOutputDraftTouchedRef.current = false;
                syncAudioOutputConfiguration(status, true);
              });
            }}
            onTestTone={() => {
              setAudioOutputBusy(true);
              void invokeAudioOutputBackendStatus('play_portaudio_test_tone', { request: { frequency_hz: 440, duration_ms: 400, amplitude: 0.12 } })
                .then(publishAudioOutputBackend)
                .catch((cause) => setError(getDisplayErrorMessage(cause, 'PortAudio 测试音失败')))
                .finally(() => setAudioOutputBusy(false));
            }}
          />

          <DesktopPanel title="话术功能">
            <div className="desktop-speech-summary">
              <div className="desktop-status-line"><span>固定话术</span><Tag color={fixedSpeechState.status === 'failed' ? 'error' : fixedSpeechBusy ? 'processing' : 'default'}>{fixedSpeechStatusLabel}</Tag></div>
              <div className="desktop-status-line"><span>本地预制</span><strong>{fixedSpeechPresets.length}/10</strong></div>
              <Button icon={<MessageOutlined />} onClick={() => setFixedSpeechDrawerOpen(true)}>打开固定话术</Button>
            </div>
          </DesktopPanel>

          <DesktopPanel title="实时处理引擎">
            <Steps
              current={diagnosticFresh ? 3 : audioProcessingEnabled ? 2 : videoProcessingEnabled ? 1 : 0}
              direction="vertical"
              size="small"
              items={[
                { title: '素材', description: currentSource ? '已就绪' : '等待导入' },
                { title: '视频', description: getProcessingStatusLabel(videoProcessingStatus) },
                { title: '声音', description: getProcessingStatusLabel(audioProcessingStatus) },
                { title: '输出', description: diagnosticFresh ? '已连接' : '等待窗口' },
              ]}
            />
            <Typography.Paragraph className="desktop-muted" style={{ marginTop: 10, marginBottom: 0 }}>
              当前处理以真实快照和最终效果窗口诊断为准。
            </Typography.Paragraph>
          </DesktopPanel>

          <DesktopPanel title="本地运行状态" extra={<Tag color="success">本机</Tag>}>
            <div className="desktop-local-status">
              <div className="desktop-status-line"><span>媒体引擎</span><Tag color={mediaEngineCapabilities?.available ? 'success' : 'warning'}>{mediaEngineCapabilities?.available ? '就绪' : '不可用'}</Tag></div>
              <div className="desktop-status-line"><span>音频出口</span><Tag color={actualAudioOutputLabel === 'PortAudio' ? 'success' : 'processing'}>{actualAudioOutputLabel}</Tag></div>
              <div className="desktop-status-line"><span>最终窗口</span><Tag color={diagnosticFresh ? 'success' : 'default'}>{diagnosticFresh ? '已连接' : '未连接'}</Tag></div>
              <div className="desktop-status-line"><span>本地源</span><Tag>{currentSource ? '可用' : '未导入'}</Tag></div>
              <div className="desktop-status-line"><span>当前条目</span><strong>{currentSource ? `第 ${(snapshot?.source_media_index ?? 0) + 1}/${sourceMediaPool.length} 项` : '等待导入'}</strong></div>
              <div className="desktop-status-line"><span>播放池循环</span><strong>{snapshot?.playback_pool_cycle ?? 0}</strong></div>
            </div>
          </DesktopPanel>
        </DesktopColumn>
      </DesktopShell>

      <FeatureDrawer
        title="随机插话"
        description="递归扫描本地媒体目录及其子目录；视频仅使用音轨，不显示画面，并在插话期间自动压低主音轨。"
        width={560}
        open={interludeDrawerOpen}
        onClose={() => setInterludeDrawerOpen(false)}
        summary={(
          <>
            <Tag color={interludeDraft.enabled ? 'success' : 'default'}>{interludeDraft.enabled ? '已启用' : '未启用'}</Tag>
            <Tag color={interludeDirty ? 'warning' : 'blue'}>{interludeDirty ? '有未保存更改' : '配置已同步'}</Tag>
            <Tag>{interludeDraft.directory ? '媒体目录已选择' : '未选择媒体目录'}</Tag>
            <Tag>已保存受支持文件 {snapshot?.interlude?.audio_count ?? 0}/{MAX_INTERLUDE_AUDIO_FILES}</Tag>
            <Tag>{interludeDraft.audioSelectionMode === 'fixed'
              ? `固定 ${interludeDraft.audioFixedPresetId}`
              : interludeDraft.audioMixEnabled
                ? `随机合一 ${interludeDraft.audioMixPickMin}–${interludeDraft.audioMixPickMax} 轨`
                : '随机单轨'}</Tag>
            {interludeDraft.audioSelectionMode === 'random' ? (
              <Tag>{interludeDraft.audioSelectionMode === 'random' ? '周期换组' : '固定预设'}</Tag>
            ) : null}
            <Button
              className="interlude-compact-save"
              type="primary"
              size="small"
              loading={interludeSaving}
              disabled={interludeValidationErrors.length > 0}
              onClick={() => void saveInterludeConfig()}
            >
              保存插话配置
            </Button>
          </>
        )}
        footer={(
          <Button type="primary" loading={interludeSaving} disabled={interludeValidationErrors.length > 0} onClick={() => void saveInterludeConfig()}>
            保存插话配置
          </Button>
        )}
      >
        {interludeValidationErrors.length > 0 ? (
          <Alert
            type="warning"
            showIcon
            message="请检查插话配置"
            description={interludeValidationErrors.join('；')}
          />
        ) : null}
        {interludeSaveError ? <Alert type="error" showIcon message={interludeSaveError} /> : null}
        <FeatureDrawerSection
          title="插话配置"
          description="所有可编辑项集中在这里，按媒体来源、触发周期、声音预设和混音参数依次设置。"
        >
          <div className="feature-drawer-parameter-groups">
            <div className="feature-drawer-parameter-group">
              <div className="feature-drawer-toggle-row">
                <div><strong>启用状态</strong><Typography.Text>关闭后保留当前设置，但不会在播放过程中触发插话。</Typography.Text></div>
                <Switch aria-label="启用随机插话" checked={interludeDraft.enabled} onChange={(enabled) => updateInterludeDraft({ enabled })} />
              </div>
              {interludePlaybackNotice ? <Alert type="warning" showIcon message={interludePlaybackNotice} /> : (
                <Typography.Text className="desktop-muted">插话按独立随机周期运行，不会改变视频循环进度。</Typography.Text>
              )}
            </div>

            <div className="feature-drawer-parameter-group">
              <strong>媒体来源</strong>
              <FeatureDrawerField
                label="插话媒体目录"
                htmlFor="interlude-audio-directory"
                hint="音频：mp3、wav、m4a、aac、ogg、flac；视频：mp4、mov、mkv、avi、webm、m4v、ts、m2ts、flv、wmv、3gp。视频仅使用音轨，不显示画面。"
              >
                <Space.Compact style={{ width: '100%' }}>
                  <Input id="interlude-audio-directory" readOnly value={interludeDraft.directory ?? ''} placeholder="请选择媒体文件夹" />
                  <Button onClick={() => void chooseInterludeDirectory()}>选择媒体文件夹</Button>
                </Space.Compact>
              </FeatureDrawerField>
            </div>

            <div className="feature-drawer-parameter-group">
              <strong>插话声音周期</strong>
              <Typography.Text className="desktop-muted">设置两次插话之间的随机等待范围，每次从该范围重新取值。</Typography.Text>
              <div className="feature-drawer-field-grid">
                <CompactNumberField ariaLabel="插话声音周期最小值（秒）" label="最小周期" unit="秒" step={0.5} min={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.min)} max={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.max)} value={interludeIntervalMsToSeconds(interludeDraft.intervalMinMs)} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMinMs: interludeIntervalSecondsToMs(value) })} />
                <CompactNumberField ariaLabel="插话声音周期最大值（秒）" label="最大周期" unit="秒" step={0.5} min={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.min)} max={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.max)} value={interludeIntervalMsToSeconds(interludeDraft.intervalMaxMs)} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMaxMs: interludeIntervalSecondsToMs(value) })} />
              </div>
            </div>

            <div className="feature-drawer-parameter-group">
              <strong>独立声音轨</strong>
              <Typography.Text className="desktop-muted">固定一条预设，或从插话自己的 22 项预设池随机抽样；配置变化从下一段开始生效。</Typography.Text>
              <FeatureDrawerField label="音轨选择方式" htmlFor="interlude-audio-selection-mode">
                <Select
                  id="interlude-audio-selection-mode"
                  aria-label="插话音轨选择方式"
                  value={interludeDraft.audioSelectionMode}
                  options={[
                    { value: 'fixed', label: '固定选择' },
                    { value: 'random', label: '从所选音轨随机' },
                  ]}
                  onChange={(audioSelectionMode: InterludeAudioSelectionMode) => updateInterludeDraft({ audioSelectionMode })}
                  style={{ width: '100%' }}
                />
              </FeatureDrawerField>

              {interludeDraft.audioSelectionMode === 'fixed' ? (
                <FeatureDrawerField label="固定声音预设" htmlFor="interlude-fixed-audio-preset">
                  <Select
                    id="interlude-fixed-audio-preset"
                    aria-label="插话固定声音预设"
                    value={interludeDraft.audioFixedPresetId}
                    options={AUDIO_VALUE_PRESETS.map((preset) => ({ value: preset.id, label: preset.label }))}
                    onChange={(audioFixedPresetId) => updateInterludeDraft({ audioFixedPresetId })}
                    style={{ width: '100%' }}
                  />
                </FeatureDrawerField>
              ) : (
                <>
                  <div className="feature-drawer-toggle-row">
                    <div><strong>随机多轨合一</strong><Typography.Text>关闭时每段只抽一轨；开启后随机抽取 1–4 轨等权合一。</Typography.Text></div>
                    <Switch aria-label="随机多轨合一" checked={interludeDraft.audioMixEnabled} onChange={(audioMixEnabled) => updateInterludeDraft({ audioMixEnabled })} />
                  </div>
                  {interludeDraft.audioMixEnabled ? (
                    <div className="feature-drawer-field-grid feature-drawer-field-grid-compact">
                      <FeatureDrawerField label="最少随机轨数">
                        <InputNumber
                          aria-label="插话最少随机轨数"
                          min={1}
                          max={AUDIO_MIX_PICK_HARD_MAX}
                          value={interludeDraft.audioMixPickMin}
                          onChange={(value) => typeof value === 'number' && updateInterludeDraft({
                            audioMixPickMin: normalizeAudioMixPickMin(value, interludeDraft.audioMixPickMax),
                          })}
                          style={{ width: '100%' }}
                        />
                      </FeatureDrawerField>
                      <FeatureDrawerField label="最多随机轨数">
                        <InputNumber
                          aria-label="插话最多随机轨数"
                          min={1}
                          max={AUDIO_MIX_PICK_HARD_MAX}
                          value={interludeDraft.audioMixPickMax}
                          onChange={(value) => {
                            if (typeof value !== 'number') return;
                            const audioMixPickMax = normalizeAudioMixPickMax(value);
                            updateInterludeDraft({
                              audioMixPickMax,
                              audioMixPickMin: normalizeAudioMixPickMin(interludeDraft.audioMixPickMin, audioMixPickMax),
                            });
                          }}
                          style={{ width: '100%' }}
                        />
                      </FeatureDrawerField>
                    </div>
                  ) : null}
                  <FeatureDrawerField label="插话声音预设" hint="22 项均可进入插话随机池，至少保留一项；p21、p22 为明显强效果。">
                    <Space direction="vertical" size="small" style={{ width: '100%' }}>
                      <Button
                        size="small"
                        aria-label="全选插话声音预设"
                        onClick={() => updateInterludeDraft({ audioPresetIds: AUDIO_VALUE_PRESETS.map((preset) => preset.id) })}
                      >全选</Button>
                      <Checkbox.Group
                        aria-label="插话声音预设"
                        value={interludeDraft.audioPresetIds}
                        onChange={(values) => updateInterludeDraft({ audioPresetIds: normalizeInterludeAudioPresetIds(values.map(String)) })}
                        style={{ width: '100%' }}
                      >
                        <div className="feature-drawer-checkbox-grid">
                          {AUDIO_VALUE_PRESETS.map((preset) => <Checkbox key={preset.id} value={preset.id}>{preset.label}</Checkbox>)}
                        </div>
                      </Checkbox.Group>
                    </Space>
                  </FeatureDrawerField>

                  <div className="feature-drawer-field-grid feature-drawer-field-grid-compact">
                    <FeatureDrawerField label="插话预设变化周期最小值">
                      <CompactNumberField ariaLabel="插话预设变化周期最小值（秒）" unit="秒" min={INTERLUDE_PRESET_PERIOD_LIMITS.min / 1000} max={INTERLUDE_PRESET_PERIOD_LIMITS.max / 1000} step={0.5} value={interludeDraft.audioVariationPeriodMinMs / 1000} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ audioVariationPeriodMinMs: value * 1000 })} />
                    </FeatureDrawerField>
                    <FeatureDrawerField label="插话预设变化周期最大值">
                      <CompactNumberField ariaLabel="插话预设变化周期最大值（秒）" unit="秒" min={INTERLUDE_PRESET_PERIOD_LIMITS.min / 1000} max={INTERLUDE_PRESET_PERIOD_LIMITS.max / 1000} step={0.5} value={interludeDraft.audioVariationPeriodMaxMs / 1000} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ audioVariationPeriodMaxMs: value * 1000 })} />
                    </FeatureDrawerField>
                  </div>
                </>
              )}
              <Alert
                type="info"
                showIcon
                message={actualAudioOutputLabel === 'PortAudio' ? '当前由 PortAudio 应用插话声音预设。' : '当前由 WebView 本地处理插话声音预设。'}
                description="PortAudio 与 WebView 均应用固定或随机预设与多轨合一；周期按当前插话媒体时间推进，到期后保持同一文件和播放位置并切换声音预设。"
              />
            </div>

            <div className="feature-drawer-parameter-group">
              <strong>音量与混音</strong>
              <Typography.Text className="desktop-muted">设置插话期间主媒体的压低幅度和进入、恢复过渡。</Typography.Text>
              <div className="feature-drawer-field-grid">
                <CompactNumberField ariaLabel="原声压低" label="原声压低" unit="dB" min={INTERLUDE_LIMITS.duckingDepthDb.min} max={INTERLUDE_LIMITS.duckingDepthDb.max} step={0.5} value={interludeDraft.duckingDepthDb} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingDepthDb: value })} />
                <CompactNumberField ariaLabel="原声压低过渡" label="原声压低过渡" unit="ms" min={INTERLUDE_LIMITS.duckingAttackMs.min} max={INTERLUDE_LIMITS.duckingAttackMs.max} value={interludeDraft.duckingAttackMs} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingAttackMs: value })} />
                <CompactNumberField ariaLabel="原声恢复过渡" label="原声恢复过渡" unit="ms" min={INTERLUDE_LIMITS.duckingReleaseMs.min} max={INTERLUDE_LIMITS.duckingReleaseMs.max} value={interludeDraft.duckingReleaseMs} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingReleaseMs: value })} />
              </div>
            </div>
          </div>
          {snapshot?.interlude?.error ? <Alert type="error" showIcon message={snapshot.interlude.error} style={{ marginTop: 10 }} /> : null}
        </FeatureDrawerSection>
      </FeatureDrawer>

      <FeatureDrawer
        title="固定话术"
        description="调用本机系统语音朗读当前文案；朗读期间静音原声，结束后自动恢复。"
        width={560}
        open={fixedSpeechDrawerOpen}
        onClose={() => setFixedSpeechDrawerOpen(false)}
        summary={(
          <>
            <Tag color="green">本机系统语音</Tag>
            <Tag color={fixedSpeechState.status === 'failed' ? 'red' : fixedSpeechBusy ? 'processing' : 'blue'}>状态：{fixedSpeechStatusLabel}</Tag>
            <Tag>预制：{fixedSpeechPresets.length}/10</Tag>
          </>
        )}
        footer={(
          <>
            <Popconfirm title="删除这条预制文本？" description="删除后无法恢复。" onConfirm={deleteSelectedFixedSpeechPreset}>
              <Button danger disabled={!selectedFixedSpeechPreset}>删除预制文本</Button>
            </Popconfirm>
            <Button disabled={!fixedSpeechBusy} onClick={cancelCurrentFixedSpeech}>停止朗读</Button>
            <Button disabled={fixedSpeechSaveDisabledReason !== null} onClick={saveFixedSpeechPresetFromForm}>{selectedFixedSpeechPreset ? '更新文案' : '添加文案'}</Button>
            <Button type="primary" loading={fixedSpeechBusy} disabled={fixedSpeechPlayDisabledReason !== null} title={fixedSpeechPlayDisabledReason ?? undefined} onClick={playCurrentFixedSpeechText}>播放当前文案</Button>
          </>
        )}
      >
        <Alert type={fixedSpeechNotice.type} showIcon message={fixedSpeechNotice.message} />

        <FeatureDrawerSection title="预制文案" description="最多保存 10 条本地文案，可选择后修改或删除。">
          <FeatureDrawerField label="已保存的文案" htmlFor="fixed-speech-preset">
            <Select
              id="fixed-speech-preset"
              aria-label="固定话术预制选择"
              allowClear
              placeholder="选择一条本地预制文本"
              value={selectedFixedSpeechPresetId ?? undefined}
              options={fixedSpeechPresets.map((preset) => ({ label: preset.title, value: preset.id }))}
              onChange={(value) => applyFixedSpeechPresetSelection(typeof value === 'string' ? value : null)}
              style={{ width: '100%' }}
            />
          </FeatureDrawerField>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="当前文案" description="标题用于保存预制；正文限制 1–500 个字符。">
          <div className="feature-drawer-form-stack">
            <FeatureDrawerField label="预制标题" htmlFor="fixed-speech-title">
              <Input id="fixed-speech-title" aria-label="固定话术预制标题" placeholder="输入便于识别的预制标题" maxLength={80} value={fixedSpeechPresetTitle} onChange={(event) => { setFixedSpeechPresetTitle(event.target.value); setFixedSpeechFormError(null); }} />
            </FeatureDrawerField>
            <FeatureDrawerField label="朗读正文" htmlFor="fixed-speech-text" hint="仅调用本机系统语音，不下载模型、不识别人声。">
              <Input.TextArea id="fixed-speech-text" aria-label="固定话术文本" rows={8} maxLength={500} showCount value={fixedSpeechText} placeholder="输入要由系统语音朗读的文字" onChange={(event) => { setFixedSpeechText(event.target.value); setFixedSpeechFormError(null); }} />
            </FeatureDrawerField>
            {fixedSpeechFormError ? <Alert type="error" showIcon message={fixedSpeechFormError} /> : null}
          </div>
        </FeatureDrawerSection>
      </FeatureDrawer>

      <FeatureDrawer
        title="高级声音设置"
        description="管理普通声音处理、PortAudio 输出、多轨预设和参数生效状态。"
        width={760}
        open={audioSettingsDrawerOpen}
        onClose={() => setAudioSettingsDrawerOpen(false)}
        summary={(
          <>
            <Tag color={audioProcessingEnabled ? 'success' : 'default'}>{audioProcessingEnabled ? '声音处理已开启' : '声音处理未开启'}</Tag>
            <Tag color={getProcessingStatusColor(audioProcessingStatus)}>{getProcessingStatusLabel(audioProcessingStatus)}</Tag>
            <Tag>实际输出：{actualAudioOutputLabel}</Tag>
            <Tag>实际混音：{actualAudioMixLabel}</Tag>
          </>
        )}
        footer={(
          <Button onClick={rerollSubtleAudioParams} disabled={!mediaEffectParams}>重新生成本周期参数</Button>
        )}
      >
        <FeatureDrawerSection
          title="处理与输出"
          description="普通声音与视频处理开关彼此独立；声音参数由自动周期生成独立候选文件。"
              extra={<Switch aria-label="高级声音处理" checked={audioProcessingEnabled} onChange={(checked) => void updateProcessingSwitches({ video_processing_enabled: videoProcessingEnabledRef.current, audio_processing_enabled: checked, realtime_audio_variant_enabled: false })} />}
        >
          <div className="feature-drawer-status-grid">
            <div><Typography.Text>处理状态</Typography.Text><strong>{getProcessingStatusLabel(audioProcessingStatus)}</strong></div>
            <div><Typography.Text>当前出口</Typography.Text><strong>{actualAudioOutputLabel}</strong></div>
            <div><Typography.Text>混音支路</Typography.Text><strong>{actualAudioMixLabel}</strong></div>
          </div>
        </FeatureDrawerSection>

        <FeatureDrawerSection
          title="多轨与预设"
          description="从已勾选预设中抽样；开启多轨后可配置每轮随机合并数量。"
          extra={(
            <Space size="small" wrap>
              <Tag color={audioValuePresetIds.length > 0 ? 'blue' : 'error'}>已选 {audioValuePresetIds.length} 项</Tag>
              {audioActivePresetIds.length > 0 ? <Button type="link" size="small" aria-label="查看当前声音预设参数" onClick={() => setAudioPresetDrawerOpen(true)}>查看本周期 {audioActivePresetIds.length} 套</Button> : null}
            </Space>
          )}
        >
          <div className="feature-drawer-toggle-row">
            <div><strong>多轨合并</strong><Typography.Text>将多套声音预设合并为当前输出。</Typography.Text></div>
            <Switch aria-label="多轨合并" checked={audioMixEnabled} disabled={!audioProcessingEnabled} onChange={setAudioMixEnabled} />
          </div>
          {audioMixEnabled ? (
            <div className="feature-drawer-field-grid feature-drawer-field-grid-compact">
              <FeatureDrawerField label="最少随机轨数">
                <InputNumber aria-label="最少随机轨数" min={1} max={AUDIO_MIX_PICK_HARD_MAX} value={audioMixPickMin} onChange={(value) => typeof value === 'number' && setAudioMixPickMin(normalizeAudioMixPickMin(value, audioMixPickMax))} style={{ width: '100%' }} />
              </FeatureDrawerField>
              <FeatureDrawerField label="最多随机轨数">
                <InputNumber aria-label="最多随机轨数" min={1} max={AUDIO_MIX_PICK_HARD_MAX} value={audioMixPickMax} onChange={(value) => typeof value === 'number' && setAudioMixPickMax(normalizeAudioMixPickMax(value))} style={{ width: '100%' }} />
              </FeatureDrawerField>
            </div>
          ) : null}
          <Space wrap size="small" style={{ marginBottom: 10 }}>
            <Button size="small" onClick={() => setAudioValuePresetIds([...DEFAULT_AUDIO_VALUE_PRESET_IDS])}>选择默认 20 项</Button>
            <Button size="small" onClick={() => setAudioValuePresetIds(AUDIO_VALUE_PRESETS.map((preset) => preset.id))}>全选 22 项</Button>
          </Space>
          <FeatureDrawerField label="声音参数值预设" hint="1–20 为默认低感知随机池；21、22 为可手动选择的明显效果。">
            <Checkbox.Group aria-label="声音参数值预设" value={audioValuePresetIds} disabled={!audioProcessingEnabled} onChange={(values) => setAudioValuePresetIds(values.map(String))} style={{ width: '100%' }}>
              <div className="feature-drawer-checkbox-grid">{AUDIO_VALUE_PRESETS.map((preset) => <Checkbox key={preset.id} value={preset.id}>{preset.label}</Checkbox>)}</div>
            </Checkbox.Group>
          </FeatureDrawerField>
          {audioValuePresetIds.length === 0 ? <Alert type="warning" showIcon message="至少选择一项声音预设" style={{ marginTop: 10 }} /> : null}
        </FeatureDrawerSection>

        {audioParameterControls ? (
          <FeatureDrawerSection
            title="声音来源与自动基线"
            description="保留原有声音来源配置能力，主页仅展示运行结果与只读参数。"
          >
            {audioParameterControls}
          </FeatureDrawerSection>
        ) : null}

        <FeatureDrawerSection title="音频参数状态" description="展示当前配置值及每个字段在实际处理链路中的生效状态。">
          <Descriptions column={{ xs: 1, sm: 1, md: 2, lg: 2, xl: 2, xxl: 2 }} size="small" bordered>
            {audioCapabilityRows.map((row) => <Descriptions.Item key={row.key} label={row.label}>{row.value === null ? '自动/未设置' : formatAudioPreviewValue(row.value, row.unit)} <Tag color={getProcessingStatusColor(row.status)}>{getProcessingStatusLabel(row.status)}</Tag></Descriptions.Item>)}
          </Descriptions>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="缓存管理" description="只删除未被当前播放或待切换任务引用的处理缓存。">
          <div className="feature-drawer-inline-feedback">
            <Popconfirm title="删除已生成缓存？" description="只删除未被当前或待切换引用的处理缓存。" onConfirm={() => void cleanupLocalCaches()}>
              <Button loading={cacheCleanupBusy}>删除已生成缓存</Button>
            </Popconfirm>
            {cacheCleanup ? <Tag>已删除 {cacheCleanup.removed_files} 个文件</Tag> : null}
          </div>
          {cacheCleanupError ? <Alert type="error" showIcon message={cacheCleanupError} style={{ marginTop: 8 }} /> : null}
        </FeatureDrawerSection>
      </FeatureDrawer>

      <Drawer rootStyle={DESKTOP_DRAWER_ROOT_STYLE} title={`当前插话预设${interludeRuntime?.preset_segment ? `（第 ${interludeRuntime.preset_segment} 段）` : ''}`} width={520} open={interludePresetDrawerOpen} onClose={() => setInterludePresetDrawerOpen(false)}>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          {activeInterludePresets.map((preset, index) => (
            <Card key={preset.id} size="small" title={`${index + 1}. ${preset.label}`} extra={<Tag>{preset.id}</Tag>}>
              <Descriptions column={1} size="small" bordered>{AUDIO_PRESET_FIELD_DEFINITIONS.map((field) => <Descriptions.Item key={field.key} label={field.label}>{formatAudioPresetFieldValue(field.key, preset.values[field.key], field.unit, field.digits)}</Descriptions.Item>)}</Descriptions>
            </Card>
          ))}
        </Space>
      </Drawer>

      <Drawer rootStyle={DESKTOP_DRAWER_ROOT_STYLE} title={`当前声音预设${audioVariationCycle > 0 ? `（第 ${audioVariationCycle} 轮）` : ''}`} width={520} open={audioPresetDrawerOpen} onClose={() => setAudioPresetDrawerOpen(false)}>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          {activeAudioPresets.map((preset) => (
            <Card key={preset.id} size="small" title={preset.label} extra={<Tag>{preset.id}</Tag>}>
              <Descriptions column={1} size="small" bordered>{AUDIO_PRESET_FIELD_DEFINITIONS.map((field) => <Descriptions.Item key={field.key} label={field.label}>{formatAudioPresetFieldValue(field.key, preset.values[field.key], field.unit, field.digits)}</Descriptions.Item>)}</Descriptions>
            </Card>
          ))}
        </Space>
      </Drawer>
    </>
  );
}

function DesktopRouter() {
  return (
    <Routes>
      <Route path="/" element={<DesktopApp />} />
      <Route path="/settings" element={<DesktopApp />} />
      <Route path="/status" element={<DesktopApp />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

export default function App() {
  return isFinalEffectWindow ? (
    <FinalEffectWindow />
  ) : (
    <HashRouter>
      <ControlPlaneGate>
        <DesktopRouter />
      </ControlPlaneGate>
    </HashRouter>
  );
}
