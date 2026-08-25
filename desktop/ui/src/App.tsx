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
import { App as AntApp, Alert, Button, Card, Checkbox, ConfigProvider, Descriptions, Drawer, Empty, Input, InputNumber, Layout, Popconfirm, Progress, Select, Slider, Space, Steps, Switch, Tag, Typography, theme as antdTheme } from 'antd';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { HashRouter, Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { buildInterludeScheduleKey, chooseInterludeIndex, interludeIntervalMsToSeconds, interludeIntervalSecondsToMs, INTERLUDE_LIMITS, nextInterludeAtMs, resolvePlaybackAudioSource, shouldPauseInterlude } from './interlude-player';
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
} from './runtime-parameter-scheduler';
import type { AudioCycleSample, PeriodRangeMs, RuntimeBaseParameters, RuntimePreviewParameters, SubtleAudioSample } from './runtime-parameter-scheduler';
import { appendAudioCycleSnapshot } from './audioCycleSnapshot';
import {
  createAudioCycleCandidatePlan,
  getAudioCycleCoordinatorAction,
  isAudioCycleCommandMessage,
  isExpectedAudioCycleSkipCode,
  isAudioCycleResultMessage,
  updateAudioCycleCandidateStatus,
} from './audio-cycle-prewarm-coordinator';
import type {
  AudioCycleCandidatePlan,
  AudioCycleCommandMessage,
  AudioCycleResultMessage,
} from './audio-cycle-prewarm-coordinator';
import {
  advanceMediaCycleQueue,
  createMediaCycleQueue,
  createIndependentMediaCycleQueues,
  createLinkedMediaCycleQueues,
  intersectPeriodRanges,
} from './media-cycle-planner';
import type { MediaCyclePlan, MediaCycleQueue, MediaCycleSeed } from './media-cycle-planner';
import { AudioParameterControls, MediaParameterPanels, type MediaEffectParams } from './media-parameter-panels';
import { sampleAutomaticVideoParameters } from './video-parameter-randomizer';
import { buildAudioCapabilityRows, resolveAudioStreamBranches } from './audio-processing-capabilities';
import {
  classifyAudioOutputSync,
  clearResolvedPortAudioSyncError,
  getPortAudioSourceRetryMode,
  isExpectedAudioOutputSyncCancellation,
  isRetryableAudioOutputSyncCode,
  resolvePortAudioSourcePath,
  shouldKeepPortAudioCycleScheduling,
} from './audio-output-recovery';
import { resolveSynchronizedVideoPlaybackRate } from './audio-playback-sync';
import { shouldIgnoreLoopBoundaryPause, shouldRestartCurrentSourceImmediately, shouldRestartPlayback } from './playback-loop';
import {
  buildFinalEffectMediaIdentity,
  buildFinalEffectWindowResizeKey,
  isFinalEffectMediaIdentityCurrent,
} from './final-effect-window-size';
import { capturePlaybackPosition, clampMediaTime, clampVolume, createAudioSyncClock, formatMediaTime, isPlaybackMediaControlMessage, isPlaybackMediaStateMessage, resolvePlaybackResumePosition, shouldApplyPlaybackSeek, shouldIssuePlaybackCommand } from './playback-control-message';
import type { AudioSyncClock, PlaybackMediaControlMessage, PlaybackMediaStateMessage, PlaybackPositionCheckpoint } from './playback-control-message';
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
import { CompactNumberField } from './desktop/compact-number-field';
import { ControlPlaneGate } from './desktop/control-plane-gate';
import { DesktopPanel } from './desktop/desktop-panel';
import { FeatureDrawer, FeatureDrawerField, FeatureDrawerSection } from './desktop/feature-drawer';
import { DesktopColumn, DesktopShell } from './desktop/desktop-shell';
import { PlaybackPoolPanel, type PlaybackPoolSource } from './desktop/playback-pool-panel';
import { DesktopStatusStrip } from './desktop/status-strip';
import type { DesktopStatusItem } from './desktop/status-strip';
import './desktop-layout.css';

const PLAYBACK_CHANNEL_NAME = 'autolive-playback-ui-v1';
const FIXED_SPEECH_ACK_TIMEOUT_MS = 3_000;
const RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500;
const PLAYBACK_SNAPSHOT_POLL_MS = 1_000;
const AUDIO_OUTPUT_STATUS_POLL_MS = 2_000;
const AUDIO_VIDEO_REALIGN_THRESHOLD_MS = 80;
const AUDIO_VIDEO_REALIGN_CONSECUTIVE_POLLS = 3;
const AUDIO_VIDEO_REALIGN_COOLDOWN_MS = 10_000;
const DIAGNOSTIC_PUBLISH_INTERVAL_MS = 50;
const DIAGNOSTIC_SAMPLE_COUNT = 128;
const DIAGNOSTIC_LINE_SAMPLE_COUNT = 96;
const REALTIME_AUDIO_SAFETY_LEAD_MS = 6_000;
const REALTIME_AUDIO_WORKER_TIMEOUT_MS = 5_000;
const PORTAUDIO_FORMAL_SOURCE_SYNC_READY = true;
const AUTO_PORTAUDIO_ENABLED = true;
const AUTO_PORTAUDIO_RETRY_COOLDOWN_MS = 5_000;
const AUDIO_CYCLE_RECOVERY_RETRY_MS = 500;
const PORTAUDIO_SAMPLE_RATE_HZ = 44_100;
const PORTAUDIO_MIN_MEMORY_BUFFER_KIB = 128;
const PORTAUDIO_MAX_MEMORY_BUFFER_KIB = 2_048;
const PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB = 1_024;
const PORTAUDIO_DEFAULT_FRAMES_PER_BUFFER = 256;
const SUPPORTED_VIDEO_EXTENSIONS = [
  'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'ts', 'm2ts', 'flv', 'wmv', '3gp',
] as const;

type DiagnosticSource = 'portaudio-mixed-pcm' | 'web-audio-analyser';

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

type DiagnosticMessage = {
  version: 1;
  type: 'diagnostic';
  source: DiagnosticSource;
  sequence: number | null;
  has_pcm: boolean;
  sample_rate_hz: number | null;
  captured_frame_count: number | null;
  line: number[];
  rms_dbfs: number | null;
  peak_dbfs: number | null;
  low_band_rms_dbfs: number | null;
  cutoff_hz: number | null;
  mfcc: number[];
  mfcc_available: boolean;
  noise_floor_dbfs: number | null;
  snr_db: number | null;
  formants_hz: Array<number | null>;
  current_formant_hz: number | null;
  waveform?: number[];
  spectrum?: number[];
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
  audio_stream_params?: MediaEffectParams['audio'] | null;
  audio_stream_variants?: MediaEffectParams['audio'][];
  audio_stream_revision: number;
  audio_processing_status: string;
  audio_processing_runtime: boolean;
  audio_processing_gain_db: number;
  interlude?: InterludeSnapshot | null;
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
  audioVariationMode: InterludeAudioVariationMode;
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

type ReleaseWebViewInterludeCacheResult = {
  released: boolean;
  removed: boolean;
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

function audioCommitLeadPlaybackMs(status: AudioOutputBackendStatus | null): number {
  if (!status) return 0;
  const sampleRateHz = status.actual_sample_rate_hz ?? status.sample_rate_hz;
  const channels = Math.max(1, status.channels);
  const ringMs = Number.isFinite(sampleRateHz) && sampleRateHz > 0
    ? status.ring_len_samples / channels / sampleRateHz * 1_000
    : 0;
  return Math.max(ringMs, status.playback_watermark_ms) + Math.max(0, status.output_latency_ms);
}

type AudioOutputDevice = {
  id: string;
  name: string;
  host_api: string;
  max_output_channels: number;
  default_sample_rate_hz: number;
};

type PrepareAudioCycleCandidateResult = {
  candidate_id: number;
  state: 'preparing' | 'ready';
  target_absolute_position_ms: number;
  buffered_ms: number;
  ffmpeg_pid: number | null;
};

type CommitAudioCycleCandidateResult = {
  candidate_id: number;
  committed: boolean;
  reason: string | null;
  reason_code: string | null;
  snapshot: PlaybackSnapshot;
};

type PlannedAudioCyclePayload = {
  sample: AudioCycleSample;
  audio: MediaEffectParams['audio'];
  audioVariants: MediaEffectParams['audio'][];
  periodMs: number;
};

type PendingAudioCyclePayload = PlannedAudioCyclePayload & {
  playbackGeneration: number;
  baseAudioStreamRevision: number;
  planId: string;
  sequence: number;
};

type PlannedVideoCyclePayload = {
  seed: number;
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
  const validSamples = (samples: unknown, maxLength = DIAGNOSTIC_SAMPLE_COUNT): samples is number[] =>
    Array.isArray(samples)
    && samples.length <= maxLength
    && samples.every((sample) => typeof sample === 'number' && Number.isFinite(sample) && sample >= -1 && sample <= 1);
  const nullableFiniteNumber = (sample: unknown): sample is number | null =>
    sample === null || (typeof sample === 'number' && Number.isFinite(sample));
  return (
    record.version === 1 &&
    record.type === 'diagnostic' &&
    (record.source === 'portaudio-mixed-pcm' || record.source === 'web-audio-analyser') &&
    (record.sequence === null || (typeof record.sequence === 'number' && Number.isSafeInteger(record.sequence))) &&
    typeof record.has_pcm === 'boolean' &&
    nullableFiniteNumber(record.sample_rate_hz) &&
    nullableFiniteNumber(record.captured_frame_count) &&
    nullableFiniteNumber(record.rms_dbfs) &&
    nullableFiniteNumber(record.peak_dbfs) &&
    nullableFiniteNumber(record.low_band_rms_dbfs) &&
    nullableFiniteNumber(record.cutoff_hz) &&
    nullableFiniteNumber(record.noise_floor_dbfs) &&
    nullableFiniteNumber(record.snr_db) &&
    nullableFiniteNumber(record.current_formant_hz) &&
    typeof record.mfcc_available === 'boolean' &&
    Array.isArray(record.mfcc) &&
    record.mfcc.length <= 40 &&
    record.mfcc.every((coefficient) => typeof coefficient === 'number' && Number.isFinite(coefficient)) &&
    Array.isArray(record.formants_hz) &&
    record.formants_hz.length === 3 &&
    record.formants_hz.every(nullableFiniteNumber) &&
    typeof record.sent_at_ms === 'number' &&
    Number.isFinite(record.sent_at_ms) &&
    validSamples(record.line, DIAGNOSTIC_LINE_SAMPLE_COUNT) &&
    (record.waveform === undefined || validSamples(record.waveform)) &&
    (record.spectrum === undefined || validSamples(record.spectrum)) &&
    (record.error === null || typeof record.error === 'string')
  );
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

function isTransientAudioCandidateCode(code: string | null | undefined): boolean {
  return code === 'audio_candidate_target_invalid'
    || code === 'audio_candidate_stale'
    || code === 'audio_mixer_candidate_stale'
    || code === 'audio_mixer_candidate_superseded';
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

function playbackVideoUrl(snapshot: PlaybackSnapshot | null | undefined): string | null {
  const useProcessedVideo = snapshot?.current_video_source === 'processed'
    && Boolean(snapshot.current_video_reference);
  return toAssetUrl(
    useProcessedVideo
      ? snapshot.current_video_reference
      : (snapshot?.source_media?.source_path ?? null),
  );
}

const INTERLUDE_AUDIO_PRESET_IDS = new Set(AUDIO_VALUE_PRESETS.map((preset) => preset.id));

function normalizeInterludeAudioPresetIds(values?: readonly string[] | null): string[] {
  const presetIds = [...new Set(values ?? [])].filter((id) => INTERLUDE_AUDIO_PRESET_IDS.has(id));
  return presetIds.length > 0 ? presetIds : [...DEFAULT_AUDIO_VALUE_PRESET_IDS];
}

function buildInterludeDraft(interlude?: InterludeSnapshot | null): InterludeConfigDraft {
  const audioMixPickMax = normalizeAudioMixPickMax(
    interlude?.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
  );
  const audioVariationPeriod = normalizeAudioPeriodRange(
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
    audioVariationMode: interlude?.audio_variation_mode === 'periodic' ? 'periodic' : 'each_playback',
    audioVariationPeriodMinMs: audioVariationPeriod.minMs,
    audioVariationPeriodMaxMs: audioVariationPeriod.maxMs,
    intervalMinMs: interlude?.interval_min_ms ?? 8_000,
    intervalMaxMs: interlude?.interval_max_ms ?? 13_000,
    volumeDb: interlude?.volume_db ?? 0,
    duckingDepthDb: interlude?.ducking_depth_db ?? -12,
    duckingAttackMs: interlude?.ducking_attack_ms ?? 50,
    duckingReleaseMs: interlude?.ducking_release_ms ?? 250,
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
  const audioSourceSyncRef = useRef({
    latestRequest: 0,
    pending: false,
    running: false,
    recoverUnhealthy: false,
    reanchorLoopBoundary: false,
  });
  const committedAudioCycleRevisionRef = useRef<number | null>(null);
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
  const clockEpochRef = useRef(Date.now());
  const clockSequenceRef = useRef(0);
  const clockIdentityRef = useRef<string | null>(null);
  const sourceRevisionRef = useRef(0);
  const suppressClockDiscontinuityRef = useRef(false);
  const lastRustPositionSyncAtRef = useRef(0);
  const loopSyncPromiseRef = useRef<Promise<void> | null>(null);
  const lastRestartTokenRef = useRef<string | number | null>(null);
  // 续播点只属于已经完成 metadata 加载的播放代次，旧 video 的迟到事件不得污染下一项。
  const playbackPositionRef = useRef<PlaybackPositionCheckpoint | null>(null);
  const loadedPlaybackGenerationRef = useRef<number | null>(null);
  const [processedAudioUrl, setProcessedAudioUrl] = useState<string | null>(null);
  const [processedAudioReady, setProcessedAudioReady] = useState(false);
  const interludeScheduleKeyRef = useRef<string | null>(null);
  const nextInterludeAtMsRef = useRef<number | null>(null);
  const lastInterludeIndexRef = useRef<number | null>(null);
  const interludeAudioCycleRef = useRef<AudioCycleSample | null>(null);
  const interludeAudioVariationConfigKeyRef = useRef<string | null>(null);
  const nextInterludeAudioVariationAtMsRef = useRef<number | null>(null);
  const lastInterludeAudioPresetIdsRef = useRef<string[]>([]);
  const interludeActiveRef = useRef(false);
  const interludePausedRef = useRef(false);
  const interludeStopTimerRef = useRef<number | null>(null);
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [fixedSpeechActive, setFixedSpeechActive] = useState(false);
  const [interludeAudioUrl, setInterludeAudioUrl] = useState<string | null>(null);
  const nextSegmentStartRef = useRef<number | null>(null);
  const scheduleGenerationRef = useRef<number | null>(null);
  const scheduleLoopRef = useRef<number | null>(null);
  const snapshotRef = useRef<PlaybackSnapshot | null>(null);
  const finalEffectResizeKeyRef = useRef<string | null>(null);
  const finalEffectResizeQueueRef = useRef<Promise<void>>(Promise.resolve());
  const workerAvailableRef = useRef(false);
  const [audioDiagnosticsReady, setAudioDiagnosticsReady] = useState(false);
  const [portAudioHardwareEnabled, setPortAudioHardwareEnabled] = useState(false);
  const [realtimeAudioPlaying, setRealtimeAudioPlaying] = useState(false);
  const [userMuted, setUserMuted] = useState(false);
  const [userVolume, setUserVolume] = useState(1);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [finalEffectResizeError, setFinalEffectResizeError] = useState<string | null>(null);
  const portAudioSourcePath = resolvePortAudioSourcePath(snapshot);
  const finalEffectMediaIdentity = buildFinalEffectMediaIdentity({
    playbackGeneration: snapshot?.playback_generation,
    sourceIdentity: sourceUrl,
  });

  function isCurrentFinalEffectVideo(
    video: HTMLVideoElement,
    expectedIdentity: string | null,
    allowUnselectedCurrentSource = false,
  ): boolean {
    const currentSnapshot = snapshotRef.current;
    const currentSourceUrl = playbackVideoUrl(currentSnapshot);
    return isFinalEffectMediaIdentityCurrent(expectedIdentity, {
      playbackGeneration: currentSnapshot?.playback_generation,
      sourceIdentity: currentSourceUrl,
    })
      && currentSourceUrl !== null
      && video.getAttribute('src') === currentSourceUrl
      && (
        video.currentSrc === currentSourceUrl
        || (allowUnselectedCurrentSource && video.currentSrc.length === 0)
      );
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
      // WebView 模式由媒体元素出声；PortAudio 模式只把它当结束时钟，避免双播。
      interludeAudio.volume = clampVolume(volume * interludeGainLevelRef.current);
      interludeAudio.muted = interludePortAudioRef.current || muted;
    }
    applyRealtimeVideoFx(
      runtimeAudioParamsRef.current,
      runtimeAudioEnabledRef.current && !replacementAudioActive && !processedActive && !(muted && !hardwareOut),
    );
  }

  function setPortAudioHardwareActive(requested: boolean) {
    const nextActive = requested && PORTAUDIO_FORMAL_SOURCE_SYNC_READY;
    const previousActive = portAudioHardwareRef.current;
    if (portAudioHardwareRef.current === nextActive) {
      syncUserAudioSettings();
      return;
    }
    if (nextActive && !previousActive && interludeActiveRef.current && !interludePortAudioRef.current) {
      clearInterludePlayback({ releaseMs: 0, resetSchedule: false });
      nextInterludeAtMsRef.current = performance.now();
    }
    if (!nextActive && previousActive && interludePortAudioRef.current) {
      interludePortAudioRef.current = false;
    }
    portAudioHardwareRef.current = nextActive;
    setPortAudioHardwareEnabled(nextActive);
    syncUserAudioSettings();
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
    void invoke<ReleaseWebViewInterludeCacheResult>('release_webview_interlude_cache', {
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
    duckGainLevelRef.current = 1;
    nextInterludeAtMsRef.current = null;
    releaseCurrentWebViewInterludeCache();
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
    duckGainLevelRef.current = 1;
    syncUserAudioSettings();
    if (shouldStopPortAudio) {
      void invoke<void>('stop_portaudio_interlude').catch(() => undefined);
    }
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
    clearInterludePlayback({
      releaseMs: interludeReleaseMsRef.current,
      resetSchedule: true,
    });
  }

  function handleInterludeError(event: SyntheticEvent<HTMLAudioElement>) {
    if (!interludeActiveRef.current) return;
    const detail = event.currentTarget.error?.message;
    setPlaybackError(detail ? `插话音频播放失败：${detail}` : '插话音频播放失败，请检查文件格式和文件权限。');
    clearInterludePlayback({ resetSchedule: true });
  }

  function resolveInterludeAudioCycle(
    interlude: InterludeSnapshot,
    currentClockMs: number,
  ): AudioCycleSample {
    const selectionMode = interlude.audio_selection_mode === 'fixed' ? 'fixed' : 'random';
    const fixedPresetId = normalizeInterludeAudioPresetIds([
      interlude.audio_fixed_preset_id ?? 'p01',
    ])[0];
    const presetIds = normalizeInterludeAudioPresetIds(interlude.audio_preset_ids);
    const configKey = JSON.stringify([
      selectionMode,
      fixedPresetId,
      presetIds,
      interlude.audio_mix_enabled ?? false,
      interlude.audio_mix_pick_min ?? DEFAULT_AUDIO_MIX_PICK_MIN,
      interlude.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
      interlude.audio_variation_mode ?? 'each_playback',
      interlude.audio_variation_period_min_ms ?? 8_000,
      interlude.audio_variation_period_max_ms ?? 15_000,
    ]);
    if (configKey !== interludeAudioVariationConfigKeyRef.current) {
      interludeAudioVariationConfigKeyRef.current = configKey;
      interludeAudioCycleRef.current = null;
      nextInterludeAudioVariationAtMsRef.current = null;
    }

    if (selectionMode === 'fixed') {
      const sample = sampleAudioCycle([fixedPresetId], {
        mixEnabled: false,
        pickMin: 1,
        pickMax: 1,
      });
      interludeAudioCycleRef.current = sample;
      lastInterludeAudioPresetIdsRef.current = [...sample.presetIds];
      nextInterludeAudioVariationAtMsRef.current = null;
      return sample;
    }

    const periodic = interlude.audio_variation_mode === 'periodic';
    const periodExpired = nextInterludeAudioVariationAtMsRef.current !== null
      && currentClockMs >= nextInterludeAudioVariationAtMsRef.current;
    if (!periodic || interludeAudioCycleRef.current === null || periodExpired) {
      const sample = sampleAudioCycle(presetIds, {
        mixEnabled: interlude.audio_mix_enabled ?? false,
        pickMin: interlude.audio_mix_pick_min ?? DEFAULT_AUDIO_MIX_PICK_MIN,
        pickMax: interlude.audio_mix_pick_max ?? DEFAULT_AUDIO_MIX_PICK_MAX,
        previousPresetIds: lastInterludeAudioPresetIdsRef.current,
      });
      interludeAudioCycleRef.current = sample;
      lastInterludeAudioPresetIdsRef.current = [...sample.presetIds];
      const period = normalizeAudioPeriodRange(
        interlude.audio_variation_period_min_ms ?? 8_000,
        interlude.audio_variation_period_max_ms ?? 15_000,
      );
      nextInterludeAudioVariationAtMsRef.current = periodic
        ? currentClockMs + samplePeriodMsInRange(period)
        : null;
    }
    return interludeAudioCycleRef.current;
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
    const audioCycle = resolveInterludeAudioCycle(interlude, performance.now());
    const audioPeriod = normalizeAudioPeriodRange(
      interlude.audio_variation_period_min_ms ?? 8_000,
      interlude.audio_variation_period_max_ms ?? 15_000,
    );
    const audioPeriodMs = samplePeriodMsInRange(audioPeriod);
    const audio = {
      ...audioCycle.values,
      random_change_period_ms: audioPeriodMs,
      current_formant_hz: null,
    };
    const audioVariants = interlude.audio_selection_mode !== 'fixed'
      && interlude.audio_mix_enabled
      && audioCycle.variants.length > 1
      ? buildAudioVariantsFromCycle(audio, audioCycle)
      : [];
    const usePortAudio = portAudioHardwareRef.current;
    let playbackPath = selectedPath;
    let processedPath: string | null = null;
    if (usePortAudio) {
      try {
        await invoke<void>('start_portaudio_interlude', {
          request: {
            path: selectedPath,
            audio,
            audio_variants: audioVariants,
          },
        });
      } catch (cause) {
        if (operation === interludeOperationRef.current) {
          interludeStartingRef.current = false;
          setPlaybackError(
            `${getDisplayErrorMessage(cause, 'PortAudio 插话解码或混音失败')}；主音轨保持播放。`,
          );
        }
        return;
      }
    } else {
      try {
        const prepared = await invoke<PrepareWebViewInterludeResult>('prepare_webview_interlude', {
          request: {
            path: selectedPath,
            audio,
            audio_variants: audioVariants,
          },
        });
        if (prepared.state === 'processed') {
          playbackPath = prepared.path;
          processedPath = prepared.path;
        } else if (prepared.state === 'original_fallback') {
          playbackPath = prepared.path;
          if (prepared.reason) setPlaybackError(`${prepared.reason}；已回退插话原声。`);
        }
      } catch (cause) {
        if (operation === interludeOperationRef.current) {
          setPlaybackError(`${getDisplayErrorMessage(cause, 'WebView 插话本地处理失败')}；已回退插话原声。`);
        }
      }
    }
    const selectedUrl = toAssetUrl(playbackPath);
    if (!selectedUrl) {
      if (processedPath) releaseWebViewInterludeCache(processedPath);
      interludeStartingRef.current = false;
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
    interludeAudioUrlRef.current = selectedUrl;
    setInterludeAudioUrl(selectedUrl);
    syncUserAudioSettings();
  }

  function publishMediaState() {
    const channel = playbackChannelRef.current;
    const video = videoRef.current;
    if (!channel || !video) return;
    const currentSnapshot = snapshotRef.current;
    const duration = Number.isFinite(video.duration) && video.duration >= 0 ? video.duration : 0;
    const currentTime = clampMediaTime(video.currentTime, duration);
    const probedDurationMs = currentSnapshot?.source_media?.duration_ms;
    const durationMs = typeof probedDurationMs === 'number' && probedDurationMs > 0
      ? Math.round(probedDurationMs)
      : Math.max(0, Math.round(duration * 1_000));
    const positionMs = Math.min(durationMs, Math.max(0, Math.round(currentTime * 1_000)));
    const playbackGeneration = currentSnapshot?.playback_generation ?? 0;
    const identity = `${playbackGeneration}:${currentSnapshot?.current_video_reference ?? sourceUrl ?? ''}`;
    if (clockIdentityRef.current !== identity) {
      clockIdentityRef.current = identity;
      clockEpochRef.current += 1;
      clockSequenceRef.current = 0;
      sourceRevisionRef.current += 1;
    }
    loopSequenceRef.current = Math.max(loopSequenceRef.current, currentSnapshot?.loop_index ?? 0);
    clockSequenceRef.current += 1;
    const absolutePositionMs = loopSequenceRef.current * durationMs + positionMs;
    if (!Number.isSafeInteger(absolutePositionMs)) return;
    const nowMs = Date.now();
    if (nowMs - lastRustPositionSyncAtRef.current >= 500) {
      lastRustPositionSyncAtRef.current = nowMs;
      void invoke<PlaybackSnapshot>('update_playback_position', {
        request: { position_ms: positionMs },
      }).catch(() => undefined);
    }
    try {
      channel.postMessage({
        version: 2,
        type: 'playback-media-state',
        current_time: currentTime,
        duration,
        volume: clampVolume(video.volume),
        muted: userMutedRef.current,
        paused: video.paused,
        playback_generation: playbackGeneration,
        source_revision: sourceRevisionRef.current,
        clock_epoch: clockEpochRef.current,
        clock_sequence: clockSequenceRef.current,
        loop_index: loopSequenceRef.current,
        position_ms: positionMs,
        duration_ms: durationMs,
        absolute_position_ms: absolutePositionMs,
        playback_rate: Number.isFinite(video.playbackRate) && video.playbackRate > 0 ? video.playbackRate : 1,
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
      suppressClockDiscontinuityRef.current = true;
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

  function publishAudioCycleResult(message: AudioCycleResultMessage) {
    try {
      playbackChannelRef.current?.postMessage(message);
    } catch {
      // 最终效果窗关闭时，Rust 生命周期命令仍由停止/暂停路径兜底回收。
    }
  }

  async function handleAudioCycleCommand(message: AudioCycleCommandMessage) {
    const video = videoRef.current;
    try {
      if (message.action === 'cancel') {
        const result = await invoke<{ candidate_id: number | null; cancelled: boolean }>(
          'cancel_audio_cycle_candidate',
          { request: { candidate_id: message.candidate_id } },
        );
        publishAudioCycleResult({
          version: 1,
          type: 'audio-cycle-result',
          action: 'cancel',
          candidate_id: message.candidate_id,
          accepted: result.cancelled,
          committed: false,
          reason: null,
        });
        return;
      }
      await loopSyncPromiseRef.current;
      const currentSnapshot = snapshotRef.current;
      if (!currentSnapshot || !video) throw new Error('最终效果窗口尚未建立视频时钟');

      if (message.action === 'prepare') {
        if (
          typeof message.target_absolute_position_ms !== 'number'
          || !Number.isSafeInteger(message.target_absolute_position_ms)
          || message.target_absolute_position_ms < 0
          || typeof message.base_audio_stream_revision !== 'number'
          || !message.audio
        ) {
          throw new Error('候选音轨准备参数不完整');
        }
        const result = await invoke<PrepareAudioCycleCandidateResult>('prepare_audio_cycle_candidate', {
          request: {
            candidate_id: message.candidate_id,
            audio: message.audio,
            audio_variants: message.audio_variants ?? [],
            playback_generation: message.playback_generation,
            base_audio_stream_revision: message.base_audio_stream_revision,
            target_absolute_position_ms: message.target_absolute_position_ms,
          },
        });
        publishAudioCycleResult({
          version: 1,
          type: 'audio-cycle-result',
          action: 'prepare',
          candidate_id: message.candidate_id,
          accepted: result.candidate_id === message.candidate_id,
          committed: false,
          reason: null,
        });
        return;
      }

      const syncClock = resolveAudioSyncClock(
        currentSnapshot,
        null,
        video,
        loopSequenceRef.current,
      );
      if (!syncClock) throw new Error('最终效果窗口尚未建立绝对媒体时钟');
      const result = await invoke<CommitAudioCycleCandidateResult>('commit_audio_cycle_candidate', {
        request: {
          candidate_id: message.candidate_id,
          ...syncClock,
        },
      });
      if (result.committed) {
        committedAudioCycleRevisionRef.current = result.snapshot.audio_stream_revision;
        applyPlayerSnapshot(result.snapshot);
      }
      publishAudioCycleResult({
        version: 1,
        type: 'audio-cycle-result',
        action: 'commit',
        candidate_id: message.candidate_id,
        accepted: true,
        committed: result.committed,
        reason: result.reason,
        error_code: result.reason_code,
        snapshot: result.snapshot,
      });
    } catch (cause) {
      publishAudioCycleResult({
        version: 1,
        type: 'audio-cycle-result',
        action: message.action,
        candidate_id: message.candidate_id,
        accepted: false,
        committed: false,
        reason: getDisplayErrorMessage(cause, '候选音轨操作失败'),
        error_code: getCommandErrorCode(cause),
      });
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
      if (isAudioCycleCommandMessage(event.data)) {
        void handleAudioCycleCommand(event.data);
        return;
      }
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
      shutdownInterludePlayback();
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
      shutdownInterludePlayback();
      window.removeEventListener('pagehide', handlePageHide);
      channel.removeEventListener('message', handleMessage);
      channel.close();
      if (playbackChannelRef.current === channel) playbackChannelRef.current = null;
    };
  }, []);

  useEffect(() => {
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
      void invoke<AudioCycleDiagnosticSnapshot>('get_audio_cycle_diagnostic')
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
  }, [finalEffectResizeError, playbackError]);

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
    let avDriftPollCount = 0;
    let avDriftCooldownUntilMs = 0;
    let sourceRetryAttemptedAtMs = 0;
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
              const next = await invoke<AudioOutputBackendStatus>('set_audio_output_backend', {
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
            const status = await invoke('sync_audio_output_source', {
              request: { ...syncClock, recover_unhealthy, reanchor_loop_boundary },
            }) as AudioOutputBackendStatus;
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
    if (committedAudioCycleRevisionRef.current === snapshot?.audio_stream_revision) {
      committedAudioCycleRevisionRef.current = null;
      return;
    }
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
    void invoke<SpeechToSpeechWorkerCapabilities>('get_speech_to_speech_worker_capabilities')
      .then(setWorkerCapabilities)
      .catch(() => setWorkerCapabilities(null));
  }, []);

  useEffect(() => {
    // 普通声音已改为独立流式出口，视频缓存可以安全地只承担画面处理；
    // PortAudio 会从源视频取音频，WebView 回退则使用缓存中的源 AAC。
    const nextUrl = playbackVideoUrl(snapshot);
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
        video.currentTime = Math.min(resumeAt, safeEnd);
        playbackPositionRef.current = {
          playbackGeneration: expectedPlaybackGeneration,
          positionSec: video.currentTime,
        };
      }
      void audioContextRef.current?.resume().catch(() => undefined);
      userMutedRef.current = false;
      syncUserAudioSettings();
      if (snapshotRef.current?.playback_state === 'playing') {
        suppressMediaEventRef.current = true;
        void video.play().catch(() => {
          if (!isCurrentFinalEffectVideo(video, expectedIdentity)) return;
          suppressMediaEventRef.current = false;
          setPlaybackError('处理结果已切换，但自动播放被拦截，请点击视频恢复声音。');
        });
      }
    };
    video.addEventListener('loadedmetadata', restore);
    if (video.readyState >= 1) restore();
    return () => {
      cancelled = true;
      video.removeEventListener('loadedmetadata', restore);
    };
  }, [finalEffectMediaIdentity, snapshot?.playback_generation, sourceUrl]);

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
      const t = video && Number.isFinite(video.currentTime) ? video.currentTime : 0;
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
    if (!video || !sourceUrl) return;
    if (snapshot?.playback_state === 'stopped') {
      video.pause();
      playbackPositionRef.current = null;
      loadedPlaybackGenerationRef.current = null;
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
      clearInterludePlayback({ resetSchedule: true, resetIndex: true });
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
      'ratechange',
      'seeked',
      'ended',
      'enterpictureinpicture',
      'leavepictureinpicture',
    ];
    const handleSeeking = () => {
      if (suppressClockDiscontinuityRef.current) {
        suppressClockDiscontinuityRef.current = false;
        return;
      }
      clockEpochRef.current += 1;
      clockSequenceRef.current = 0;
    };
    mediaEvents.forEach((eventName) => video.addEventListener(eventName, publishMediaState));
    video.addEventListener('seeking', handleSeeking);
    publishMediaState();
    const timer = window.setInterval(publishMediaState, 250);
    return () => {
      window.clearInterval(timer);
      video.removeEventListener('seeking', handleSeeking);
      mediaEvents.forEach((eventName) => video.removeEventListener(eventName, publishMediaState));
    };
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
        lastInterludeIndexRef.current = null;
        interludeAudioCycleRef.current = null;
        interludeAudioVariationConfigKeyRef.current = null;
        nextInterludeAudioVariationAtMsRef.current = null;
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
        }
        return;
      }

      if (interludeActiveRef.current || interludeStartingRef.current) return;

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
      void startInterludePlayback(interlude);
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

  function restartCurrentPlayback(video: HTMLVideoElement, autoplayError: string) {
    // 自然结束会先触发 pause；必须先占住事件，避免误暂停 PortAudio 和候选。
    suppressMediaEventRef.current = true;
    suppressClockDiscontinuityRef.current = true;
    video.currentTime = 0;
    if (audioRef.current) audioRef.current.currentTime = 0;
    const processed = processedAudioPlayingRef.current
      ? (processedActiveSlotRef.current === 0 ? processedAudioARef.current : processedAudioBRef.current)
      : null;
    if (processed) processed.currentTime = 0;
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
      restartCurrentPlayback(video, '视频已回到开头，但自动播放失败，请点击视频播放。');
    } else {
      const operationId = fixedSpeechOperationRef.current?.operationId;
      if (operationId) cancelFixedSpeech(operationId);
      clearInterludePlayback({ resetSchedule: true, resetIndex: true });
    }

    const loopSyncPromise = invoke<PlaybackItemCompletionResult>('complete_playback_item', {
      request: {
        playback_generation: currentSnapshot.playback_generation,
        loop_index: currentSnapshot.loop_index,
        source_media_index: currentSnapshot.source_media_index,
      },
    })
      .then((result) => {
        const nextSnapshot = result.snapshot;
        if (result.source_changed && restartImmediately) {
          const operationId = fixedSpeechOperationRef.current?.operationId;
          if (operationId) cancelFixedSpeech(operationId);
          clearInterludePlayback({ resetSchedule: true, resetIndex: true });
        }
        loopSequenceRef.current = nextSnapshot.loop_index;
        applyPlayerSnapshot(nextSnapshot);
        if (!result.source_changed && !restartImmediately) {
          restartCurrentPlayback(video, '视频已回到开头，但自动播放失败，请点击视频播放。');
        }
        if (portAudioHardwareRef.current) syncAudioOutputSourceLatest(false, true);
      })
      .catch((cause) => {
        lastRestartTokenRef.current = null;
        loopSequenceRef.current = currentSnapshot.loop_index;
        if (!restartImmediately) {
          restartCurrentPlayback(video, '当前视频重新播放失败，请点击视频恢复播放。');
        }
        setPlaybackError(getDisplayErrorMessage(cause, '播放项切换失败，已重播当前视频。'));
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
    const duration = Number.isFinite(video.duration) ? video.duration : 0;
    const restartToken = `${currentSourceKey}:${currentSnapshot.playback_generation}:${loopSequenceRef.current}`;
    if (
      !shouldRestartPlayback({
        restartToken,
        lastRestartToken: lastRestartTokenRef.current,
        syncInFlight: loopSyncPromiseRef.current !== null,
        ended: event.type === 'ended',
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
                onPlay={(event) => {
                  if (!isCurrentFinalEffectVideo(event.currentTarget, finalEffectMediaIdentity)) return;
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
                onPause={(event) => {
                  if (!isCurrentFinalEffectVideo(event.currentTarget, finalEffectMediaIdentity)) return;
                  if (shouldIgnoreLoopBoundaryPause({
                    suppressMediaEvent: suppressMediaEventRef.current,
                    ended: event.currentTarget.ended,
                    currentTime: event.currentTarget.currentTime,
                    duration: event.currentTarget.duration,
                  })) {
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
                  position: 'absolute',
                  inset: 0,
                  width: '100%',
                  height: '100%',
                  maxWidth: 'none',
                  maxHeight: 'none',
                  objectFit: 'contain',
                  display: 'block',
                  background: '#000',
                }}
                onError={(event) => {
                  if (isCurrentFinalEffectVideo(event.currentTarget, finalEffectMediaIdentity, true)) {
                    setPlaybackError('独立播放器加载视频失败，请回到主页重新导入。');
                  }
                }}
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
                muted={userMuted || portAudioHardwareEnabled}
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
  const [interludeDraft, setInterludeDraft] = useState<InterludeConfigDraft>(() => buildInterludeDraft());
  const [interludeDirty, setInterludeDirty] = useState(false);
  const [interludeSaving, setInterludeSaving] = useState(false);
  const [interludeSaveError, setInterludeSaveError] = useState<string | null>(null);
  const [mediaEngineCapabilities, setMediaEngineCapabilities] = useState<MediaEngineCapabilities | null>(null);
  const [audioOutputBackend, setAudioOutputBackend] = useState<AudioOutputBackendStatus | null>(null);
  const audioOutputBackendRef = useRef<AudioOutputBackendStatus | null>(null);
  const [audioOutputDevices, setAudioOutputDevices] = useState<AudioOutputDevice[]>([]);
  const [audioOutputBusy, setAudioOutputBusy] = useState(false);
  const [audioOutputDeviceId, setAudioOutputDeviceId] = useState<string | null>(null);
  const [audioOutputMemoryKib, setAudioOutputMemoryKib] = useState(PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB);
  const [audioOutputMemoryKibInput, setAudioOutputMemoryKibInput] = useState<number | null>(
    PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB,
  );
  const [audioOutputHostApiFilter, setAudioOutputHostApiFilter] = useState<string>('all');
  const hasAsioOutputDevice = audioOutputDevices.some(
    (device) => device.host_api.trim().toLowerCase() === 'asio',
  );
  const autoPortAudioAttemptRef = useRef({ key: null as string | null, startedAtMs: 0, inFlight: false });
  const autoPortAudioRetryTimerRef = useRef<number | null>(null);
  const [autoPortAudioRetryRevision, setAutoPortAudioRetryRevision] = useState(0);

  useLayoutEffect(() => {
    audioOutputBackendRef.current = audioOutputBackend;
  }, [audioOutputBackend]);

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
  ): Promise<AudioOutputBackendStatus | null> {
    setAudioOutputBusy(true);
    try {
      const syncClock = resolveAudioSyncClock(snapshot, mediaState, null);
      if (preferPortaudio && !syncClock) throw new Error('当前播放尚未建立绝对媒体时钟');
      const sampleRateHz = PORTAUDIO_SAMPLE_RATE_HZ;
      const status = await invoke<AudioOutputBackendStatus>('set_audio_output_backend', {
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
  const [mediaProcessingBusy, setMediaProcessingBusy] = useState<MediaProcessingScope | null>(null);
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
  const mediaStateRef = useRef<PlaybackMediaStateMessage | null>(null);
  const pictureInPictureVideoRef = useRef<HTMLVideoElement | null>(null);
  const runtimeMessageRef = useRef<RuntimeParameterMessage | null>(null);
  const runtimeSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const audioSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const audioCycleCandidateIdRef = useRef(0);
  const mediaCyclePlanIdRef = useRef(0);
  const mediaCycleClockIdentityRef = useRef<string | null>(null);
  const audioFuturePlansRef = useRef<MediaCycleQueue<PlannedAudioCyclePayload> | null>(null);
  const videoFuturePlansRef = useRef<MediaCycleQueue<PlannedVideoCyclePayload> | null>(null);
  const nextAudioCyclePlanRef = useRef<AudioCycleCandidatePlan<PendingAudioCyclePayload> | null>(null);
  const audioCyclePlansByIdRef = useRef(new Map<number, AudioCycleCandidatePlan<PendingAudioCyclePayload>>());
  const audioCycleSnapshotRefreshInFlightRef = useRef(false);
  const pendingMediaApplyRef = useRef<{
    params: MediaEffectParams;
    cycle: AudioCycleSample | null;
    scope: MediaProcessingScope;
  } | null>(null);
  const mediaApplyInFlightRef = useRef(false);
  const mediaEffectParamsRef = useRef<MediaEffectParams | null>(null);
  const mediaEffectParamsMutationVersionRef = useRef(0);
  const audioProcessingEnabledRef = useRef(false);
  const videoProcessingEnabledRef = useRef(false);
  const snapshotRefHome = useRef<PlaybackSnapshot | null>(null);
  const schedulePeriodRenderRef = useRef<(params: MediaEffectParams) => void>(() => undefined);
  const lowFrequencyLineCanvasRef = useRef<HTMLCanvasElement | null>(null);

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
    clearFutureMediaCyclePlans();
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
      scheduleRender?: boolean;
      applyAudioToMediaEffectParams?: boolean;
      baseParams?: MediaEffectParams;
      randomChangePeriodMs?: number;
    } = {},
  ) {
    const {
      scheduleRender = true,
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
    const refSource = baseParams ?? mediaEffectParamsRef.current;
    if (refSource) {
      mediaEffectParamsRef.current = {
        ...refSource,
        audio: {
          ...refSource.audio,
          ...sample.values,
          ...(typeof randomChangePeriodMs === 'number'
            ? { random_change_period_ms: randomChangePeriodMs }
            : {}),
        },
      };
    }
    setMediaEffectParams((current) => {
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

  function applyAudioCycleSample(scheduleRender = true) {
    sampleAndCommitAudioCycle({ scheduleRender });
  }

  function nextMediaCyclePlanId(kind: 'audio' | 'video' | 'linked'): string {
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
    const sample = sampleAutomaticVideoParameters();
    return {
      planId,
      periodMediaMs: periodMs,
      payload: { seed: sample.seed, video: sample.video, advanced: sample.advanced, periodMs },
    };
  }

  function bindNextAudioCandidate(plan: MediaCyclePlan<PlannedAudioCyclePayload>) {
    const currentSnapshot = snapshotRefHome.current;
    if (!currentSnapshot?.source_media) {
      nextAudioCyclePlanRef.current = null;
      return;
    }
    audioCycleCandidateIdRef.current += 1;
    nextAudioCyclePlanRef.current = createAudioCycleCandidatePlan(
      audioCycleCandidateIdRef.current,
      {
        ...plan.payload,
        playbackGeneration: currentSnapshot.playback_generation,
        baseAudioStreamRevision: currentSnapshot.audio_stream_revision,
        planId: plan.planId,
        sequence: plan.sequence,
      },
      plan.targetAbsolutePositionMs - plan.periodMediaMs,
      plan.periodMediaMs,
    );
    audioCyclePlansByIdRef.current.set(
      nextAudioCyclePlanRef.current.candidateId,
      nextAudioCyclePlanRef.current,
    );
    while (audioCyclePlansByIdRef.current.size > 4) {
      const oldestCandidateId = audioCyclePlansByIdRef.current.keys().next().value;
      if (typeof oldestCandidateId !== 'number') break;
      audioCyclePlansByIdRef.current.delete(oldestCandidateId);
    }
  }

  function clearFutureMediaCyclePlans(cancelCandidate = true) {
    if (cancelCandidate) cancelNextAudioCycle();
    audioFuturePlansRef.current = null;
    videoFuturePlansRef.current = null;
    mediaCycleClockIdentityRef.current = null;
  }

  function initializeFutureMediaCyclePlans(
    clock: PlaybackMediaStateMessage,
    audioActive: boolean,
    videoActive: boolean,
  ): boolean {
    const identity = `${clock.playback_generation}:${clock.source_revision}:${clock.clock_epoch}`;
    if (mediaCycleClockIdentityRef.current !== identity) {
      clearFutureMediaCyclePlans();
      mediaCycleClockIdentityRef.current = identity;
    }
    if (!audioActive) audioFuturePlansRef.current = null;
    if (!videoActive) videoFuturePlansRef.current = null;
    if (cycleLinkEnabledRef.current && audioActive && videoActive) {
      if (audioFuturePlansRef.current && videoFuturePlansRef.current) return true;
      const sharedRange = intersectPeriodRanges(audioPeriodRangeRef.current, videoPeriodRangeRef.current);
      if (!sharedRange.ok) {
        setError(sharedRange.reason);
        return false;
      }
      const firstPeriod = samplePeriodMsInRange(sharedRange.range);
      const secondPeriod = samplePeriodMsInRange(sharedRange.range);
      const firstPlanId = nextMediaCyclePlanId('linked');
      const secondPlanId = nextMediaCyclePlanId('linked');
      const firstAudio = buildAudioCycleSeed(
        firstPeriod,
        audioCycleSampleRef.current?.presetIds,
        firstPlanId,
      );
      const secondAudio = buildAudioCycleSeed(
        secondPeriod,
        firstAudio?.payload.sample.presetIds,
        secondPlanId,
      );
      if (!firstAudio || !secondAudio) return false;
      const queues = createLinkedMediaCycleQueues(clock.absolute_position_ms, [
        {
          planId: firstPlanId,
          periodMediaMs: firstPeriod,
          audioPayload: firstAudio.payload,
          videoPayload: buildVideoCycleSeed(firstPeriod, firstPlanId).payload,
        },
        {
          planId: secondPlanId,
          periodMediaMs: secondPeriod,
          audioPayload: secondAudio.payload,
          videoPayload: buildVideoCycleSeed(secondPeriod, secondPlanId).payload,
        },
      ]);
      audioFuturePlansRef.current = queues.audio;
      videoFuturePlansRef.current = queues.video;
    } else if (audioActive && videoActive && !audioFuturePlansRef.current && !videoFuturePlansRef.current) {
      const firstAudioPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      const secondAudioPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
      const firstAudio = buildAudioCycleSeed(firstAudioPeriod, audioCycleSampleRef.current?.presetIds);
      const secondAudio = buildAudioCycleSeed(secondAudioPeriod, firstAudio?.payload.sample.presetIds);
      if (!firstAudio || !secondAudio) return false;
      const firstVideoPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
      const secondVideoPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
      const queues = createIndependentMediaCycleQueues(clock.absolute_position_ms, {
        audio: [firstAudio, secondAudio],
        video: [buildVideoCycleSeed(firstVideoPeriod), buildVideoCycleSeed(secondVideoPeriod)],
      });
      audioFuturePlansRef.current = queues.audio;
      videoFuturePlansRef.current = queues.video;
    } else {
      if (audioActive && !audioFuturePlansRef.current) {
        const firstPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
        const secondPeriod = samplePeriodMsInRange(audioPeriodRangeRef.current);
        const first = buildAudioCycleSeed(firstPeriod, audioCycleSampleRef.current?.presetIds);
        const second = buildAudioCycleSeed(secondPeriod, first?.payload.sample.presetIds);
        if (!first || !second) return false;
        audioFuturePlansRef.current = createMediaCycleQueue(clock.absolute_position_ms, [first, second]);
      }
      if (videoActive && !videoFuturePlansRef.current) {
        const firstPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
        const secondPeriod = samplePeriodMsInRange(videoPeriodRangeRef.current);
        videoFuturePlansRef.current = createMediaCycleQueue(clock.absolute_position_ms, [
          buildVideoCycleSeed(firstPeriod),
          buildVideoCycleSeed(secondPeriod),
        ]);
      }
    }
    const nextAudio = audioFuturePlansRef.current?.[0];
    const nextVideo = videoFuturePlansRef.current?.[0];
    if (nextAudio) {
      nextAudioPeriodMsRef.current = nextAudio.periodMediaMs;
      setAudioPeriodMs(nextAudio.periodMediaMs);
    }
    if (nextVideo) {
      nextVideoPeriodMsRef.current = nextVideo.periodMediaMs;
      setVideoPeriodMs(nextVideo.periodMediaMs);
    }
    return true;
  }

  function applyPlannedVideoCycle(plan: MediaCyclePlan<PlannedVideoCyclePayload>) {
    const current = runtimeSchedulerRef.current;
    current.cycle += 1;
    current.lastChangeMs = Date.now();
    setRuntimeCycle(current.cycle);
    setRuntimeLastChangeMs(current.lastChangeMs);
    setVideoPeriodMs(plan.periodMediaMs);
    nextVideoPeriodMsRef.current = plan.periodMediaMs;
    if (mediaEffectParamsRef.current) {
      const next = {
        ...mediaEffectParamsRef.current,
        video: { ...mediaEffectParamsRef.current.video, ...plan.payload.video },
        advanced: { ...mediaEffectParamsRef.current.advanced, ...plan.payload.advanced },
      };
      mediaEffectParamsRef.current = next;
      schedulePeriodRenderRef.current(next);
    }
    setMediaEffectParams((params) => params ? {
      ...params,
      video: { ...params.video, ...plan.payload.video },
      advanced: { ...params.advanced, ...plan.payload.advanced },
    } : params);
  }

  function applyPlannedAudioCycle(
    plan: MediaCyclePlan<PlannedAudioCyclePayload>,
    scheduleRender: boolean,
  ) {
    commitAudioCycleSample(plan.payload.sample, {
      scheduleRender,
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

  function advanceIndependentAudioQueue() {
    const queue = audioFuturePlansRef.current;
    if (!queue) return;
    const period = samplePeriodMsInRange(audioPeriodRangeRef.current);
    const seed = buildAudioCycleSeed(period, queue[1].payload.sample.presetIds);
    if (!seed) {
      audioFuturePlansRef.current = null;
      return;
    }
    audioFuturePlansRef.current = advanceMediaCycleQueue(queue, seed);
  }

  function advanceIndependentVideoQueue() {
    const queue = videoFuturePlansRef.current;
    if (!queue) return;
    const period = samplePeriodMsInRange(videoPeriodRangeRef.current);
    videoFuturePlansRef.current = advanceMediaCycleQueue(queue, buildVideoCycleSeed(period));
  }

  function advanceLinkedQueues() {
    const audioQueue = audioFuturePlansRef.current;
    const videoQueue = videoFuturePlansRef.current;
    const sharedRange = intersectPeriodRanges(audioPeriodRangeRef.current, videoPeriodRangeRef.current);
    if (!audioQueue || !videoQueue || !sharedRange.ok) {
      clearFutureMediaCyclePlans(false);
      if (!sharedRange.ok) setError(sharedRange.reason);
      return;
    }
    const period = samplePeriodMsInRange(sharedRange.range);
    const planId = nextMediaCyclePlanId('linked');
    const audioSeed = buildAudioCycleSeed(period, audioQueue[1].payload.sample.presetIds, planId);
    if (!audioSeed) {
      clearFutureMediaCyclePlans(false);
      return;
    }
    audioFuturePlansRef.current = advanceMediaCycleQueue(audioQueue, audioSeed);
    videoFuturePlansRef.current = advanceMediaCycleQueue(videoQueue, buildVideoCycleSeed(period, planId));
  }

  function postAudioCycleCommand(
    plan: AudioCycleCandidatePlan<PendingAudioCyclePayload>,
    action: 'prepare' | 'commit' | 'cancel',
  ) {
    playbackChannelRef.current?.postMessage({
      version: 1,
      type: 'audio-cycle-command',
      action,
      candidate_id: plan.candidateId,
      playback_generation: plan.sample.playbackGeneration,
      ...(action === 'prepare'
        ? {
            base_audio_stream_revision: plan.sample.baseAudioStreamRevision,
            target_absolute_position_ms: plan.targetAbsolutePositionMs,
            audio: plan.sample.audio,
            audio_variants: plan.sample.audioVariants,
          }
        : {}),
    } satisfies AudioCycleCommandMessage);
  }

  function cancelNextAudioCycle() {
    const plan = nextAudioCyclePlanRef.current;
    nextAudioCyclePlanRef.current = null;
    if (plan) postAudioCycleCommand(plan, 'cancel');
  }
  const [runtimeChannelError, setRuntimeChannelError] = useState<string | null>(null);
  const [diagnosticMessage, setDiagnosticMessage] = useState<DiagnosticMessage | null>(null);
  const [diagnosticNow, setDiagnosticNow] = useState(() => Date.now());
  const [mediaState, setMediaState] = useState<PlaybackMediaStateMessage | null>(null);
  const [pictureInPictureActive, setPictureInPictureActive] = useState(false);
  const pictureInPictureSourceUrl = toAssetUrl(
    snapshot?.current_video_reference ?? snapshot?.source_media?.source_path,
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
        const previous = mediaStateRef.current;
        if (
          previous
          && previous.playback_generation === event.data.playback_generation
          && previous.source_revision === event.data.source_revision
          && (
            previous.clock_epoch > event.data.clock_epoch
            || (
              previous.clock_epoch === event.data.clock_epoch
              && previous.clock_sequence >= event.data.clock_sequence
            )
          )
        ) {
          return;
        }
        mediaStateRef.current = event.data;
        setMediaState(event.data);
        return;
      }
      if (isAudioCycleResultMessage(event.data)) {
        const activePlan = nextAudioCyclePlanRef.current;
        const isActivePlan = activePlan?.candidateId === event.data.candidate_id;
        const plan = isActivePlan
          ? activePlan
          : audioCyclePlansByIdRef.current.get(event.data.candidate_id) ?? null;
        if (!plan) {
          return;
        }
        if (!isActivePlan && event.data.action !== 'commit') {
          if (event.data.action === 'cancel') audioCyclePlansByIdRef.current.delete(event.data.candidate_id);
          return;
        }
        if (event.data.action === 'prepare') {
          if (event.data.accepted) {
            nextAudioCyclePlanRef.current = updateAudioCycleCandidateStatus(plan, 'prepared');
            return;
          }
          if (event.data.error_code === 'audio_candidate_recovery_in_progress' && isActivePlan) {
            const candidateId = plan.candidateId;
            window.setTimeout(() => {
              const current = nextAudioCyclePlanRef.current;
              if (current?.candidateId === candidateId && current.status === 'preparing') {
                nextAudioCyclePlanRef.current = updateAudioCycleCandidateStatus(current, 'planned');
              }
            }, AUDIO_CYCLE_RECOVERY_RETRY_MS);
            return;
          }
          nextAudioCyclePlanRef.current = null;
          audioCyclePlansByIdRef.current.delete(event.data.candidate_id);
          const futurePlan = audioFuturePlansRef.current?.[0];
          const advanceRejectedAudioPlan = () => {
            const latestPlan = audioFuturePlansRef.current?.[0];
            if (latestPlan?.planId !== plan.sample.planId) return;
            if (
              cycleLinkEnabledRef.current
              && latestPlan.planId === videoFuturePlansRef.current?.[0].planId
            ) {
              advanceLinkedQueues();
            } else {
              advanceIndependentAudioQueue();
            }
          };
          const canRefreshCandidate = isTransientAudioCandidateCode(event.data.error_code)
            && futurePlan?.planId === plan.sample.planId
            && !audioCycleSnapshotRefreshInFlightRef.current;
          if (!canRefreshCandidate) {
            if (
              event.data.reason
              && !isTransientAudioCandidateCode(event.data.error_code)
            ) {
              setError(event.data.reason);
            }
            advanceRejectedAudioPlan();
            return;
          }
          audioCycleSnapshotRefreshInFlightRef.current = true;
          void invoke<PlaybackSnapshot>('get_snapshot')
            .then((latestSnapshot) => {
              snapshotRefHome.current = latestSnapshot;
              setSnapshot(latestSnapshot);
              const latestPlan = audioFuturePlansRef.current?.[0];
              const latestClock = resolveAudioSyncClock(latestSnapshot, null, null);
              if (
                !nextAudioCyclePlanRef.current
                && latestPlan?.planId === plan.sample.planId
                && latestClock?.playback_generation === plan.sample.playbackGeneration
              ) {
                const targetDeltaMs = latestPlan.targetAbsolutePositionMs - latestClock.absolute_position_ms;
                if (targetDeltaMs <= 0) {
                  advanceRejectedAudioPlan();
                  return;
                }
                if (targetDeltaMs <= PERIOD_HARD_MAX_MS) {
                  bindNextAudioCandidate(latestPlan);
                  return;
                }
                clearFutureMediaCyclePlans(false);
                mediaStateRef.current = null;
                return;
              }
              advanceRejectedAudioPlan();
            })
            .catch((cause) => {
              setError(getDisplayErrorMessage(cause, '刷新最新声音快照失败'));
              advanceRejectedAudioPlan();
            })
            .finally(() => {
              audioCycleSnapshotRefreshInFlightRef.current = false;
            });
          return;
        }
        if (event.data.action === 'cancel') {
          nextAudioCyclePlanRef.current = null;
          audioCyclePlansByIdRef.current.delete(event.data.candidate_id);
          return;
        }
        if (
          event.data.action === 'commit'
          && [
            'audio_candidate_not_due',
            'audio_candidate_not_ready',
            'audio_candidate_commit_busy',
          ].includes(event.data.error_code ?? '')
          && isActivePlan
        ) {
          nextAudioCyclePlanRef.current = updateAudioCycleCandidateStatus(plan, 'prepared');
          return;
        }
        if (!event.data.committed) {
          if (isActivePlan) nextAudioCyclePlanRef.current = null;
          audioCyclePlansByIdRef.current.delete(event.data.candidate_id);
          if (
            event.data.reason
            && !isTransientAudioCandidateCode(event.data.error_code)
            && !isExpectedAudioCycleSkipCode(event.data.error_code)
          ) {
            setError(event.data.reason);
          }
          if (!isActivePlan) return;
          if (
            cycleLinkEnabledRef.current
            && audioFuturePlansRef.current?.[0].planId === videoFuturePlansRef.current?.[0].planId
          ) {
            advanceLinkedQueues();
          } else {
            advanceIndependentAudioQueue();
          }
          return;
        }
        const committedSnapshot = event.data.snapshot as PlaybackSnapshot | undefined;
        if (!committedSnapshot || committedSnapshot.playback_generation !== plan.sample.playbackGeneration) {
          nextAudioCyclePlanRef.current = null;
          setError('候选音轨已提交，但返回的播放快照无效');
          return;
        }
        snapshotRefHome.current = committedSnapshot;
        setSnapshot(committedSnapshot);
        const futureAudioPlan = audioFuturePlansRef.current?.[0];
        const committedPlan: MediaCyclePlan<PlannedAudioCyclePayload> =
          futureAudioPlan?.planId === plan.sample.planId
            ? futureAudioPlan
            : {
                planId: plan.sample.planId,
                sequence: plan.sample.sequence,
                periodMediaMs: plan.sample.periodMs,
                targetAbsolutePositionMs: plan.targetAbsolutePositionMs,
                payload: plan.sample,
              };
        applyPlannedAudioCycle(committedPlan, false);
        audioCyclePlansByIdRef.current.delete(event.data.candidate_id);
        if (!isActivePlan) {
          clearFutureMediaCyclePlans();
          setError('旧计划在取消竞态中已提交，已接受后端结果并重建后续周期');
          return;
        }
        nextAudioCyclePlanRef.current = null;
        const linkedVideoPlan = videoFuturePlansRef.current?.[0];
        if (cycleLinkEnabledRef.current && linkedVideoPlan?.planId === committedPlan.planId) {
          applyPlannedVideoCycle(linkedVideoPlan);
          advanceLinkedQueues();
        } else {
          advanceIndependentAudioQueue();
        }
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
      if (!isDiagnosticMessage(event.data)) return;
      const diagnostic = event.data;
      setDiagnosticMessage((current) => {
        const currentPortAudioIsFresh = current?.source === 'portaudio-mixed-pcm'
          && current.line.length > 0
          && Date.now() - current.sent_at_ms < 1_500;
        return currentPortAudioIsFresh && diagnostic.source === 'web-audio-analyser'
          ? current
          : diagnostic;
      });
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
    drawDiagnosticCanvas(
      lowFrequencyLineCanvasRef.current,
      diagnosticMessage?.line ?? [],
      diagnosticMessage?.source === 'portaudio-mixed-pcm' ? '#1677ff' : '#fa8c16',
      diagnosticMessage?.source === 'portaudio-mixed-pcm' ? 'rgba(22, 119, 255, 0.12)' : 'rgba(250, 140, 22, 0.12)',
    );
  }, [diagnosticMessage]);

  useEffect(() => {
    const timer = window.setInterval(() => setDiagnosticNow(Date.now()), 250);
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
  }, [snapshot?.audio_processing_enabled, snapshot?.video_processing_enabled]);

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
      void invoke<AudioOutputDevice[]>('list_audio_output_devices')
        .then((devices) => {
          if (!cancelled) {
            setAudioOutputDevices(
              devices.filter((device) => typeof device.host_api === 'string'),
            );
          }
        })
        .catch(() => {
          if (!cancelled) setAudioOutputDevices([]);
        });
      void invoke<MediaEffectParams>('get_default_media_effect_params')
        .then((params) => {
          if (!cancelled && mediaEffectParamsMutationVersionRef.current === initialMutationVersion) {
            // 声音处理参数默认从预设抽样；采样率/码率等结构字段保留默认。
            sampleAndCommitAudioCycle({
              scheduleRender: false,
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

  const runtimeBaseParameters = useMemo<RuntimeBaseParameters | null>(() => {
    if (!mediaEffectParams) return null;
    return {
      audio_gain_db:
        mediaEffectParams.audio.input_gain_db +
        mediaEffectParams.audio.output_gain_db +
        mediaEffectParams.audio.loudness_adjustment_db,
      audio_low_eq_db: mediaEffectParams.audio.low_eq_db,
      audio_mid_eq_db: mediaEffectParams.audio.mid_eq_db,
      audio_high_eq_db: mediaEffectParams.audio.high_eq_db,
      audio_input_gain_db: mediaEffectParams.audio.input_gain_db,
      audio_output_gain_db: mediaEffectParams.audio.output_gain_db,
      audio_loudness_adjustment_db: mediaEffectParams.audio.loudness_adjustment_db,
      audio_pitch_shift_semitones: mediaEffectParams.audio.pitch_shift_semitones,
      audio_playback_speed: mediaEffectParams.audio.playback_speed,
      audio_fade_in_ms: mediaEffectParams.audio.fade_in_ms,
      audio_fade_out_ms: mediaEffectParams.audio.fade_out_ms,
      audio_reverb_wet_percent: mediaEffectParams.audio.reverb_wet_percent,
      audio_noise_reduction_percent: mediaEffectParams.audio.noise_reduction_percent,
      audio_phase_perturbation_percent: mediaEffectParams.audio.phase_perturbation_percent,
      audio_vibrato_frequency_hz: mediaEffectParams.audio.vibrato_frequency_hz,
      audio_vibrato_depth_percent: mediaEffectParams.audio.vibrato_depth_percent,
      audio_environment_noise_percent: mediaEffectParams.audio.environment_noise_percent,
      audio_environment_noise_dbfs: mediaEffectParams.audio.environment_noise_dbfs,
      audio_filter_q: mediaEffectParams.audio.filter_q,
      audio_sample_rate_hz: mediaEffectParams.audio.sample_rate_hz ?? 0,
      audio_output_bitrate_kbps: mediaEffectParams.audio.output_bitrate_kbps,
      video_brightness_percent: mediaEffectParams.video.brightness_percent,
      video_contrast_percent: mediaEffectParams.video.contrast_percent,
      video_saturation_percent: mediaEffectParams.video.saturation_percent,
      video_hue_rotation_degrees: mediaEffectParams.video.hue_rotation_degrees,
      video_blur_radius_px: mediaEffectParams.video.blur_radius_px,
      video_pixel_scale_percent: mediaEffectParams.video.pixel_scale_percent,
      video_space_x_offset_px: mediaEffectParams.video.space_x_offset_px,
      video_space_y_offset_px: mediaEffectParams.video.space_y_offset_px,
    };
  }, [
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
  const [cycleLinkEnabled, setCycleLinkEnabled] = useState(false);
  const audioPeriodRangeRef = useRef(audioPeriodRange);
  const videoPeriodRangeRef = useRef(videoPeriodRange);
  const cycleLinkEnabledRef = useRef(cycleLinkEnabled);
  const nextAudioPeriodMsRef = useRef(samplePeriodMsInRange(audioPeriodRange));
  const nextVideoPeriodMsRef = useRef(samplePeriodMsInRange(videoPeriodRange));
  const [, setAudioPeriodMs] = useState(() => nextAudioPeriodMsRef.current);
  const [, setVideoPeriodMs] = useState(() => nextVideoPeriodMsRef.current);
  const playbackActive = snapshot?.playback_state?.toLowerCase() === 'playing';
  // 暂停/停止时冻结音视频周期变化，避免画面停了参数还在跳。
  const runtimeActive = playbackActive && videoProcessingEnabled;
  const audioPeriodActive = playbackActive && audioProcessingEnabled && Boolean(mediaEffectParams);
  const portAudioCycleEnabled = shouldKeepPortAudioCycleScheduling(audioOutputBackend);

  useEffect(() => {
    if (!AUTO_PORTAUDIO_ENABLED || !PORTAUDIO_FORMAL_SOURCE_SYNC_READY) return;
    const attempt = autoPortAudioAttemptRef.current;
    const clearRetryTimer = () => {
      if (autoPortAudioRetryTimerRef.current !== null) {
        window.clearTimeout(autoPortAudioRetryTimerRef.current);
        autoPortAudioRetryTimerRef.current = null;
      }
    };
    if (!playbackActive || !audioProcessingEnabled) {
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
    playbackActive,
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
    clearFutureMediaCyclePlans();
  }, [audioPeriodRange]);
  useEffect(() => {
    videoPeriodRangeRef.current = videoPeriodRange;
    saveVideoPeriodRange(videoPeriodRange);
    clearFutureMediaCyclePlans();
  }, [videoPeriodRange]);
  useEffect(() => {
    cycleLinkEnabledRef.current = cycleLinkEnabled;
    clearFutureMediaCyclePlans();
  }, [cycleLinkEnabled]);
  useEffect(() => {
    if (!cycleLinkEnabled) return;
    const sharedRange = intersectPeriodRanges(audioPeriodRange, videoPeriodRange);
    if (sharedRange.ok) return;
    setCycleLinkEnabled(false);
    setError(sharedRange.reason);
  }, [audioPeriodRange, cycleLinkEnabled, videoPeriodRange]);

  useEffect(() => {
    if (!runtimeActive || !runtimeBaseParameters) {
      if (!runtimeActive) {
        runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
        setRuntimeCycle(0);
        setRuntimeLastChangeMs(null);
      }
      return;
    }
    const scheduler = runtimeSchedulerRef.current;
    if (scheduler.cycle === 0 || scheduler.lastChangeMs === null) {
      const initialNowMs = Date.now();
      const initialVideo = sampleAutomaticVideoParameters();
      runtimeSchedulerRef.current = { cycle: 1, lastChangeMs: initialNowMs };
      setRuntimeCycle(1);
      setRuntimeLastChangeMs(initialNowMs);
      if (mediaEffectParamsRef.current) {
        const next = {
          ...mediaEffectParamsRef.current,
          video: { ...mediaEffectParamsRef.current.video, ...initialVideo.video },
          advanced: { ...mediaEffectParamsRef.current.advanced, ...initialVideo.advanced },
        };
        mediaEffectParamsRef.current = next;
        schedulePeriodRenderRef.current(next);
      }
      setMediaEffectParams((current) => {
        if (!current) return current;
        return {
          ...current,
          video: { ...current.video, ...initialVideo.video },
          advanced: { ...current.advanced, ...initialVideo.advanced },
        };
      });
    }
  }, [runtimeActive, runtimeBaseParameters, snapshot?.playback_state]);

  useEffect(() => {
    if (!audioPeriodActive) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      clearFutureMediaCyclePlans();
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
      sampleAndCommitAudioCycle({ randomChangePeriodMs: firstPeriod });
    }
  }, [audioPeriodActive]);

  // 定时器只负责唤醒；prepare/commit/apply 全部以最终播放窗的绝对媒体时间为准。
  useEffect(() => {
    if (!audioPeriodActive && !runtimeActive) {
      clearFutureMediaCyclePlans();
      return;
    }
    let cancelled = false;
    const timer = window.setInterval(() => {
      if (cancelled) return;
      const clock = mediaStateRef.current;
      if (!clock || clock.paused || clock.playback_generation !== snapshotRefHome.current?.playback_generation) return;
      if (!initializeFutureMediaCyclePlans(clock, audioPeriodActive, runtimeActive)) return;
      const linked = cycleLinkEnabledRef.current && audioPeriodActive && runtimeActive;

      if (linked) {
        const audioPlan = audioFuturePlansRef.current?.[0];
        const videoPlan = videoFuturePlansRef.current?.[0];
        if (!audioPlan || !videoPlan || audioPlan.planId !== videoPlan.planId) {
          clearFutureMediaCyclePlans();
          return;
        }
        if (!portAudioCycleEnabled && clock.absolute_position_ms >= audioPlan.targetAbsolutePositionMs) {
          applyPlannedAudioCycle(audioPlan, false);
          applyPlannedVideoCycle(videoPlan);
          advanceLinkedQueues();
          return;
        }
      } else if (runtimeActive) {
        const videoPlan = videoFuturePlansRef.current?.[0];
        if (videoPlan && clock.absolute_position_ms >= videoPlan.targetAbsolutePositionMs) {
          applyPlannedVideoCycle(videoPlan);
          advanceIndependentVideoQueue();
        }
      }

      if (!audioPeriodActive) return;
      const audioPlan = audioFuturePlansRef.current?.[0];
      if (!audioPlan) return;
      if (!portAudioCycleEnabled) {
        if (clock.absolute_position_ms >= audioPlan.targetAbsolutePositionMs) {
          applyPlannedAudioCycle(audioPlan, true);
          advanceIndependentAudioQueue();
        }
        return;
      }

      {
        const currentSnapshot = snapshotRefHome.current;
        let plan = nextAudioCyclePlanRef.current;
        const stalePlan = plan && currentSnapshot && (
          plan.sample.playbackGeneration !== currentSnapshot.playback_generation
          || plan.sample.baseAudioStreamRevision !== currentSnapshot.audio_stream_revision
          || plan.sample.planId !== audioPlan.planId
        );
        if (stalePlan) {
          cancelNextAudioCycle();
          plan = null;
        }
        if (audioCycleSnapshotRefreshInFlightRef.current) return;
        if (!plan) {
          bindNextAudioCandidate(audioPlan);
          plan = nextAudioCyclePlanRef.current;
        }
        if (
          plan
          && clock.absolute_position_ms >= plan.targetAbsolutePositionMs
          && plan.status !== 'prepared'
          && plan.status !== 'committing'
        ) {
          cancelNextAudioCycle();
          if (linked) advanceLinkedQueues();
          else advanceIndependentAudioQueue();
          return;
        }
        const action = getAudioCycleCoordinatorAction(
          plan,
          clock.absolute_position_ms,
          clock.playback_rate,
          audioCommitLeadPlaybackMs(audioOutputBackendRef.current),
        );
        if (plan && action === 'prepare') {
          nextAudioCyclePlanRef.current = updateAudioCycleCandidateStatus(plan, 'preparing');
          postAudioCycleCommand(plan, 'prepare');
        } else if (plan && action === 'commit') {
          nextAudioCyclePlanRef.current = updateAudioCycleCandidateStatus(plan, 'committing');
          postAudioCycleCommand(plan, 'commit');
        } else if (plan && action === 'expire') {
          cancelNextAudioCycle();
          if (linked) advanceLinkedQueues();
          else advanceIndependentAudioQueue();
        }
      }
    }, 100);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      cancelNextAudioCycle();
    };
  }, [audioPeriodActive, portAudioCycleEnabled, runtimeActive]);

  // 仅播放中才推送实时参数；暂停时冻结播放窗效果。
  useEffect(() => {
    const liveAudio = playbackActive && audioProcessingEnabled;
    const liveVideo = runtimeActive;
    const audioPayload = runtimeBaseParameters && (liveAudio || liveVideo)
      ? runtimeBaseParameters
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

  function updateMediaEffectParam(section: 'audio' | 'video' | 'advanced', field: PropertyKey, value: unknown) {
    if (!mediaEffectParams) return;
    mediaEffectParamsMutationVersionRef.current += 1;
    if (section === 'audio' || section === 'video') clearFutureMediaCyclePlans();
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
      const defaults = await invoke<MediaEffectParams>('get_default_media_effect_params');
      if (mutationVersion !== mediaEffectParamsMutationVersionRef.current) return;
      sampleAndCommitAudioCycle({ scheduleRender: false, baseParams: defaults });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '恢复媒体效果参数默认值失败');
    }
  }

  function rerollSubtleAudioParams() {
    if (!mediaEffectParams) return;
    clearFutureMediaCyclePlans();
    const nowMs = Date.now();
    audioSchedulerRef.current = {
      cycle: Math.max(1, audioSchedulerRef.current.cycle + 1),
      lastChangeMs: nowMs,
    };
    setAudioVariationCycle(audioSchedulerRef.current.cycle);
    setAudioLastChangeMs(nowMs);
    applyAudioCycleSample(true);
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
    sourceOverride?: PlaybackPoolSource | null,
  ): Promise<boolean> {
    setPlayerWindowBusy(true);
    setError(null);
    try {
      const source = sourceOverride ?? snapshot?.source_media ?? null;
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
    if (!shouldIssuePlaybackCommand(command, snapshotRefHome.current?.playback_state)) return;
    if (action === 'pause' || action === 'stop') clearFutureMediaCyclePlans();
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
    clearFutureMediaCyclePlans();
    mediaEffectParamsMutationVersionRef.current += 1;
    pendingMediaApplyRef.current = null;
    audioProcessingEnabledRef.current = next.audio_processing_enabled;
    videoProcessingEnabledRef.current = next.video_processing_enabled;
    setVideoProcessingEnabled(next.video_processing_enabled);
    setAudioProcessingEnabled(next.audio_processing_enabled);
    let sampledParams: MediaEffectParams | null = null;
    // 打开声音/视频时立刻形成首轮快照；播放中会进入同一条本地处理队列。
    if (mediaEffectParams && (next.audio_processing_enabled || next.video_processing_enabled)) {
      const nowMs = Date.now();
      const initialAudioPeriodMs = next.audio_processing_enabled
        ? samplePeriodMsInRange(audioPeriodRangeRef.current)
        : null;
      const audioSample = next.audio_processing_enabled
        ? sampleAndCommitAudioCycle({
            scheduleRender: false,
            applyAudioToMediaEffectParams: false,
            randomChangePeriodMs: initialAudioPeriodMs ?? undefined,
          })
        : null;
      const videoSample = next.video_processing_enabled
        ? sampleAutomaticVideoParameters()
        : null;
      const base = mediaEffectParamsRef.current ?? mediaEffectParams;
      sampledParams = {
        ...base,
        audio: audioSample ? {
          ...base.audio,
          ...audioSample.values,
          random_change_period_ms: initialAudioPeriodMs ?? base.audio.random_change_period_ms,
        } : base.audio,
        video: videoSample ? { ...base.video, ...videoSample.video } : base.video,
        advanced: videoSample ? { ...base.advanced, ...videoSample.advanced } : base.advanced,
      };
      mediaEffectParamsRef.current = sampledParams;
      setMediaEffectParams(sampledParams);
      if (next.audio_processing_enabled) {
        if (initialAudioPeriodMs !== null) {
          nextAudioPeriodMsRef.current = initialAudioPeriodMs;
          setAudioPeriodMs(initialAudioPeriodMs);
        }
        audioSchedulerRef.current = playbackActive
          ? { cycle: 1, lastChangeMs: nowMs }
          : { cycle: 0, lastChangeMs: null };
        setAudioVariationCycle(playbackActive ? 1 : 0);
        setAudioLastChangeMs(playbackActive ? nowMs : null);
      }
      if (next.video_processing_enabled) {
        runtimeSchedulerRef.current = playbackActive
          ? { cycle: 1, lastChangeMs: nowMs }
          : { cycle: 0, lastChangeMs: null };
        setRuntimeCycle(playbackActive ? 1 : 0);
        setRuntimeLastChangeMs(playbackActive ? nowMs : null);
      }
    }
    if (!next.audio_processing_enabled) {
      audioSchedulerRef.current = { cycle: 0, lastChangeMs: null };
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
      snapshotRefHome.current = nextSnapshot;
      setSnapshot(nextSnapshot);
      if (playbackActive && nextSnapshot.source_media && sampledParams) {
        schedulePeriodRenderRef.current(sampledParams);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '更新处理开关失败');
    }
  }

  function schedulePeriodMediaRender(params: MediaEffectParams) {
    if (!audioProcessingEnabledRef.current && !videoProcessingEnabledRef.current) return;
    if (!snapshotRefHome.current?.source_media) return;
    const scope: MediaProcessingScope = audioProcessingEnabledRef.current
      ? (videoProcessingEnabledRef.current ? 'both' : 'audio')
      : 'video';
    void applyMediaProcessing(scope, params);
  }
  schedulePeriodRenderRef.current = schedulePeriodMediaRender;

  async function applyMediaProcessing(
    scope: MediaProcessingScope,
    paramsOverride?: MediaEffectParams,
    showSaveSuccess = false,
  ) {
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
    const scopeEnabled = scope === 'video'
      ? videoProcessingEnabledRef.current && !audioProcessingEnabledRef.current
      : scope === 'audio'
        ? audioProcessingEnabledRef.current && !videoProcessingEnabledRef.current
        : videoProcessingEnabledRef.current && audioProcessingEnabledRef.current;
    const currentSnapshot = snapshotRefHome.current ?? snapshot;
    if (!params || !currentSnapshot?.source_media || !scopeEnabled) return;
    let validation: MediaParameterValidationResult;
    try {
      validation = await invoke<MediaParameterValidationResult>('validate_media_effect_params', {
        request: params,
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '媒体效果参数校验失败'));
      return;
    }
    if (!validation.valid) {
      setError(validation.errors[0]?.message ?? '媒体效果参数校验失败');
      return;
    }
    // 已有 FFmpeg 在跑 / IPC 在途：只排队最新参数，绝不中断当前任务。
    if (
      mediaApplyInFlightRef.current
      || currentSnapshot.audio_processing_status === 'processing'
      || currentSnapshot.video_processing_status === 'processing'
    ) {
      pendingMediaApplyRef.current = {
        params,
        cycle: audioCycleSampleRef.current,
        scope,
      };
      if (showSaveSuccess) void messageApi.success('保存成功');
      return;
    }
    mediaApplyInFlightRef.current = true;
    setError(null);
    setMediaProcessingBusy(scope);
    try {
      await ensureRuntimeResources('media', async () => {
        setMediaProcessingBusy(scope);
        try {
          // 不抢停当前处理任务：启动请求直接携带最新参数。
          const cycle = audioCycleSampleRef.current;
          const audio_variants =
            scope !== 'video'
            && audioProcessingEnabledRef.current
            && audioMixEnabledRef.current
            && cycle
            && cycle.variants.length > 1
              ? buildAudioVariantsFromCycle(params.audio, cycle)
              : undefined;
          const nextSnapshot = await invoke<PlaybackSnapshot>('start_media_processing', {
            request: {
              params,
              audio_variants,
              ambient_sound_path: scope === 'video' ? null : ambientSoundPath,
            },
          });
          setSnapshot(nextSnapshot);
          if (
            nextSnapshot.fallback_reason &&
            (nextSnapshot.audio_processing_status === 'failed' || nextSnapshot.video_processing_status === 'failed')
          ) {
            setError(nextSnapshot.fallback_reason);
            return;
          }
          if (showSaveSuccess) void messageApi.success('保存成功');
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

  async function flushPendingMediaApply() {
    const pending = pendingMediaApplyRef.current;
    if (!pending || !snapshot?.source_media) return;
    if (mediaApplyInFlightRef.current || mediaProcessingBusy !== null) return;
    if (snapshot.audio_processing_status === 'processing' || snapshot.video_processing_status === 'processing') return;
    pendingMediaApplyRef.current = null;
    if (pending.scope !== 'video' && pending.cycle) {
      commitAudioCycleSample(pending.cycle, { applyAudioToMediaEffectParams: false });
    }
    await applyMediaProcessing(pending.scope, pending.params);
  }

  // FFmpeg 结束后稍等再续跑排队任务，避免 Tag 一直停在 processing、听感被连续重渲打断。
  useEffect(() => {
    const processing =
      snapshot?.audio_processing_status === 'processing'
      || snapshot?.video_processing_status === 'processing';
    if (mediaWasProcessingRef.current && !processing) {
      const timer = window.setTimeout(() => {
        void flushPendingMediaApply();
      }, 2_500);
      mediaWasProcessingRef.current = false;
      return () => window.clearTimeout(timer);
    }
    mediaWasProcessingRef.current = Boolean(processing);
  }, [snapshot?.audio_processing_status, snapshot?.video_processing_status]);

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
        multiple: true,
        filters: [{ name: '视频文件', extensions: [...SUPPORTED_VIDEO_EXTENSIONS] }],
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
        importVideoInFlightRef.current = true;
        setImportVideoBusy(true);
        try {
          const nextSnapshot = await invoke<PlaybackSnapshot>('probe_local_videos', { request: { paths } });
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
    setInterludeSaveError(null);
    setInterludeDraft((current) => ({ ...current, ...patch }));
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
      const audioVariationPeriod = normalizeAudioPeriodRange(
        interludeDraft.audioVariationPeriodMinMs,
        interludeDraft.audioVariationPeriodMaxMs,
      );
      const nextSnapshot = await invoke<PlaybackSnapshot>('set_interlude_config', {
        request: {
          enabled: interludeDraft.enabled,
          directory: interludeDraft.directory,
          audio_selection_mode: interludeDraft.audioSelectionMode,
          audio_fixed_preset_id: audioFixedPresetId,
          audio_preset_ids: audioPresetIds,
          audio_mix_enabled: interludeDraft.audioMixEnabled,
          audio_mix_pick_min: audioMixPickMin,
          audio_mix_pick_max: audioMixPickMax,
          audio_variation_mode: interludeDraft.audioVariationMode,
          audio_variation_period_min_ms: audioVariationPeriod.minMs,
          audio_variation_period_max_ms: audioVariationPeriod.maxMs,
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
  const runtimeRemainingMs = nextVideoPlan
    ? Math.max(0, nextVideoPlan.targetAbsolutePositionMs - (mediaState?.absolute_position_ms ?? 0))
    : null;
  const runtimeProgressPercent = !nextVideoPlan || nextVideoPlan.periodMediaMs <= 0 || runtimeRemainingMs === null
    ? 0
    : Math.max(0, Math.min(100, Math.round(((nextVideoPlan.periodMediaMs - runtimeRemainingMs) / nextVideoPlan.periodMediaMs) * 100)));
  const nextAudioPlan = audioFuturePlansRef.current?.[0];
  const audioRemainingMs = nextAudioPlan
    ? Math.max(0, nextAudioPlan.targetAbsolutePositionMs - (mediaState?.absolute_position_ms ?? 0))
    : null;
  const audioProgressPercent = !nextAudioPlan || nextAudioPlan.periodMediaMs <= 0 || audioRemainingMs === null
    ? 0
    : Math.max(0, Math.min(100, Math.round(((nextAudioPlan.periodMediaMs - audioRemainingMs) / nextAudioPlan.periodMediaMs) * 100)));
  const diagnosticAgeMs = diagnosticMessage === null ? null : Math.max(0, diagnosticNow - diagnosticMessage.sent_at_ms);
  const diagnosticFresh = diagnosticAgeMs !== null && diagnosticAgeMs < 1_500;
  const diagnosticHasLine = diagnosticMessage !== null && diagnosticMessage.line.length > 0;
  const diagnosticHasPcm = diagnosticMessage?.source === 'portaudio-mixed-pcm'
    ? diagnosticMessage.has_pcm
    : diagnosticHasLine;
  const diagnosticStatus = diagnosticMessage === null
    ? '无数据：尚未收到诊断消息'
    : !diagnosticFresh
      ? `数据已过期（${diagnosticAgeMs}ms 前）`
      : !diagnosticHasPcm
        ? '无数据：PortAudio 快照和 Web Audio 回退均不可用'
        : diagnosticMessage.source === 'portaudio-mixed-pcm'
          ? '实时：来自 PortAudio 最终混音 PCM'
          : '兼容回退：PortAudio 快照不可用，来自 Web Audio analyser';
  const displayedMediaEffectParams = mediaEffectParams
    ? {
        ...mediaEffectParams,
        audio: {
          ...mediaEffectParams.audio,
          current_formant_hz: diagnosticFresh
            && diagnosticMessage?.source === 'portaudio-mixed-pcm'
            && diagnosticMessage.current_formant_hz !== null
            ? diagnosticMessage.current_formant_hz
            : null,
        },
      }
    : null;
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
  const actualAudioOutputLabel = getActualAudioOutputLabel(audioOutputBackend);
  const actualAudioStreamVariantCount = getActualAudioStreamVariantCount(snapshot);
  const actualAudioMixLabel = actualAudioStreamVariantCount === null
    ? '未上报（兼容旧快照）'
    : actualAudioOutputLabel === 'PortAudio'
      ? `${actualAudioStreamVariantCount} 条支路`
      : `未进入正式输出（配置 ${actualAudioStreamVariantCount} 条）`;
  const audioSettingsApplyScope: MediaProcessingScope = videoProcessingEnabled ? 'both' : 'audio';
  const portAudioFallback = audioProcessingEnabled
    && Boolean(audioOutputBackend?.preferred_portaudio)
    && actualAudioOutputLabel === 'WebView';
  const activeAudioPresets = useMemo(
    () => audioActivePresetIds
      .map((id) => AUDIO_VALUE_PRESETS.find((preset) => preset.id === id))
      .filter((preset): preset is (typeof AUDIO_VALUE_PRESETS)[number] => Boolean(preset)),
    [audioActivePresetIds],
  );
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
  const audioCapabilityRows = useMemo(
    () => buildAudioCapabilityRows(
      audioDisplayParams,
      audioProcessingEnabled,
      actualAudioBranches.length > 0,
    ),
    [actualAudioBranches.length, audioDisplayParams, audioProcessingEnabled],
  );

  const playbackProgressPercent = mediaDuration > 0 ? (mediaCurrentTime / mediaDuration) * 100 : 0;
  const playbackPoolPosition = snapshot?.source_media_index === null || snapshot?.source_media_index === undefined
    ? '等待导入'
    : `第 ${snapshot.source_media_index + 1}/${sourceMediaPool.length} 项`;
  const sourceSpecification = currentSource
    ? `${currentSource.width ?? '-'}×${currentSource.height ?? '-'} · ${currentSource.frame_rate_fps?.toFixed(0) ?? '-'}fps`
    : '未导入';
  const statusItems: DesktopStatusItem[] = [
    {
      key: 'playback',
      label: '播放进度',
      value: `${Math.round(playbackProgressPercent)}%`,
      meta: `${formatMediaTime(mediaCurrentTime)} / ${formatMediaTime(mediaDuration)}`,
      tone: 'green',
      progress: playbackProgressPercent,
    },
    {
      key: 'loop',
      label: '循环次数',
      value: String(snapshot?.playback_pool_cycle ?? 0),
      meta: `${playbackPoolPosition} · ${getPlaybackDisplayLabel(playbackDisplayState)}`,
      tone: 'green',
    },
    {
      key: 'video',
      label: '视频处理',
      value: videoProcessingEnabled ? getProcessingStatusLabel(videoProcessingStatus) : '已关闭',
      meta: `第 ${runtimeCycle} 次变化`,
      tone: 'blue',
    },
    {
      key: 'audio',
      label: '普通声音',
      value: audioProcessingEnabled ? getProcessingStatusLabel(audioProcessingStatus) : '未开启',
      meta: actualAudioOutputLabel,
      tone: 'pink',
    },
    {
      key: 'source',
      label: '源规格',
      value: sourceSpecification,
      meta: currentSource?.file_name ?? '等待导入视频',
      tone: 'blue',
    },
    {
      key: 'output',
      label: '最终效果窗口',
      value: diagnosticFresh ? '已连接' : '未连接',
      meta: pictureInPictureActive ? '画中画已打开' : '单实例输出',
      tone: 'pink',
      actionLabel: '打开',
      onAction: () => void openFinalEffectWindowFromHome(),
    },
  ];
  const audioParameterControls = mediaEffectParams ? (
    <AudioParameterControls
      value={mediaEffectParams.audio}
      disabled={!audioProcessingEnabled}
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
        src={pictureInPictureSourceUrl ?? undefined}
        muted
        playsInline
        preload="auto"
        aria-hidden="true"
        onLoadedMetadata={syncPictureInPictureVideo}
        style={{ position: 'fixed', width: 1, height: 1, opacity: 0, pointerEvents: 'none' }}
      />
      <DesktopShell>
        <DesktopColumn area="source" ariaLabel="视频素材与播放控制">
          <PlaybackPoolPanel
            sources={sourceMediaPool}
            currentIndex={snapshot?.source_media_index ?? null}
            importBusy={importVideoBusy}
            importDisabled={importVideoBusy || runtimeResourceBusy}
            error={error}
            onImport={() => void importVideo()}
          />

          <DesktopPanel title="播放控制">
            <div className="desktop-control-row">
              <Button
                type="primary"
                icon={<PlayCircleOutlined />}
                onClick={() => void startPlaybackFromHome()}
                disabled={!canStartPlayback}
                loading={playbackActionBusy === 'resume' || playerWindowBusy}
              >
                播放
              </Button>
              <Button
                icon={<PauseCircleOutlined />}
                onClick={() => void runPlaybackAction('pause', 'pause_playback')}
                disabled={!canPause}
                loading={playbackActionBusy === 'pause'}
              >
                暂停
              </Button>
              <Button
                icon={<PlayCircleOutlined />}
                onClick={() => void runPlaybackAction('resume', 'resume_playback')}
                disabled={!canResume}
              >
                继续
              </Button>
              <Button
                danger
                aria-label="停止播放"
                icon={<StopOutlined />}
                onClick={() => void runPlaybackAction('stop', 'stop_playback')}
                disabled={!canStop}
              />
            </div>
            <div className="desktop-inline-between" style={{ marginTop: 10 }}>
              <Typography.Text className="desktop-muted">播放进度</Typography.Text>
              <Typography.Text>{formatMediaTime(mediaCurrentTime)} / {formatMediaTime(mediaDuration)}</Typography.Text>
            </div>
            <Slider
              aria-label="播放进度"
              min={0}
              max={Math.max(mediaDuration, 1)}
              value={mediaCurrentTime}
              disabled={!mediaState || mediaDuration <= 0}
              tooltip={{ formatter: (value) => formatMediaTime(value ?? 0) }}
              onChange={(value) => {
                if (!mediaState) return;
                clearFutureMediaCyclePlans();
                postPlaybackMediaControl({
                  version: 1,
                  type: 'playback-media-control',
                  action: 'seek',
                  current_time: value,
                  playback_generation: mediaState.playback_generation,
                });
              }}
            />
            <div className="desktop-control-row">
              <Button
                aria-label={mediaState?.muted ? '取消静音' : '静音'}
                icon={mediaState?.muted ? <MutedOutlined /> : <SoundOutlined />}
                disabled={!mediaState}
                onClick={() => postPlaybackMediaControl({ version: 1, type: 'playback-media-control', action: 'toggle-muted' })}
              />
              <Slider
                aria-label="音量"
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
              <Button
                aria-label="切换画中画"
                icon={<PictureOutlined />}
                disabled={!mediaState || !pictureInPictureSupported}
                onClick={() => void togglePictureInPicture()}
              />
            </div>
          </DesktopPanel>

          <DesktopPanel title="周期变化">
            <div className="desktop-period-grid">
              <CompactNumberField
                ariaLabel="视频周期最小秒"
                label="视频最小"
                unit="s"
                min={PERIOD_HARD_MIN_MS / 1000}
                max={PERIOD_HARD_MAX_MS / 1000}
                value={videoPeriodRange.minMs / 1000}
                onChange={(value) => typeof value === 'number' && setVideoPeriodRange((current) => normalizeVideoPeriodRange(value * 1000, current.maxMs))}
              />
              <CompactNumberField
                ariaLabel="视频周期最大秒"
                label="视频最大"
                unit="s"
                min={PERIOD_HARD_MIN_MS / 1000}
                max={PERIOD_HARD_MAX_MS / 1000}
                value={videoPeriodRange.maxMs / 1000}
                onChange={(value) => typeof value === 'number' && setVideoPeriodRange((current) => normalizeVideoPeriodRange(current.minMs, value * 1000))}
              />
              <CompactNumberField
                ariaLabel="声音周期最小秒"
                label="声音最小"
                unit="s"
                min={PERIOD_HARD_MIN_MS / 1000}
                max={PERIOD_HARD_MAX_MS / 1000}
                value={audioPeriodRange.minMs / 1000}
                onChange={(value) => typeof value === 'number' && setAudioPeriodRange((current) => normalizeAudioPeriodRange(value * 1000, current.maxMs))}
              />
              <CompactNumberField
                ariaLabel="声音周期最大秒"
                label="声音最大"
                unit="s"
                min={PERIOD_HARD_MIN_MS / 1000}
                max={PERIOD_HARD_MAX_MS / 1000}
                value={audioPeriodRange.maxMs / 1000}
                onChange={(value) => typeof value === 'number' && setAudioPeriodRange((current) => normalizeAudioPeriodRange(current.minMs, value * 1000))}
              />
            </div>
            <div className="desktop-status-line" style={{ marginTop: 10 }}>
              <Typography.Text>声音与视频联动周期</Typography.Text>
              <Switch
                aria-label="声音与视频联动周期"
                checked={cycleLinkEnabled}
                disabled={!audioProcessingEnabled || !videoProcessingEnabled}
                onChange={(checked) => {
                  if (checked) {
                    const sharedRange = intersectPeriodRanges(audioPeriodRange, videoPeriodRange);
                    if (!sharedRange.ok) {
                      setError(sharedRange.reason);
                      return;
                    }
                  }
                  setCycleLinkEnabled(checked);
                }}
              />
            </div>
          </DesktopPanel>

          <DesktopPanel title="本地运行状态">
            <div className="desktop-status-line"><span>媒体引擎</span><Tag color={mediaEngineCapabilities?.available ? 'success' : 'warning'}>{mediaEngineCapabilities?.available ? '就绪' : '不可用'}</Tag></div>
            <div className="desktop-status-line"><span>音频出口</span><Tag color={actualAudioOutputLabel === 'PortAudio' ? 'success' : 'processing'}>{actualAudioOutputLabel}</Tag></div>
            <div className="desktop-status-line"><span>最终窗口</span><Tag color={diagnosticFresh ? 'success' : 'default'}>{diagnosticFresh ? '已连接' : '未连接'}</Tag></div>
            <div className="desktop-status-line"><span>本地源</span><Tag>{currentSource ? '可用' : '未导入'}</Tag></div>
          </DesktopPanel>
        </DesktopColumn>

        <DesktopColumn area="video" ariaLabel="视频处理与实时参数">
          <DesktopStatusStrip items={statusItems} />
          <DesktopPanel
            title="音视频处理 · 实时参数"
            className="desktop-panel-fill"
            extra={
              <Space size={6} wrap>
                <Tag>全部正式参数</Tag>
                <Button icon={<ReloadOutlined />} size="small" onClick={() => void resetMediaEffectParams()}>重置</Button>
                <Button
                  size="small"
                  onClick={() => void applyMediaProcessing('video')}
                  loading={mediaProcessingBusy === 'video'}
                  disabled={!snapshot?.source_media || !mediaEffectParams || !videoProcessingEnabled || audioProcessingEnabled || mediaProcessingBusy !== null || runtimeResourceBusy}
                >
                  应用视频
                </Button>
                <Button
                  type="primary"
                  size="small"
                  onClick={() => void applyMediaProcessing('both')}
                  loading={mediaProcessingBusy === 'both'}
                  disabled={!snapshot?.source_media || !mediaEffectParams || !videoProcessingEnabled || !audioProcessingEnabled || mediaProcessingBusy !== null || runtimeResourceBusy}
                >
                  音视频同时应用
                </Button>
                <Switch
                  aria-label="视频处理"
                  checked={videoProcessingEnabled}
                  onChange={(checked) => void updateProcessingSwitches({
                    video_processing_enabled: checked,
                    audio_processing_enabled: audioProcessingEnabled,
                    realtime_audio_variant_enabled: false,
                  })}
                />
              </Space>
            }
          >
            {snapshot?.fallback_reason && videoProcessingStatus === 'failed' ? <Alert type="error" showIcon message={snapshot.fallback_reason} style={{ marginBottom: 10 }} /> : null}
            {!mediaEngineCapabilities?.available ? <Alert type="warning" showIcon message={mediaEngineCapabilities?.reason ?? '本地媒体引擎不可用'} style={{ marginBottom: 10 }} /> : null}
            {displayedMediaEffectParams ? (
              <MediaParameterPanels
                value={displayedMediaEffectParams}
                loading={false}
                error={null}
                audioControls={audioParameterControls}
              />
            ) : (
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="正在读取正式媒体参数" />
            )}
            <div className="desktop-inline-between" style={{ marginTop: 12 }}>
              <Typography.Text className="desktop-muted">
                视频周期 {(videoPeriodRange.minMs / 1000).toFixed(0)}–{(videoPeriodRange.maxMs / 1000).toFixed(0)}s · 第 {runtimeCycle} 次变化
              </Typography.Text>
              <Tag color={videoProcessingEnabled ? 'processing' : 'default'}>{getProcessingStatusLabel(videoProcessingStatus)}</Tag>
            </div>
            <Progress
              percent={runtimeProgressPercent}
              showInfo={false}
              status={runtimeActive && nextVideoPlan ? 'active' : 'normal'}
              size="small"
            />
            <div className="desktop-inline-between" style={{ marginTop: 12 }}>
              <Typography.Text className="desktop-muted">
                声音周期 {(audioPeriodRange.minMs / 1000).toFixed(0)}–{(audioPeriodRange.maxMs / 1000).toFixed(0)}s · 第 {audioVariationCycle} 次变化
              </Typography.Text>
              <Tag color={getProcessingStatusColor(audioProcessingStatus)}>{getProcessingStatusLabel(audioProcessingStatus)}</Tag>
            </div>
            <Progress
              percent={audioProgressPercent}
              showInfo={false}
              status={audioPeriodActive && nextAudioPlan ? 'active' : 'normal'}
              size="small"
            />
          </DesktopPanel>
        </DesktopColumn>

        <DesktopColumn area="audio-output" ariaLabel="声音处理与最终输出">
          <DesktopPanel
            title="最终效果窗口"
            className="desktop-output-panel"
            extra={<Button size="small" type="primary" icon={<FolderOpenOutlined />} loading={playerWindowBusy} onClick={() => void openFinalEffectWindowFromHome()}>打开</Button>}
          >
            <Typography.Paragraph className="desktop-muted" style={{ marginBottom: 8 }}>单实例输出，切换参数时保持窗口。</Typography.Paragraph>
            <Space wrap>
              <Tag color={diagnosticFresh ? 'success' : 'default'}>{diagnosticFresh ? '已连接' : '未连接'}</Tag>
              <Button size="small" icon={<PictureOutlined />} disabled={!mediaState || !pictureInPictureSupported} onClick={() => void togglePictureInPicture()}>
                {pictureInPictureActive ? '退出画中画' : '画中画'}
              </Button>
            </Space>
          </DesktopPanel>

          <DesktopPanel title="普通声音处理" className="desktop-audio-panel">
            <div className="desktop-status-line">
              <Space size={6}><AudioOutlined /><Typography.Text>声音处理</Typography.Text></Space>
              <Switch
                aria-label="声音处理"
                checked={audioProcessingEnabled}
                onChange={(checked) => void updateProcessingSwitches({
                  video_processing_enabled: videoProcessingEnabled,
                  audio_processing_enabled: checked,
                  realtime_audio_variant_enabled: false,
                })}
              />
            </div>
            <div className="desktop-status-line"><span>实际出口</span><Tag color={actualAudioOutputLabel === 'PortAudio' ? 'success' : 'processing'}>{actualAudioOutputLabel}</Tag></div>
            <div className="desktop-status-line"><span>处理状态</span><Tag color={getProcessingStatusColor(audioProcessingStatus)}>{getProcessingStatusLabel(audioProcessingStatus)}</Tag></div>
            <div className="desktop-status-line"><span>声音周期</span><span>{(audioPeriodRange.minMs / 1000).toFixed(0)}–{(audioPeriodRange.maxMs / 1000).toFixed(0)}s</span></div>
            <div className="desktop-status-line">
              <span>多轨与预设</span>
              <Typography.Text className="desktop-muted" style={{ minWidth: 0, textAlign: 'right' }}>{actualAudioMixLabel}</Typography.Text>
            </div>
            <div className="desktop-status-line">
              <span>当前预设</span>
              {activeAudioPresets.length > 0 ? (
                <Space aria-label="当前声音预设" size={[4, 4]} wrap style={{ minWidth: 0, flex: '1 1 auto', justifyContent: 'flex-end' }}>
                  {activeAudioPresets.map((preset) => <Tag key={preset.id}>{preset.label}</Tag>)}
                </Space>
              ) : <Tag>等待首轮</Tag>}
            </div>
            <canvas ref={lowFrequencyLineCanvasRef} width={320} height={64} aria-label="声音波形" className="desktop-diagnostic-canvas" />
            <Typography.Text className="desktop-muted">{diagnosticStatus}</Typography.Text>
            <div className="desktop-diagnostic-grid" aria-label="音频只读诊断">
              {[
                ['采样率', diagnosticMessage?.sample_rate_hz === null || diagnosticMessage?.sample_rate_hz === undefined ? '等待' : `${diagnosticMessage.sample_rate_hz} Hz`],
                ['捕获帧', diagnosticMessage?.captured_frame_count === null || diagnosticMessage?.captured_frame_count === undefined ? '等待' : String(diagnosticMessage.captured_frame_count)],
                ['RMS', diagnosticMessage?.rms_dbfs === null || diagnosticMessage?.rms_dbfs === undefined ? '等待' : `${diagnosticMessage.rms_dbfs.toFixed(2)} dBFS`],
                ['峰值', diagnosticMessage?.peak_dbfs === null || diagnosticMessage?.peak_dbfs === undefined ? '等待' : `${diagnosticMessage.peak_dbfs.toFixed(2)} dBFS`],
                ['低频 RMS', diagnosticMessage?.low_band_rms_dbfs === null || diagnosticMessage?.low_band_rms_dbfs === undefined ? '等待' : `${diagnosticMessage.low_band_rms_dbfs.toFixed(2)} dBFS`],
                ['低频截止', diagnosticMessage?.cutoff_hz === null || diagnosticMessage?.cutoff_hz === undefined ? '等待' : `${diagnosticMessage.cutoff_hz.toFixed(0)} Hz`],
                ['噪声底', diagnosticMessage?.noise_floor_dbfs === null || diagnosticMessage?.noise_floor_dbfs === undefined ? '等待' : `${diagnosticMessage.noise_floor_dbfs.toFixed(2)} dBFS`],
                ['SNR', diagnosticMessage?.snr_db === null || diagnosticMessage?.snr_db === undefined ? '等待' : `${diagnosticMessage.snr_db.toFixed(2)} dB`],
                ['F1/F2/F3', diagnosticMessage?.formants_hz.some((value) => value !== null) ? diagnosticMessage.formants_hz.map((value) => value === null ? '—' : value.toFixed(0)).join(' / ') + ' Hz' : '等待'],
                ['MFCC', diagnosticMessage?.mfcc_available ? `${diagnosticMessage.mfcc.length} 维` : '等待'],
              ].map(([label, value]) => (
                <Card className="desktop-diagnostic-item" key={label} size="small" styles={{ body: { padding: '6px 8px' } }}>
                  <Typography.Text className="desktop-diagnostic-label">{label}</Typography.Text>
                  <strong className="desktop-diagnostic-value">{value}</strong>
                </Card>
              ))}
            </div>
            {portAudioFallback ? <Alert type="warning" showIcon message="PortAudio 已回退 WebView" style={{ marginTop: 8 }} /> : null}
            <div className="desktop-control-row" style={{ marginTop: 10 }}>
              <Button size="small" icon={<SettingOutlined />} onClick={() => setAudioSettingsDrawerOpen(true)}>高级声音</Button>
              <Button size="small" icon={<ThunderboltOutlined />} onClick={() => setInterludeDrawerOpen(true)}>插话</Button>
              <Button size="small" icon={<MessageOutlined />} onClick={() => setFixedSpeechDrawerOpen(true)}>固定话术</Button>
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
        </DesktopColumn>
      </DesktopShell>

      <FeatureDrawer
        title="随机插话"
        description="递归扫描本地音频目录及其子目录，并在插话期间自动压低视频原声。"
        width={560}
        open={interludeDrawerOpen}
        onClose={() => setInterludeDrawerOpen(false)}
        summary={(
          <>
            <Tag color={interludeDraft.enabled ? 'success' : 'default'}>{interludeDraft.enabled ? '已启用' : '未启用'}</Tag>
            <Tag color={interludeDirty ? 'warning' : 'blue'}>{interludeDirty ? '有未保存更改' : '配置已同步'}</Tag>
            <Tag>{interludeDraft.directory ? '音频目录已选择' : '未选择音频目录'}</Tag>
            <Tag>{interludeDraft.audioSelectionMode === 'fixed'
              ? `固定 ${interludeDraft.audioFixedPresetId}`
              : interludeDraft.audioMixEnabled
                ? `随机合一 ${interludeDraft.audioMixPickMin}–${interludeDraft.audioMixPickMax} 轨`
                : '随机单轨'}</Tag>
            {interludeDraft.audioSelectionMode === 'random' ? (
              <Tag>{interludeDraft.audioVariationMode === 'periodic' ? '周期换组' : '每次插话换组'}</Tag>
            ) : null}
          </>
        )}
        footer={(
          <Button type="primary" loading={interludeSaving} onClick={() => void saveInterludeConfig()}>
            保存插话配置
          </Button>
        )}
      >
        {interludeSaveError ? <Alert type="error" showIcon message={interludeSaveError} /> : null}
        <FeatureDrawerSection
          title="启用状态"
          description="关闭后保留当前设置，但不会在播放过程中触发插话。"
          extra={<Switch aria-label="启用随机插话" checked={interludeDraft.enabled} onChange={(enabled) => updateInterludeDraft({ enabled })} />}
        >
          {interludePlaybackNotice ? <Alert type="warning" showIcon message={interludePlaybackNotice} /> : (
            <Typography.Text className="desktop-muted">插话按独立随机周期运行，不会改变视频循环进度。</Typography.Text>
          )}
        </FeatureDrawerSection>

        <FeatureDrawerSection title="音频来源" description="选择插话音频根目录，系统会递归扫描其子目录。">
          <FeatureDrawerField label="插话音频目录" htmlFor="interlude-audio-directory">
            <Space.Compact style={{ width: '100%' }}>
              <Input id="interlude-audio-directory" readOnly value={interludeDraft.directory ?? ''} placeholder="请选择音频文件夹" />
              <Button onClick={() => void chooseInterludeDirectory()}>选择音频文件夹</Button>
            </Space.Compact>
          </FeatureDrawerField>
        </FeatureDrawerSection>

        <FeatureDrawerSection title="独立声音轨" description="插话可固定一条预设，或从自己的 22 项预设池随机抽样；配置变化从下一段插话开始生效。">
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

              <FeatureDrawerField label="抽样方式" htmlFor="interlude-audio-variation-mode">
                <Select
                  id="interlude-audio-variation-mode"
                  aria-label="插话声音抽样方式"
                  value={interludeDraft.audioVariationMode}
                  options={[
                    { value: 'each_playback', label: '每次插话重新随机' },
                    { value: 'periodic', label: '按周期更新' },
                  ]}
                  onChange={(audioVariationMode: InterludeAudioVariationMode) => updateInterludeDraft({ audioVariationMode })}
                  style={{ width: '100%' }}
                />
              </FeatureDrawerField>
              {interludeDraft.audioVariationMode === 'periodic' ? (
                <div className="feature-drawer-field-grid feature-drawer-field-grid-compact">
                  <FeatureDrawerField label="变化周期最小值">
                    <CompactNumberField ariaLabel="插话变化周期最小值" unit="ms" min={PERIOD_HARD_MIN_MS} max={PERIOD_HARD_MAX_MS} value={interludeDraft.audioVariationPeriodMinMs} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ audioVariationPeriodMinMs: value })} />
                  </FeatureDrawerField>
                  <FeatureDrawerField label="变化周期最大值">
                    <CompactNumberField ariaLabel="插话变化周期最大值" unit="ms" min={PERIOD_HARD_MIN_MS} max={PERIOD_HARD_MAX_MS} value={interludeDraft.audioVariationPeriodMaxMs} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ audioVariationPeriodMaxMs: value })} />
                  </FeatureDrawerField>
                </div>
              ) : null}
            </>
          )}
          <Alert
            type="info"
            showIcon
            message={actualAudioOutputLabel === 'PortAudio' ? '当前由 PortAudio 应用插话声音预设。' : '当前由 WebView 本地处理插话声音预设。'}
            description="PortAudio 与 WebView 均应用固定或随机预设与多轨合一；WebView 本地处理失败时明确回退插话原声。周期到期只影响下一段，不会重启当前插话。"
          />
        </FeatureDrawerSection>

        <FeatureDrawerSection title="触发与混音" description="间隔决定插话频率；音量与包络决定原声压低和恢复速度。">
          <div className="feature-drawer-parameter-groups">
            <div className="feature-drawer-parameter-group">
              <strong>触发间隔</strong>
              <div className="feature-drawer-field-grid">
                <CompactNumberField ariaLabel="插话最小间隔（秒）" label="最小间隔" unit="秒" step={0.5} min={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.min)} max={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.max)} value={interludeIntervalMsToSeconds(interludeDraft.intervalMinMs)} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMinMs: interludeIntervalSecondsToMs(value) })} />
                <CompactNumberField ariaLabel="插话最大间隔（秒）" label="最大间隔" unit="秒" step={0.5} min={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.min)} max={interludeIntervalMsToSeconds(INTERLUDE_LIMITS.intervalMinMs.max)} value={interludeIntervalMsToSeconds(interludeDraft.intervalMaxMs)} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ intervalMaxMs: interludeIntervalSecondsToMs(value) })} />
              </div>
            </div>
            <div className="feature-drawer-parameter-group">
              <strong>音量与原声压低</strong>
              <div className="feature-drawer-field-grid">
                <CompactNumberField ariaLabel="插话音量" label="插话音量" unit="dB" min={INTERLUDE_LIMITS.volumeDb.min} max={INTERLUDE_LIMITS.volumeDb.max} step={0.5} value={interludeDraft.volumeDb} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ volumeDb: value })} />
                <CompactNumberField ariaLabel="原声压低" label="原声压低" unit="dB" min={INTERLUDE_LIMITS.duckingDepthDb.min} max={INTERLUDE_LIMITS.duckingDepthDb.max} step={0.5} value={interludeDraft.duckingDepthDb} onChange={(value) => typeof value === 'number' && updateInterludeDraft({ duckingDepthDb: value })} />
              </div>
            </div>
            <div className="feature-drawer-parameter-group">
              <strong>过渡时间</strong>
              <div className="feature-drawer-field-grid">
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
          <div className="feature-drawer-footer-split">
            <Button onClick={rerollSubtleAudioParams} disabled={!mediaEffectParams}>重新生成本周期参数</Button>
            <Button
              type="primary"
              onClick={() => void applyMediaProcessing(audioSettingsApplyScope, undefined, true)}
              loading={mediaProcessingBusy === audioSettingsApplyScope}
              disabled={!snapshot?.source_media || !mediaEffectParams || !audioProcessingEnabled || mediaProcessingBusy !== null || runtimeResourceBusy}
            >
              {audioSettingsApplyScope === 'both' ? '音视频同时应用' : '应用声音参数'}
            </Button>
          </div>
        )}
      >
        <FeatureDrawerSection
          title="处理与输出"
          description="普通声音与视频处理开关彼此独立；两者同时开启时会音视频一起应用。"
          extra={<Switch aria-label="高级声音处理" checked={audioProcessingEnabled} onChange={(checked) => void updateProcessingSwitches({ video_processing_enabled: videoProcessingEnabled, audio_processing_enabled: checked, realtime_audio_variant_enabled: false })} />}
        >
          <div className="feature-drawer-status-grid">
            <div><Typography.Text>处理状态</Typography.Text><strong>{getProcessingStatusLabel(audioProcessingStatus)}</strong></div>
            <div><Typography.Text>当前出口</Typography.Text><strong>{actualAudioOutputLabel}</strong></div>
            <div><Typography.Text>混音支路</Typography.Text><strong>{actualAudioMixLabel}</strong></div>
          </div>
        </FeatureDrawerSection>

        {audioOutputBackend?.available ? (
          <FeatureDrawerSection title="PortAudio 设备" description="选择 Host API、输出设备和内存缓冲；播放中修改会立即切换。">
            <div className="feature-drawer-field-grid">
              <FeatureDrawerField label="Host API" htmlFor="portaudio-host-api">
                <Select
                  id="portaudio-host-api"
                  aria-label="PortAudio Host API"
                  value={audioOutputHostApiFilter}
                  options={[
                    { value: 'all', label: '全部 Host API' },
                    { value: 'wasapi', label: 'WASAPI' },
                    { value: 'mme', label: 'MME' },
                    { value: 'dsound', label: 'DirectSound' },
                    { value: 'wdmks', label: 'WDMKS' },
                    { value: 'asio', label: hasAsioOutputDevice ? 'ASIO' : 'ASIO（运行时未检测到设备）', disabled: !hasAsioOutputDevice },
                  ]}
                  onChange={(value) => setAudioOutputHostApiFilter(String(value))}
                  style={{ width: '100%' }}
                />
              </FeatureDrawerField>
              <FeatureDrawerField label="输出设备" htmlFor="portaudio-output-device">
                <Select
                  id="portaudio-output-device"
                  aria-label="PortAudio 输出设备"
                  allowClear
                  placeholder="默认输出设备"
                  value={audioOutputDeviceId ?? undefined}
                  options={audioOutputDevices
                    .filter((device) => audioOutputHostApiFilter === 'all' || device.host_api.trim().toLowerCase() === audioOutputHostApiFilter)
                    .map((device) => ({ value: device.id, label: `${device.name} · ${device.host_api}` }))}
                  onChange={(value) => {
                    const nextId = value === undefined ? null : String(value);
                    setAudioOutputDeviceId(nextId);
                    if (playbackActive && audioProcessingEnabled) void applyAudioOutputBackend(true, nextId, audioOutputMemoryKib);
                  }}
                  style={{ width: '100%' }}
                />
              </FeatureDrawerField>
              <FeatureDrawerField label="内存缓冲" hint="范围 128–2048 KiB，默认 1024 KiB。">
                <CompactNumberField ariaLabel="PortAudio 内存缓冲区大小" unit="KiB" min={PORTAUDIO_MIN_MEMORY_BUFFER_KIB} max={PORTAUDIO_MAX_MEMORY_BUFFER_KIB} value={audioOutputMemoryKibInput} onChange={setAudioOutputMemoryKibInput} onBlur={() => { const value = audioOutputMemoryKibInput; if (typeof value !== 'number' || !Number.isInteger(value) || value < PORTAUDIO_MIN_MEMORY_BUFFER_KIB || value > PORTAUDIO_MAX_MEMORY_BUFFER_KIB) { setAudioOutputMemoryKibInput(audioOutputMemoryKib); return; } setAudioOutputMemoryKib(value); if (playbackActive && audioProcessingEnabled) void applyAudioOutputBackend(true, audioOutputDeviceId, value); }} />
              </FeatureDrawerField>
            </div>
            <Button className="feature-drawer-section-action" loading={audioOutputBusy} disabled={!audioOutputBackend.running || audioOutputBusy} onClick={() => { setAudioOutputBusy(true); void invoke<AudioOutputBackendStatus>('play_portaudio_test_tone', { request: { frequency_hz: 440, duration_ms: 400, amplitude: 0.12 } }).then(publishAudioOutputBackend).catch((cause) => setError(getDisplayErrorMessage(cause, 'PortAudio 测试音失败'))).finally(() => setAudioOutputBusy(false)); }}>播放测试音</Button>
          </FeatureDrawerSection>
        ) : null}

        <FeatureDrawerSection
          title="多轨与预设"
          description="从已勾选预设中抽样；开启多轨后可配置每轮随机合并数量。"
          extra={audioActivePresetIds.length > 0 ? <Button type="link" size="small" aria-label="查看当前声音预设参数" onClick={() => setAudioPresetDrawerOpen(true)}>查看本周期 {audioActivePresetIds.length} 套</Button> : null}
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
          <FeatureDrawerField label="声音参数值预设" hint="1–20 为默认低感知随机池；21、22 为可手动选择的明显效果。">
            <Checkbox.Group aria-label="声音参数值预设" value={audioValuePresetIds} disabled={!audioProcessingEnabled} onChange={(values) => setAudioValuePresetIds(values.map(String))} style={{ width: '100%' }}>
              <div className="feature-drawer-checkbox-grid">{AUDIO_VALUE_PRESETS.map((preset) => <Checkbox key={preset.id} value={preset.id}>{preset.label}</Checkbox>)}</div>
            </Checkbox.Group>
          </FeatureDrawerField>
        </FeatureDrawerSection>

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

      <Drawer title={`当前声音预设${audioVariationCycle > 0 ? `（第 ${audioVariationCycle} 轮）` : ''}`} width={520} open={audioPresetDrawerOpen} onClose={() => setAudioPresetDrawerOpen(false)}>
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
  return (
    <ConfigProvider
      csp={{ nonce: getCspNonce() }}
      theme={isFinalEffectWindow ? undefined : {
        algorithm: antdTheme.darkAlgorithm,
        token: {
          colorPrimary: '#31d7aa',
          colorInfo: '#5ea2ff',
          colorBgBase: '#0b0b0f',
          colorBgContainer: '#17171c',
          colorBorder: '#303139',
          borderRadius: 8,
          fontSize: 12,
          controlHeight: 30,
        },
      }}
    >
      <AntApp>
        {isFinalEffectWindow ? (
          <FinalEffectWindow />
        ) : (
          <HashRouter>
            <ControlPlaneGate>
              <DesktopRouter />
            </ControlPlaneGate>
          </HashRouter>
        )}
      </AntApp>
    </ConfigProvider>
  );
}
