import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { App as AntApp, Alert, Button, Card, Checkbox, ConfigProvider, Descriptions, Input, InputNumber, Layout, Modal, Progress, Select, Slider, Space, Switch, Tag, Typography } from 'antd';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { buildInterludeScheduleKey, chooseInterludeIndex, INTERLUDE_LIMITS, nextInterludeAtMs, resolvePlaybackAudioSource, shouldPauseInterlude } from './插话播放器';
import type { BaseAudioSource } from './插话播放器';
import { buildRuntimePreviewParameters, isRuntimeVariationDue, normalizeRuntimeVariationPeriod } from './运行时参数自动调度';
import type { RuntimeBaseParameters, RuntimePreviewParameters } from './运行时参数自动调度';
import { shouldRestartPlayback } from './播放循环';
import { buildFinalEffectWindowResizeKey } from './最终效果窗口尺寸';
import { clampMediaTime, clampVolume, formatMediaTime, isPlaybackMediaControlMessage, isPlaybackMediaStateMessage, resolvePlaybackPositionMs } from './播放控制消息';
import type { PlaybackMediaControlMessage, PlaybackMediaStateMessage } from './播放控制消息';
import { scheduleAfterInitialPaint, waitForAbortableDelay } from './启动调度';
import { getDisplayErrorMessage } from './errorDisplay';
import {
  isCurrentRuntimeResourceAction,
  isRuntimeResourceBusy,
  isRuntimeResourceRealtimeConsumerBusy,
  isVoiceRuntimeResourceReady,
  resolvePendingRuntimeAction,
  resolveRuntimeResourceClearLifecycle,
  runtimeResourceComponentDescription,
  runtimeResourceConsumerBusyReason,
  runtimeResourceEnsureDecision,
  runtimeResourceMessage,
  runtimeResourcePercent,
  runtimeResourcePollComponent,
  runtimeResourceProgressDetails,
  shouldPollRuntimeResources,
} from './runtimeResources';
import type { RuntimeResourceComponent, RuntimeResourceStatus } from './runtimeResources';
import { addVoiceClonePreset, loadVoiceClonePresets, removeVoiceClonePreset, updateVoiceClonePreset } from './voiceClonePresets';
import type { VoiceClonePreset } from './voiceClonePresets';
import {
  canStartVoiceClonePreGenerationForRuntime,
  getVoiceCloneAutoPrepareKey,
  getVoiceCloneIdleNotice,
  getVoiceClonePreGenerationBusyReason,
  getVoiceClonePreGenerationTriggerKey,
  getVoiceClonePreGenerationItemStatusLabel,
  getVoiceClonePreGenerationSummary,
  getVoiceClonePrepareDisabledReason,
  getVoiceCloneReplaceDisabledReason,
  isVoiceCloneModelLoading,
  shouldAutoReplayVoiceClonePlaybackOnLoop,
  shouldAutoPrepareVoiceCloneSource,
  shouldAcceptVoiceClonePreGenerationResult,
  shouldRestoreVoiceCloneAutoPrepareAfterPicker,
  shouldRetryVoiceClonePreGeneration,
  shouldStartVoiceClonePreGeneration,
  shouldMuteOriginalAudioForVoiceClonePlayback,
} from './voiceCloneStatus';
import './desktop-layout.css';

const PLAYBACK_CHANNEL_NAME = 'autolive-playback-ui-v1';
const VOICE_CLONE_AUTO_PREPARE_DELAY_MS = 4_000;
const RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500;
const DIAGNOSTIC_PUBLISH_INTERVAL_MS = 50;
const DIAGNOSTIC_SAMPLE_COUNT = 128;
const REALTIME_AUDIO_SAFETY_LEAD_MS = 6_000;
const REALTIME_AUDIO_WORKER_TIMEOUT_MS = 5_000;

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
    mp4_hash_status: 'pending' | 'ready' | 'failed';
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
  audio_processing_status: string;
  audio_processing_runtime: boolean;
  audio_processing_gain_db: number;
  voice_clone_replacement: VoiceCloneReplacementState;
  voice_clone_playback: VoiceClonePlaybackState;
  voice_clone_pre_generation: VoiceClonePreGenerationState;
  interlude?: InterludeSnapshot | null;
};

type VoiceClonePlaybackState = {
  status: string;
  phase: string | null;
  progress_percent: number | null;
  progress_message: string | null;
  source_generation: number | null;
  source_path: string | null;
  operation_id: string | null;
  audio_reference: string | null;
  audio_sha256: string | null;
  duration_ms: number | null;
  start_at_ms: number | null;
  input_text: string | null;
  text_sha256: string | null;
  model: string | null;
  error: string | null;
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

type VoiceCloneWorkerCapabilities = SpeechToSpeechWorkerCapabilities;

type VoiceCloneReplacementState = {
  status: string;
  phase: string | null;
  progress_percent: number | null;
  progress_message: string | null;
  source_generation: number | null;
  source_path: string | null;
  operation_id: string | null;
  replacement_audio_reference: string | null;
  replacement_audio_sha256: string | null;
  replacement_duration_ms: number | null;
  replace_at_ms: number | null;
  resume_at_ms: number | null;
  input_text: string | null;
  model: string | null;
  error: string | null;
};

type VoiceClonePreGenerationItemState = {
  preset_id: string;
  text_sha256: string;
  status: 'pending' | 'generating' | 'cached' | 'generated' | 'failed' | 'cancelled' | string;
  audio_sha256: string | null;
  duration_ms: number | null;
  error: string | null;
};

type VoiceClonePreGenerationState = {
  status: 'idle' | 'generating' | 'ready' | 'failed' | 'cancelled' | string;
  batch_id: string | null;
  source_generation: number | null;
  total: number;
  completed: number;
  cache_hits: number;
  generated: number;
  failed: number;
  items: VoiceClonePreGenerationItemState[];
  error: string | null;
};

type SpeechToSpeechStartResult = {
  accepted: boolean;
  snapshot: PlaybackSnapshot;
};

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
    random_change_period_ms: number;
    pitch_shift_semitones: number;
    mfcc_shift_percent: number;
    snr_variation_db: number;
    formant_shift_percent: number;
    filter_q: number;
    loudness_adjustment_db: number;
    input_gain_db: number;
    output_gain_db: number;
    sample_rate_hz: number | null;
    output_bitrate_kbps: number;
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
    frame_rate_jitter_percent: number;
    pixel_scale_percent: number;
    pixel_jitter_px: number;
    dynamic_crop_percent: number;
    space_x_offset_px: number;
    space_y_offset_px: number;
  };
  research: {
    band_weights: Record<string, number>;
    target_frequency_hz: number | null;
    core_frequency_hz: number | null;
    wave_intensity: number;
    wave_level: number;
    frame_perturbation_probability_percent: number;
    abstract_face_count: number;
    slice_length_ms: number;
    slice_min_length_ms: number;
    slice_trigger_interval_ms: number;
  };
};

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
  new URLSearchParams(window.location.search).get('view') === 'final-effect';

function toAssetUrl(path: string | null | undefined) {
  if (!path) return null;
  return convertFileSrc(path.replace(/^file:\/\//, ''));
}

type PlaybackDisplayState = 'loading' | 'no-source' | 'error' | 'disabled' | 'ready' | 'playing' | 'paused' | 'stopped' | 'unknown';

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

function getVoiceCloneTextError(text: string) {
  const trimmed = text.trim();
  if (!trimmed) return '文本不能为空';
  if (countUnicodeCharacters(trimmed) > 500) return '文本最多 500 个字符';
  return null;
}

function getVoiceClonePresetError(title: string, text: string) {
  const trimmedTitle = title.trim();
  if (!trimmedTitle) return '标题不能为空';
  if (countUnicodeCharacters(trimmedTitle) > 80) return '标题最多 80 个字符';
  return getVoiceCloneTextError(text);
}

function toGainValue(db: number) {
  return Math.pow(10, db / 20);
}

function getEffectiveAudioSource(snapshot: PlaybackSnapshot | null): BaseAudioSource {
  return resolvePlaybackAudioSource({
    effectiveAudioSource: snapshot?.effective_audio_source,
    voiceCloneStatus: snapshot?.voice_clone_replacement?.status,
    currentAudioSource: snapshot?.current_audio_source,
    currentVideoSource: snapshot?.current_video_source,
  });
}

function isCurrentVoiceClonePlaybackActive(snapshot: PlaybackSnapshot | null): boolean {
  return shouldMuteOriginalAudioForVoiceClonePlayback(snapshot?.voice_clone_playback?.status ?? 'idle');
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

function getVoiceCloneStatusLabel(status: string) {
  switch (status) {
    case 'preparing':
      return '准备中';
    case 'ready':
      return '已准备';
    case 'generating':
      return '替换生成中';
    case 'playing':
      return '替换播放中';
    case 'failed':
      return '失败';
    case 'cancelled':
      return '已取消';
    default:
      return '未准备';
  }
}

function getVoiceCloneProgress(status: string) {
  switch (status) {
    case 'preparing':
      return 35;
    case 'ready':
      return 100;
    case 'generating':
      return 75;
    case 'playing':
      return 100;
    case 'failed':
    case 'cancelled':
      return 100;
    default:
      return 0;
  }
}

function FinalEffectWindow() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const voiceCloneAudioRef = useRef<HTMLAudioElement | null>(null);
  const interludeAudioRef = useRef<HTMLAudioElement | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const audioContextCleanupTimerRef = useRef<number | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const videoGainNodeRef = useRef<GainNode | null>(null);
  const audioGainNodeRef = useRef<GainNode | null>(null);
  const duckGainNodeRef = useRef<GainNode | null>(null);
  const interludeGainNodeRef = useRef<GainNode | null>(null);
  const suppressMediaEventRef = useRef(false);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const userMutedRef = useRef(false);
  const userVolumeRef = useRef(1);
  const audioUrlRef = useRef<string | null>(null);
  const voiceCloneAudioUrlRef = useRef<string | null>(null);
  const voiceCloneAudioFailureRef = useRef<string | null>(null);
  const currentVoiceCloneAudioPlayingRef = useRef(false);
  const interludeAudioUrlRef = useRef<string | null>(null);
  const audioDiagnosticsReadyRef = useRef(false);
  const realtimeAudioPlayingRef = useRef(false);
  const loopSourceKeyRef = useRef<string | null>(null);
  const loopGenerationRef = useRef<number | null>(null);
  const loopSequenceRef = useRef(0);
  const lastRestartTokenRef = useRef<string | number | null>(null);
  const interludeScheduleKeyRef = useRef<string | null>(null);
  const nextInterludeAtMsRef = useRef<number | null>(null);
  const lastInterludeIndexRef = useRef<number | null>(null);
  const interludeActiveRef = useRef(false);
  const interludePausedRef = useRef(false);
  const interludeStopTimerRef = useRef<number | null>(null);
  const [sourceUrl, setSourceUrl] = useState<string | null>(() => {
    const path = window.localStorage.getItem('autolive.source.path');
    return toAssetUrl(path);
  });
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [voiceCloneAudioUrl, setVoiceCloneAudioUrl] = useState<string | null>(null);
  const [currentVoiceCloneAudioPlaying, setCurrentVoiceCloneAudioPlaying] = useState(false);
  const [interludeAudioUrl, setInterludeAudioUrl] = useState<string | null>(null);
  const [runtimeParameters, setRuntimeParameters] = useState<RuntimePreviewParameters | null>(null);
  const [runtimeAudioProcessingEnabled, setRuntimeAudioProcessingEnabled] = useState(false);
  const [runtimeVideoProcessingEnabled, setRuntimeVideoProcessingEnabled] = useState(false);
  const nextSegmentStartRef = useRef<number | null>(null);
  const scheduleGenerationRef = useRef<number | null>(null);
  const scheduleLoopRef = useRef<number | null>(null);
  const snapshotRef = useRef<PlaybackSnapshot | null>(null);
  const finalEffectResizeKeyRef = useRef<string | null>(null);
  const workerAvailableRef = useRef(false);
  const [audioDiagnosticsReady, setAudioDiagnosticsReady] = useState(false);
  const [realtimeAudioPlaying, setRealtimeAudioPlaying] = useState(false);
  const [userMuted, setUserMuted] = useState(false);
  const [userVolume, setUserVolume] = useState(1);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [finalEffectResizeError, setFinalEffectResizeError] = useState<string | null>(null);

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

  function applyPlayerSnapshot(nextSnapshot: PlaybackSnapshot) {
    snapshotSyncVersionRef.current += 1;
    setSnapshot(nextSnapshot);
  }

  function setRealtimeAudioPlaybackState(playing: boolean) {
    realtimeAudioPlayingRef.current = playing;
    setRealtimeAudioPlaying(playing);
    syncUserAudioSettings();
  }

  function syncUserAudioSettings() {
    const video = videoRef.current;
    const audio = audioRef.current;
    const voiceCloneAudio = voiceCloneAudioRef.current;
    const interludeAudio = interludeAudioRef.current;
    const volume = userVolumeRef.current;
    const muted = userMutedRef.current;
    const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
    const currentVoiceClonePlaybackActive =
      currentVoiceCloneAudioPlayingRef.current && isCurrentVoiceClonePlaybackActive(snapshotRef.current);
    const replacementAudioActive =
      currentVoiceClonePlaybackActive ||
      (audioDiagnosticsReadyRef.current &&
        effectiveAudioSource === 'realtime_variant' &&
        realtimeAudioPlayingRef.current);
    if (video) {
      video.volume = volume;
      video.muted = replacementAudioActive || muted;
    }
    if (audio) {
      audio.volume = volume;
      audio.muted = muted || currentVoiceClonePlaybackActive;
    }
    if (voiceCloneAudio) {
      voiceCloneAudio.volume = volume;
      voiceCloneAudio.muted = muted;
    }
    if (interludeAudio) {
      interludeAudio.volume = volume;
      interludeAudio.muted = muted;
    }
  }

  function clearInterludeStopTimer() {
    if (interludeStopTimerRef.current !== null) {
      window.clearTimeout(interludeStopTimerRef.current);
      interludeStopTimerRef.current = null;
    }
  }

  function rampGain(node: GainNode | null, target: number, durationMs: number) {
    const context = audioContextRef.current;
    if (!context || !node) return;
    const now = context.currentTime;
    const durationSeconds = Math.max(0, durationMs) / 1000;
    node.gain.cancelScheduledValues(now);
    node.gain.setValueAtTime(node.gain.value, now);
    node.gain.linearRampToValueAtTime(target, now + durationSeconds);
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
    if (resetSchedule) nextInterludeAtMsRef.current = null;
    if (resetIndex) lastInterludeIndexRef.current = null;
    rampGain(interludeGainNodeRef.current, 0, releaseMs);
    rampGain(duckGainNodeRef.current, 1, releaseMs);
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
    if (!interludeActiveRef.current || !interludePausedRef.current || !interludeAudioUrlRef.current) return;
    interludePausedRef.current = false;
    void interludeAudioRef.current?.play().catch(() => undefined);
  }

  function clearVoiceCloneAudioSource() {
    const voiceCloneAudio = voiceCloneAudioRef.current;
    if (voiceCloneAudio) {
      voiceCloneAudio.pause();
      voiceCloneAudio.currentTime = 0;
      voiceCloneAudio.removeAttribute('src');
      voiceCloneAudio.load();
    }
    voiceCloneAudioUrlRef.current = null;
    currentVoiceCloneAudioPlayingRef.current = false;
    setCurrentVoiceCloneAudioPlaying(false);
    setVoiceCloneAudioUrl(null);
    syncUserAudioSettings();
  }

  function applyVoiceCloneAudioFailureRecovery(nextSnapshot: PlaybackSnapshot, message: string) {
    clearVoiceCloneAudioSource();
    applyPlayerSnapshot(nextSnapshot);
    setPlaybackError(message);
  }

  function handleVoiceCloneAudioError(cause: unknown) {
    const currentPlayback = snapshotRef.current?.voice_clone_playback;
    const operationId = currentPlayback?.operation_id;
    if (currentPlayback?.status !== 'playing' || !operationId) return;
    if (voiceCloneAudioFailureRef.current === operationId) return;
    voiceCloneAudioFailureRef.current = operationId;
    currentVoiceCloneAudioPlayingRef.current = false;
    setCurrentVoiceCloneAudioPlaying(false);
    syncUserAudioSettings();
    const reason = getDisplayErrorMessage(cause, '当前文案音频加载或播放失败');
    setPlaybackError(`${reason}，正在恢复原音轨。`);

    void invoke<PlaybackSnapshot>('fail_voice_clone_playback', {
      request: { operation_id: operationId, reason },
    })
      .then((nextSnapshot) => {
        applyVoiceCloneAudioFailureRecovery(
          nextSnapshot,
          `${reason}，已恢复原音轨。请检查音频缓存后重新播放。`,
        );
      })
      .catch((failureCause) => {
        // fail IPC 运行在最终效果窗口；失败时只清理本地媒体，避免调用要求主窗口的命令。
        clearVoiceCloneAudioSource();
        setPlaybackError(
          `${reason}，原音轨恢复失败：${getDisplayErrorMessage(
            failureCause,
            '请回到主窗口点击“清空当前替换”后重试。',
          )}`,
        );
      })
      .finally(() => {
        if (voiceCloneAudioFailureRef.current === operationId) {
          voiceCloneAudioFailureRef.current = null;
        }
      });
  }

  function handleVoiceCloneAudioElementError(event: SyntheticEvent<HTMLAudioElement>) {
    const detail = event.currentTarget.error?.message;
    handleVoiceCloneAudioError(detail ? new Error(detail) : '当前文案音频文件无法加载');
  }

  function playVoiceCloneAudio(audio: HTMLAudioElement) {
    void audio.play().catch((cause) => handleVoiceCloneAudioError(cause));
  }

  function handleVoiceCloneAudioPlaying() {
    if (!isCurrentVoiceClonePlaybackActive(snapshotRef.current)) return;
    currentVoiceCloneAudioPlayingRef.current = true;
    setCurrentVoiceCloneAudioPlaying(true);
    syncUserAudioSettings();
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
    const interlude = snapshotRef.current?.interlude ?? null;
    clearInterludeStopTimer();
    interludeActiveRef.current = false;
    interludePausedRef.current = false;
    interludeAudioUrlRef.current = null;
    setInterludeAudioUrl(null);
    rampGain(interludeGainNodeRef.current, 0, interlude?.ducking_release_ms ?? 0);
    rampGain(duckGainNodeRef.current, 1, interlude?.ducking_release_ms ?? 0);
    nextInterludeAtMsRef.current = null;
  }

  function handleInterludeError(event: SyntheticEvent<HTMLAudioElement>) {
    if (!interludeActiveRef.current) return;
    const detail = event.currentTarget.error?.message;
    setPlaybackError(detail ? `插话音频播放失败：${detail}` : '插话音频播放失败，请检查文件格式和文件权限。');
    handleInterludeEnded();
  }

  function handleVoiceCloneAudioEnded() {
    const currentPlayback = snapshotRef.current?.voice_clone_playback;
    if (currentPlayback?.status !== 'playing' || !currentPlayback.operation_id) return;
    currentVoiceCloneAudioPlayingRef.current = false;
    setCurrentVoiceCloneAudioPlaying(false);
    syncUserAudioSettings();
    void invoke<PlaybackSnapshot>('finish_voice_clone_playback', {
      request: { operation_id: currentPlayback.operation_id },
    })
      .then(applyPlayerSnapshot)
      .catch((cause) => {
        setPlaybackError(getDisplayErrorMessage(cause, '当前文案播放结束状态同步失败。'));
      });
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
    interludeAudioUrlRef.current = selectedUrl;
    setInterludeAudioUrl(selectedUrl);
    rampGain(interludeGainNodeRef.current, toGainValue(interlude.volume_db), interlude.ducking_attack_ms);
    rampGain(duckGainNodeRef.current, toGainValue(interlude.ducking_depth_db), interlude.ducking_attack_ms);
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
      const voiceCloneAudio = voiceCloneAudioRef.current;
      if (audio) {
        audio.currentTime = Math.max(0, video.currentTime - (snapshotRef.current?.current_audio_start_at_ms ?? 0) / 1000);
      }
      if (voiceCloneAudio && !isCurrentVoiceClonePlaybackActive(snapshotRef.current)) {
        voiceCloneAudio.currentTime = clampMediaTime(video.currentTime, voiceCloneAudio.duration);
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
      if (isPlaybackMediaControlMessage(event.data)) {
        void applyPlaybackMediaControl(event.data);
        return;
      }
      if (isPlaybackControlMessage(event.data)) {
        const video = videoRef.current;
        const audio = audioRef.current;
        const voiceCloneAudio = voiceCloneAudioRef.current;
        if (event.data.action === 'stop') {
          video?.pause();
          if (video) video.currentTime = 0;
          audio?.pause();
          if (audio) audio.currentTime = 0;
          voiceCloneAudio?.pause();
          if (voiceCloneAudio) voiceCloneAudio.currentTime = 0;
        } else if (event.data.action === 'pause') {
          video?.pause();
          audio?.pause();
          voiceCloneAudio?.pause();
        } else if (video) {
          if (event.data.action === 'resume') resumeAudioDiagnostics();
          void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
          const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
          if (isCurrentVoiceClonePlaybackActive(snapshotRef.current) && voiceCloneAudioUrlRef.current && voiceCloneAudio) {
            playVoiceCloneAudio(voiceCloneAudio);
          } else if (effectiveAudioSource === 'voice_clone' && voiceCloneAudioUrlRef.current && voiceCloneAudio) {
            playVoiceCloneAudio(voiceCloneAudio);
          } else if (effectiveAudioSource === 'realtime_variant' && audioUrlRef.current && audioDiagnosticsReadyRef.current && audio) {
            void audio.play().catch(() => undefined);
          }
        }
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
      setRuntimeAudioProcessingEnabled(event.data.audio_processing_enabled);
      setRuntimeVideoProcessingEnabled(event.data.video_processing_enabled);
    };
    channel.addEventListener('message', handleMessage);
    return () => {
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
      const version = snapshotSyncVersionRef.current;
      void invoke<PlaybackSnapshot>('get_snapshot').then((nextSnapshot) => {
        if (version === snapshotSyncVersionRef.current) applyPlayerSnapshot(nextSnapshot);
      }).catch(() => undefined);
    };
    refreshSnapshot();
    const timer = window.setInterval(refreshSnapshot, 500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (audioContextCleanupTimerRef.current !== null) {
      window.clearTimeout(audioContextCleanupTimerRef.current);
      audioContextCleanupTimerRef.current = null;
    }

    const scheduleAudioContextCleanup = () => {
      if (audioContextCleanupTimerRef.current !== null) return;
      audioContextCleanupTimerRef.current = window.setTimeout(() => {
        const context = audioContextRef.current;
        audioContextRef.current = null;
        analyserRef.current = null;
        videoGainNodeRef.current = null;
        audioGainNodeRef.current = null;
        duckGainNodeRef.current = null;
        interludeGainNodeRef.current = null;
        audioDiagnosticsReadyRef.current = false;
        realtimeAudioPlayingRef.current = false;
        setAudioDiagnosticsReady(false);
        setRealtimeAudioPlaying(false);
        void context?.close().catch(() => undefined);
        audioContextCleanupTimerRef.current = null;
      }, 0);
    };

    if (!sourceUrl || !videoRef.current || !audioRef.current || !interludeAudioRef.current) {
      if (audioContextRef.current) return scheduleAudioContextCleanup;
      return;
    }
    if (audioContextRef.current) return scheduleAudioContextCleanup;

    let context: AudioContext | null = null;
    try {
      context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 2_048;
      analyser.smoothingTimeConstant = 0.75;
      const videoSource = context.createMediaElementSource(videoRef.current);
      const audioSource = context.createMediaElementSource(audioRef.current);
      const interludeAudioSource = context.createMediaElementSource(interludeAudioRef.current);
      const videoGain = context.createGain();
      const audioGain = context.createGain();
      const baseBus = context.createGain();
      const duckGain = context.createGain();
      const interludeGain = context.createGain();
      const interludeBus = context.createGain();
      duckGain.gain.value = 1;
      interludeGain.gain.value = 0;
      videoSource.connect(videoGain).connect(baseBus);
      audioSource.connect(audioGain).connect(baseBus);
      interludeAudioSource.connect(interludeGain).connect(interludeBus);
      baseBus.connect(duckGain).connect(analyser);
      interludeBus.connect(analyser);
      analyser.connect(context.destination);
      audioContextRef.current = context;
      analyserRef.current = analyser;
      videoGainNodeRef.current = videoGain;
      audioGainNodeRef.current = audioGain;
      duckGainNodeRef.current = duckGain;
      interludeGainNodeRef.current = interludeGain;
      audioDiagnosticsReadyRef.current = true;
      setAudioDiagnosticsReady(true);
    } catch (cause) {
      void context?.close().catch(() => undefined);
      setAudioDiagnosticsReady(false);
      setPlaybackError(cause instanceof Error ? `音频混音初始化失败：${cause.message}` : '音频混音初始化失败');
    }

    return scheduleAudioContextCleanup;
  }, [sourceUrl]);

  useEffect(() => {
    const gainDb = runtimeAudioProcessingEnabled && runtimeParameters
      ? runtimeParameters.audio_gain_db
      : snapshot?.audio_processing_runtime
        ? snapshot.audio_processing_gain_db ?? 0
        : 0;
    const gain = toGainValue(gainDb);
    const effectiveAudioSource = getEffectiveAudioSource(snapshot);
    const currentVoiceClonePlaybackActive =
      currentVoiceCloneAudioPlaying && isCurrentVoiceClonePlaybackActive(snapshot);
    if (videoGainNodeRef.current) {
      videoGainNodeRef.current.gain.value =
        audioDiagnosticsReady &&
        !currentVoiceClonePlaybackActive &&
        (effectiveAudioSource !== 'realtime_variant' || !realtimeAudioPlaying) ? gain : 0;
    }
    if (audioGainNodeRef.current) {
      audioGainNodeRef.current.gain.value =
        audioDiagnosticsReady &&
        !currentVoiceClonePlaybackActive &&
        effectiveAudioSource === 'realtime_variant' &&
        realtimeAudioPlaying ? gain : 0;
    }
  }, [
    audioDiagnosticsReady,
    currentVoiceCloneAudioPlaying,
    realtimeAudioPlaying,
    runtimeAudioProcessingEnabled,
    runtimeParameters?.audio_gain_db,
    snapshot?.audio_processing_gain_db,
    snapshot?.audio_processing_runtime,
    snapshot?.current_audio_source,
    snapshot?.effective_audio_source,
    snapshot?.voice_clone_replacement?.status,
    snapshot?.voice_clone_playback?.status,
  ]);

  useEffect(() => {
    void invoke<SpeechToSpeechWorkerCapabilities>('get_speech_to_speech_worker_capabilities')
      .then(setWorkerCapabilities)
      .catch(() => setWorkerCapabilities(null));
  }, []);

  useEffect(() => {
    const path =
      snapshot?.current_video_reference ??
      snapshot?.source_media?.source_path ??
      window.localStorage.getItem('autolive.source.path');
    const nextUrl = toAssetUrl(path);
    if (nextUrl) setSourceUrl(nextUrl);
  }, [snapshot?.current_video_reference, snapshot?.source_media?.source_path]);

  useEffect(() => {
    const source = snapshot?.source_media;
    const resizeKey = buildFinalEffectWindowResizeKey({
      width: source?.width,
      height: source?.height,
      sourcePath: source?.source_path,
      videoReference: snapshot?.current_video_reference,
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
    snapshot?.current_video_reference,
    snapshot?.source_media?.height,
    snapshot?.source_media?.source_path,
    snapshot?.source_media?.width,
  ]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !sourceUrl) return;
    video.currentTime = 0;
    if (snapshot?.playback_state === 'playing') {
      void video.play().catch(() => undefined);
    }
  }, [sourceUrl]);

  useEffect(() => {
    const video = videoRef.current;
    const audio = audioRef.current;
    const voiceCloneAudio = voiceCloneAudioRef.current;
    const interlude = snapshot?.interlude ?? null;
    if (!video || !sourceUrl) return;
    if (snapshot?.playback_state === 'stopped') {
      video.pause();
      video.currentTime = 0;
      audio?.pause();
      if (audio) audio.currentTime = 0;
      voiceCloneAudio?.pause();
      if (voiceCloneAudio) voiceCloneAudio.currentTime = 0;
      clearInterludePlayback({ releaseMs: interlude?.ducking_release_ms ?? 0, resetSchedule: true, resetIndex: true });
      return;
    }
    if (snapshot?.playback_state === 'paused') {
      video.pause();
      audio?.pause();
      voiceCloneAudio?.pause();
      pauseInterludePlayback();
      return;
    }
    if (snapshot?.playback_state === 'playing' && video.paused) {
      void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
      const effectiveAudioSource = getEffectiveAudioSource(snapshotRef.current);
      if (isCurrentVoiceClonePlaybackActive(snapshotRef.current) && voiceCloneAudioUrl && voiceCloneAudio) {
        playVoiceCloneAudio(voiceCloneAudio);
      } else if (effectiveAudioSource === 'voice_clone' && voiceCloneAudioUrl && voiceCloneAudio) {
        playVoiceCloneAudio(voiceCloneAudio);
      } else if (effectiveAudioSource === 'realtime_variant' && audioUrl && audioDiagnosticsReady && audio) {
        void audio.play().catch(() => undefined);
      }
      resumeInterludePlayback();
    }
  }, [audioDiagnosticsReady, audioUrl, snapshot?.interlude, snapshot?.playback_state, snapshot?.effective_audio_source, snapshot?.current_audio_source, snapshot?.voice_clone_replacement?.status, snapshot?.voice_clone_playback?.status, sourceUrl, voiceCloneAudioUrl]);

  useEffect(() => {
    if (!snapshot || !sourceUrl) return;
    const reference = getEffectiveAudioSource(snapshot) === 'realtime_variant'
      ? snapshot.current_audio_reference
      : null;
    setAudioUrl(reference ? convertFileSrc(reference.replace(/^file:\/\//, '')) : null);
  }, [snapshot?.current_audio_reference, snapshot?.current_audio_source, snapshot?.effective_audio_source, snapshot?.voice_clone_replacement?.status, sourceUrl]);

  useEffect(() => {
    if (!snapshot || !sourceUrl) {
      setVoiceCloneAudioUrl(null);
      return;
    }
    const replacement = snapshot.voice_clone_replacement;
    const currentTextPlayback = snapshot.voice_clone_playback;
    const replacementReference =
      replacement.status === 'playing' &&
      replacement.source_generation === snapshot.playback_generation &&
      replacement.source_path === snapshot.source_media?.source_path
        ? replacement.replacement_audio_reference
        : null;
    const currentTextReference =
      currentTextPlayback.status === 'playing' &&
      currentTextPlayback.source_generation === snapshot.playback_generation &&
      currentTextPlayback.source_path === snapshot.source_media?.source_path
        ? currentTextPlayback.audio_reference
        : null;
    const activeReference = currentTextPlayback.status === 'preparing'
      ? null
      : currentTextReference ?? replacementReference;
    setVoiceCloneAudioUrl(activeReference ? convertFileSrc(activeReference.replace(/^file:\/\//, '')) : null);
  }, [
    snapshot?.playback_generation,
    snapshot?.source_media?.source_path,
    snapshot?.voice_clone_replacement,
    snapshot?.voice_clone_playback,
    sourceUrl,
  ]);

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
  }, [audioDiagnosticsReady, audioUrl, snapshot?.current_audio_start_at_ms, snapshot?.current_audio_source, snapshot?.effective_audio_source, snapshot?.voice_clone_replacement?.status]);

  useEffect(() => {
    const video = videoRef.current;
    const voiceCloneAudio = voiceCloneAudioRef.current;
    if (!video || !voiceCloneAudio) return;
    voiceCloneAudioUrlRef.current = voiceCloneAudioUrl;
    if (!voiceCloneAudioUrl) {
      voiceCloneAudio.pause();
      voiceCloneAudio.currentTime = 0;
      voiceCloneAudio.removeAttribute('src');
      voiceCloneAudio.load();
      currentVoiceCloneAudioPlayingRef.current = false;
      setCurrentVoiceCloneAudioPlaying(false);
      syncUserAudioSettings();
      return;
    }
    voiceCloneAudio.src = voiceCloneAudioUrl;
    voiceCloneAudio.currentTime = isCurrentVoiceClonePlaybackActive(snapshotRef.current)
      ? 0
      : clampMediaTime(video.currentTime, voiceCloneAudio.duration);
    syncUserAudioSettings();
    if (isCurrentVoiceClonePlaybackActive(snapshotRef.current)) {
      playVoiceCloneAudio(voiceCloneAudio);
    } else if (!video.paused) {
      playVoiceCloneAudio(voiceCloneAudio);
    }
  }, [voiceCloneAudioUrl, snapshot?.voice_clone_playback?.status]);

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
        voiceCloneStatus: currentSnapshot.voice_clone_replacement.status,
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
        voiceCloneStatus: currentSnapshot.voice_clone_replacement.status,
        currentTextStatus: currentSnapshot.voice_clone_playback.status,
      })) {
        if (currentSnapshot.playback_state === 'paused') {
          pauseInterludePlayback();
          return;
        }
        clearInterludePlayback({
          releaseMs: interlude.ducking_release_ms,
          resetSchedule: true,
        });
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

  const currentSourceKey =
    snapshot?.current_video_reference ?? snapshot?.source_media?.source_path ?? sourceUrl ?? null;

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
    video.currentTime = 0;
    if (audioRef.current) audioRef.current.currentTime = 0;
    const currentTextPlaybackActive = isCurrentVoiceClonePlaybackActive(snapshotRef.current);
    if (voiceCloneAudioRef.current && shouldAutoReplayVoiceClonePlaybackOnLoop()) {
      voiceCloneAudioRef.current.pause();
      voiceCloneAudioRef.current.currentTime = 0;
    } else if (voiceCloneAudioRef.current && !currentTextPlaybackActive) {
      voiceCloneAudioRef.current.pause();
      voiceCloneAudioRef.current.currentTime = 0;
    }
    suppressMediaEventRef.current = true;
    void video.play().catch(() => {
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
      ? { type: 'info' as const, message: '尚未导入视频，请先回到主页导入一个 MP4。' }
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
                src={sourceUrl}
                autoPlay
                playsInline
                muted={
                  currentVoiceCloneAudioPlaying ||
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
                  if (snapshot?.playback_state === 'paused' || snapshot?.playback_state === 'ready') {
                    void invoke<PlaybackSnapshot>('resume_playback').then(applyPlayerSnapshot).catch(() => undefined);
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
                src={audioUrl ?? undefined}
                preload="auto"
                muted={userMuted}
                onPlaying={handleRealtimeAudioPlaying}
                onEnded={handleRealtimeAudioEnded}
                onError={handleRealtimeAudioElementError}
                hidden
              />
              <audio
                ref={voiceCloneAudioRef}
                src={voiceCloneAudioUrl ?? undefined}
                preload="auto"
                muted={userMuted}
                onPlaying={handleVoiceCloneAudioPlaying}
                onEnded={handleVoiceCloneAudioEnded}
                onError={handleVoiceCloneAudioElementError}
                hidden
              />
              <audio
                ref={interludeAudioRef}
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
  const [runtimeResourcePollError, setRuntimeResourcePollError] = useState<string | null>(null);
  const [playbackActionBusy, setPlaybackActionBusy] = useState<'pause' | 'resume' | 'stop' | null>(null);
  const [videoProcessingEnabled, setVideoProcessingEnabled] = useState(false);
  const [audioProcessingEnabled, setAudioProcessingEnabled] = useState(false);
  const [realtimeAudioVariantEnabled, setRealtimeAudioVariantEnabled] = useState(false);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [voiceCloneWorkerCapabilities, setVoiceCloneWorkerCapabilities] = useState<VoiceCloneWorkerCapabilities | null>(null);
  const [voiceClonePresets, setVoiceClonePresets] = useState<VoiceClonePreset[]>(() => loadVoiceClonePresets());
  const [selectedVoiceClonePresetId, setSelectedVoiceClonePresetId] = useState<string | null>(null);
  const [voiceClonePresetTitle, setVoiceClonePresetTitle] = useState('');
  const [voiceCloneText, setVoiceCloneText] = useState('');
  const [voiceClonePresetTextRevision, setVoiceClonePresetTextRevision] = useState(0);
  const [voiceClonePreGenerationCompletionVersion, setVoiceClonePreGenerationCompletionVersion] = useState(0);
  const [voiceCloneFormError, setVoiceCloneFormError] = useState<string | null>(null);
  const [voiceCloneActionBusy, setVoiceCloneActionBusy] = useState<'prepare' | 'play' | 'cancel' | 'clear' | null>(null);
  const [voiceCloneAutoPreparePhase, setVoiceCloneAutoPreparePhase] = useState<'delay' | null>(null);
  const [interludeDraft, setInterludeDraft] = useState<InterludeConfigDraft>(() => buildInterludeDraft());
  const [interludeDirty, setInterludeDirty] = useState(false);
  const [interludeSaving, setInterludeSaving] = useState(false);
  const [mediaEngineCapabilities, setMediaEngineCapabilities] = useState<MediaEngineCapabilities | null>(null);
  const [researchWorkerCapabilities, setResearchWorkerCapabilities] = useState<ResearchWorkerCapabilities | null>(null);
  const [researchStatus, setResearchStatus] = useState<ResearchStatus | null>(null);
  const [researchParams, setResearchParams] = useState<ResearchParams | null>(null);
  const [researchValidation, setResearchValidation] = useState<ParameterValidationError[]>([]);
  const [researchValidationStatus, setResearchValidationStatus] = useState<'idle' | 'valid' | 'invalid'>('idle');
  const [cacheCleanup, setCacheCleanup] = useState<CacheCleanupResult | null>(null);
  const [mediaProcessingBusy, setMediaProcessingBusy] = useState(false);
  const [researchActionBusy, setResearchActionBusy] = useState(false);
  const [researchCancelBusy, setResearchCancelBusy] = useState(false);
  const [cacheCleanupBusy, setCacheCleanupBusy] = useState(false);
  const snapshotRequestRef = useRef(0);
  const importVideoInFlightRef = useRef(false);
  const pendingRuntimeActionRef = useRef<PendingRuntimeAction | null>(null);
  const runtimeResourceActionTokenRef = useRef(0);
  const runtimeResourcePollGenerationRef = useRef(0);
  const runtimeResourceMountedRef = useRef(true);
  const runtimeResourceClearInFlightRef = useRef(false);
  const runtimeResourceBusyRef = useRef(false);
  const resourceConsumersBusyReasonRef = useRef<string | null>(null);
  const voiceRuntimeReadyRef = useRef(false);
  const sourceRestoreAttemptedRef = useRef(false);
  const voiceCloneAutoPrepareKeyRef = useRef<string | null>(null);
  const voiceCloneAutoPrepareControllerRef = useRef<AbortController | null>(null);
  const voiceClonePrepareInFlightRef = useRef(false);
  const voiceClonePreGenerationInFlightRef = useRef(false);
  const voiceClonePreGenerationStartedKeyRef = useRef<string | null>(null);
  const voiceClonePreGenerationRetriedKeyRef = useRef<string | null>(null);
  const voiceClonePreGenerationMountedRef = useRef(true);
  const voiceClonePreGenerationCurrentGenerationRef = useRef<number | null>(null);
  const voiceClonePreGenerationCurrentKeyRef = useRef<string | null>(null);
  const playbackActionRequestRef = useRef(0);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const pictureInPictureVideoRef = useRef<HTMLVideoElement | null>(null);
  const runtimeMessageRef = useRef<RuntimeParameterMessage | null>(null);
  const runtimeSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
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
    if (components.includes('voice')) {
      void invoke<VoiceCloneWorkerCapabilities>('get_voice_clone_worker_capabilities')
        .then((capabilities) => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            if (capabilities.available) voiceRuntimeReadyRef.current = true;
            setVoiceCloneWorkerCapabilities(capabilities);
          }
        })
        .catch(() => {
          if (
            runtimeResourceMountedRef.current
            && runtimeResourceActionTokenRef.current === expectedActionToken
          ) {
            setVoiceCloneWorkerCapabilities(null);
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
    setRuntimeResourcePollError(null);
    runtimeResourceBusyRef.current = isRuntimeResourceBusy(nextStatus);
    const clearWasInFlight = runtimeResourceClearInFlightRef.current;
    const clearLifecycle = resolveRuntimeResourceClearLifecycle(
      clearWasInFlight,
      nextStatus,
    );
    if (
      !clearWasInFlight
      && nextStatus.component === null
      && pendingRuntimeActionRef.current === null
    ) {
      runtimeResourceActionTokenRef.current += 1;
    }
    runtimeResourceClearInFlightRef.current = clearLifecycle.inFlight;
    if (
      clearLifecycle.inFlight
      || clearLifecycle.terminalAction === 'clear-capabilities'
      || clearLifecycle.terminalAction === 'revalidate-capabilities'
    ) {
      setMediaEngineCapabilities(null);
      setVoiceCloneWorkerCapabilities(null);
      voiceRuntimeReadyRef.current = false;
    }
    const capabilityToken = runtimeResourceActionTokenRef.current;
    if (clearLifecycle.terminalAction === 'revalidate-capabilities') {
      refreshRuntimeResourceCapabilities(['media', 'voice'], capabilityToken);
    }
    if (clearLifecycle.terminalAction === 'conflict') {
      setError('运行资源清理未启动，另一项资源操作刚刚完成，请重试。');
      refreshRuntimeResourceCapabilities(['media', 'voice'], capabilityToken);
    }
    if (
      clearLifecycle.terminalAction !== 'conflict'
      && nextStatus.state === 'ready'
      && nextStatus.component === 'media'
    ) {
      refreshRuntimeResourceCapabilities(['media'], capabilityToken);
    }
    if (
      clearLifecycle.terminalAction !== 'conflict'
      && nextStatus.state === 'ready'
      && nextStatus.component === 'voice'
    ) {
      voiceRuntimeReadyRef.current = true;
      refreshRuntimeResourceCapabilities(['voice'], capabilityToken);
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
        setError(getDisplayErrorMessage(cause, '运行资源就绪后的操作恢复失败'));
      }
    }
  }, [refreshRuntimeResourceCapabilities]);

  const ensureRuntimeResources = useCallback(async (
    component: RuntimeResourceComponent,
    resume: () => Promise<void>,
  ) => {
    if (runtimeResourceClearInFlightRef.current) {
      throw new RuntimeResourceConflictError('运行资源正在清理，请等待完成后再试');
    }
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
        throw new RuntimeResourceConflictError('另一项运行资源操作正在执行，请稍后再试');
      }
      const installingStatus = await invoke<RuntimeResourceStatus>('install_runtime_resources', { component });
      await applyRuntimeResourceStatus(installingStatus, token);
      const installingDecision = runtimeResourceEnsureDecision(component, installingStatus);
      if (installingDecision === 'conflict') {
        pendingRuntimeActionRef.current = null;
        throw new RuntimeResourceConflictError('另一项运行资源操作正在执行，请稍后再试');
      }
      if (installingDecision === 'install') {
        pendingRuntimeActionRef.current = null;
        throw new Error(installingStatus.error || '运行资源安装未启动，请重试');
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
        error: getDisplayErrorMessage(cause, '运行资源安装失败'),
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
            setRuntimeResourcePollError(getDisplayErrorMessage(cause, '读取运行资源状态失败，将自动重试'));
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

  useLayoutEffect(() => {
    voiceClonePreGenerationCurrentGenerationRef.current = snapshot?.playback_generation ?? null;
  }, [snapshot?.playback_generation]);

  useLayoutEffect(() => {
    voiceClonePreGenerationCurrentKeyRef.current = getVoiceClonePreGenerationTriggerKey(
      snapshot?.playback_generation,
      voiceClonePresetTextRevision,
    );
  }, [snapshot?.playback_generation, voiceClonePresetTextRevision]);

  useEffect(() => {
    voiceClonePreGenerationMountedRef.current = true;
    return () => {
      voiceClonePreGenerationMountedRef.current = false;
      voiceCloneAutoPrepareControllerRef.current?.abort();
    };
  }, []);

  const [runtimeCycle, setRuntimeCycle] = useState(0);
  const [runtimeLastChangeMs, setRuntimeLastChangeMs] = useState<number | null>(null);
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
      if (!isDiagnosticMessage(event.data)) return;
      setDiagnosticMessage(event.data);
    };
    channel.addEventListener('message', handleMessage);
    return () => {
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

  useEffect(() => {
    let cancelled = false;
    const refreshSnapshot = async () => {
      const requestId = ++snapshotRequestRef.current;
      setSnapshotLoading(true);
      try {
        const nextSnapshot = await invoke<PlaybackSnapshot>('get_snapshot');
        if (cancelled || requestId !== snapshotRequestRef.current) return;
        setSnapshot(nextSnapshot);
        setSnapshotFetchError(null);
        if (!nextSnapshot.source_media && !sourceRestoreAttemptedRef.current) {
          const storedSourcePath = window.localStorage.getItem('autolive.source.path')?.trim();
          if (storedSourcePath) {
            sourceRestoreAttemptedRef.current = true;
            void importVideo(storedSourcePath);
          }
        }
      } catch (cause) {
        if (cancelled || requestId !== snapshotRequestRef.current) return;
        setSnapshotFetchError(cause instanceof Error ? cause.message : '读取播放状态失败');
      } finally {
        if (!cancelled && requestId === snapshotRequestRef.current) {
          setSnapshotLoading(false);
        }
      }
    };
    void refreshSnapshot();
    const timer = window.setInterval(() => {
      void refreshSnapshot();
    }, 500);
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
    const cancelIdleWork = scheduleAfterInitialPaint(() => {
      if (cancelled) return;
      const capabilityToken = runtimeResourceActionTokenRef.current;
      if (
        !runtimeResourceBusyRef.current
        && !runtimeResourceClearInFlightRef.current
        && pendingRuntimeActionRef.current === null
      ) {
        refreshRuntimeResourceCapabilities(['media', 'voice'], capabilityToken);
      }
      void invoke<ResearchWorkerCapabilities>('get_research_worker_capabilities')
        .then((capabilities) => {
          if (!cancelled) setResearchWorkerCapabilities(capabilities);
        })
        .catch(() => {
          if (!cancelled) setResearchWorkerCapabilities(null);
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
          if (!cancelled) setResearchParams(params);
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
    if (selectedVoiceClonePresetId && !voiceClonePresets.some((preset) => preset.id === selectedVoiceClonePresetId)) {
      setSelectedVoiceClonePresetId(null);
    }
  }, [selectedVoiceClonePresetId, voiceClonePresets]);

  const runtimeBaseParameters = useMemo<RuntimeBaseParameters | null>(() => {
    if (!researchParams) return null;
    return {
      audio_gain_db:
        researchParams.audio.input_gain_db +
        researchParams.audio.output_gain_db +
        researchParams.audio.loudness_adjustment_db,
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
  const runtimePeriodMs = normalizeRuntimeVariationPeriod(researchParams?.audio.random_change_period_ms);
  const runtimeActive = videoProcessingEnabled || audioProcessingEnabled;

  useEffect(() => {
    if (!runtimeActive || !runtimeBaseParameters) {
      runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: null };
      setRuntimeCycle(0);
      setRuntimeLastChangeMs(null);
      setRuntimePreview(null);
      return;
    }

    const initialNowMs = Date.now();
    runtimeSchedulerRef.current = { cycle: 0, lastChangeMs: initialNowMs };
    setRuntimeCycle(0);
    setRuntimeLastChangeMs(initialNowMs);
    setRuntimePreview(buildRuntimePreviewParameters(runtimeBaseParameters, 0));

    const timer = window.setInterval(() => {
      const nowMs = Date.now();
      const scheduler = runtimeSchedulerRef.current;
      if (scheduler.lastChangeMs === null || !isRuntimeVariationDue(nowMs, scheduler.lastChangeMs, runtimePeriodMs)) return;
      scheduler.cycle += 1;
      scheduler.lastChangeMs = nowMs;
      setRuntimeCycle(scheduler.cycle);
      setRuntimeLastChangeMs(nowMs);
      setRuntimePreview(buildRuntimePreviewParameters(runtimeBaseParameters, scheduler.cycle));
    }, 100);
    return () => window.clearInterval(timer);
  }, [runtimeActive, runtimeBaseParameters, runtimePeriodMs]);

  useEffect(() => {
    runtimeMessageRef.current = {
      version: 1,
      type: 'runtime-parameters',
      payload: runtimeActive ? runtimePreview : null,
      audio_processing_enabled: audioProcessingEnabled,
      video_processing_enabled: videoProcessingEnabled,
      playback_generation: snapshot?.playback_generation ?? null,
    };
    playbackChannelRef.current?.postMessage(runtimeMessageRef.current);
  }, [audioProcessingEnabled, runtimeActive, runtimePreview, snapshot?.playback_generation, videoProcessingEnabled]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const message = runtimeMessageRef.current;
      if (message) playbackChannelRef.current?.postMessage(message);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

  function updateResearchParam(section: 'audio' | 'video' | 'research', field: string, value: number | null) {
    if (value === null || !researchParams) return;
    setResearchParams({
      ...researchParams,
      [section]: { ...researchParams[section], [field]: value },
    });
    setResearchValidationStatus('idle');
  }

  function updateAudioSampleRate(value: number | 'source') {
    if (!researchParams) return;
    setResearchParams({
      ...researchParams,
      audio: { ...researchParams.audio, sample_rate_hz: value === 'source' ? null : value },
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
    try {
      const defaults = await invoke<ResearchParams>('get_default_local_research_params');
      setResearchParams(defaults);
      setResearchValidation([]);
      setResearchValidationStatus('idle');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '恢复研究参数默认值失败');
    }
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
          ? '尚未导入视频，请先导入一个 MP4。'
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
  const canResume = playbackDisplayState === 'paused' || playbackDisplayState === 'ready';
  const canStop = ['ready', 'playing', 'paused'].includes(playbackDisplayState);
  const voiceCloneState = snapshot?.voice_clone_replacement ?? null;
  const voiceCloneStatus = voiceCloneState?.status ?? 'idle';
  const voiceCloneModelLoading = isVoiceCloneModelLoading(voiceCloneStatus, voiceCloneState?.phase);
  const voiceClonePlaybackState = snapshot?.voice_clone_playback ?? null;
  const voiceClonePlaybackStatus = voiceClonePlaybackState?.status ?? 'idle';
  const preGeneration = snapshot?.voice_clone_pre_generation ?? {
    status: 'idle',
    batch_id: null,
    source_generation: null,
    total: 0,
    completed: 0,
    cache_hits: 0,
    generated: 0,
    failed: 0,
    items: [],
    error: null,
  };
  const voiceCloneTextCount = countUnicodeCharacters(voiceCloneText);
  const trimmedVoiceCloneText = voiceCloneText.trim();
  const trimmedVoiceClonePresetTitle = voiceClonePresetTitle.trim();
  const selectedVoiceClonePreset =
    selectedVoiceClonePresetId === null
      ? null
      : voiceClonePresets.find((preset) => preset.id === selectedVoiceClonePresetId) ?? null;
  const realtimeAudioBusy =
    ['running', 'pending', 'active'].includes(snapshot?.worker_status ?? '') ||
    snapshot?.pending_audio_candidate === true ||
    snapshot?.current_audio_source === 'realtime_variant';
  const preGenerationResourceBusy =
    preGeneration.status === 'generating' || voiceClonePreGenerationInFlightRef.current;
  const resourceConsumersBusyReason = runtimeResourceConsumerBusyReason({
    importVideoBusy,
    mediaProcessingBusy:
      mediaProcessingBusy ||
      snapshot?.video_processing_status === 'processing' ||
      snapshot?.audio_processing_status === 'processing',
    researchRunning: researchStatus?.state === 'running',
    researchActionBusy: researchActionBusy || researchCancelBusy,
    voiceCloneActionBusy: voiceCloneActionBusy !== null,
    voiceCloneModelLoading,
    preGenerationGenerating: preGenerationResourceBusy,
    voiceClonePreparing: voiceCloneStatus === 'preparing',
    voiceCloneGenerating: voiceCloneStatus === 'generating',
    voiceClonePlaybackPreparing: voiceClonePlaybackStatus === 'preparing',
    realtimeWorkerRunning: isRuntimeResourceRealtimeConsumerBusy(snapshot?.worker_status),
  });
  const resourceConsumersBusy = resourceConsumersBusyReason !== null;
  const runtimeResourceClearDisabledReason = runtimeResourceBusy
    ? '运行资源正在安装、导入、校验或清理，请等待完成后再清理。'
    : resourceConsumersBusyReason;

  useLayoutEffect(() => {
    resourceConsumersBusyReasonRef.current = resourceConsumersBusyReason;
  }, [resourceConsumersBusyReason]);
  const voiceClonePositionMs = resolvePlaybackPositionMs(
    mediaState?.current_time,
    snapshot?.current_position_ms,
  );
  const voiceClonePreGenerationBusyReason = getVoiceClonePreGenerationBusyReason(preGeneration.status);
  const voiceRuntimeReady = isVoiceRuntimeResourceReady(
    runtimeResourceStatus,
    Boolean(voiceCloneWorkerCapabilities?.available),
    voiceRuntimeReadyRef.current,
  );
  const voiceClonePrepareDisabledReason =
    voiceClonePreGenerationBusyReason ??
    (voiceCloneModelLoading
      ? '正在加载 XTTS-v2 模型，请稍候'
      : getVoiceClonePrepareDisabledReason({
          hasSource: Boolean(currentSource),
          workerAvailable: voiceRuntimeReady ? Boolean(voiceCloneWorkerCapabilities?.available) : true,
          workerReason: voiceRuntimeReady ? voiceCloneWorkerCapabilities?.reason : null,
          status: voiceCloneStatus,
        }));
  const voiceCloneReplaceDisabledReason =
    voiceClonePreGenerationBusyReason ??
    (voiceCloneModelLoading
      ? '正在加载 XTTS-v2 模型，请稍候'
      : voiceClonePlaybackStatus === 'preparing'
        ? '当前文案正在生成人声，请稍候'
        : voiceClonePlaybackStatus === 'playing'
          ? '当前文案正在播放，请播放完成后再试'
          : getVoiceCloneReplaceDisabledReason({
              hasSource: Boolean(currentSource),
              playbackState: playbackDisplayState,
              positionMs: voiceClonePositionMs,
              workerAvailable: Boolean(voiceCloneWorkerCapabilities?.available),
              workerReason: voiceCloneWorkerCapabilities?.reason,
              status: voiceCloneStatus,
              audioProcessingBlocked: Boolean(
                snapshot?.audio_processing_enabled &&
                  !snapshot.realtime_audio_variant_enabled &&
                  snapshot.current_video_source !== 'processed',
              ),
              realtimeAudioBusy,
              textError: getVoiceCloneTextError(voiceCloneText),
            }));
  const voiceCloneSaveDisabledReason =
    selectedVoiceClonePreset
      ? getVoiceClonePresetError(voiceClonePresetTitle, voiceCloneText)
      : voiceClonePresets.length >= 10
        ? '最多保存 10 条预制文本'
        : getVoiceClonePresetError(voiceClonePresetTitle, voiceCloneText);
  const voiceCloneCanCancel =
    voiceCloneAutoPreparePhase !== null ||
    preGeneration.status === 'generating' ||
    voiceCloneStatus === 'preparing' ||
    voiceCloneStatus === 'generating' ||
    voiceClonePlaybackStatus === 'preparing';
  const voiceCloneCanClear =
    voiceCloneStatus === 'ready' ||
    voiceCloneStatus === 'playing' ||
    voiceCloneStatus === 'failed' ||
    voiceCloneStatus === 'cancelled' ||
    Boolean(voiceCloneState?.error) ||
    voiceClonePlaybackStatus === 'ready' ||
    voiceClonePlaybackStatus === 'playing' ||
    voiceClonePlaybackStatus === 'failed' ||
    voiceClonePlaybackStatus === 'cancelled' ||
    Boolean(voiceClonePlaybackState?.error);
  const voiceCloneProgressSource = ['preparing', 'failed', 'cancelled', 'playing'].includes(voiceClonePlaybackStatus)
    ? voiceClonePlaybackState
    : voiceCloneState;
  const voiceCloneProgressStatus =
    voiceCloneProgressSource?.status === 'failed'
      ? 'exception'
      : voiceCloneProgressSource?.status === 'ready' || voiceCloneProgressSource?.status === 'playing'
        ? 'success'
        : 'active';
  const voiceCloneProgressPercent =
    voiceCloneProgressSource?.progress_percent ?? getVoiceCloneProgress(voiceCloneProgressSource?.status ?? voiceCloneStatus);
  const voiceCloneProgressMessage = voiceCloneProgressSource?.progress_message?.trim() || null;
  const voiceCloneNotice =
    !voiceRuntimeReady
      ? {
          type: 'info' as const,
          message: '固定话术运行资源尚未就绪，首次使用会自动下载（包含 FFmpeg、Worker 和模型）。',
        }
      : !voiceCloneWorkerCapabilities?.available
      ? {
          type: 'warning' as const,
          message: `固定话术 Worker 不可用：${voiceCloneWorkerCapabilities?.reason ?? '未完成能力探测'}`,
        }
      : voiceCloneModelLoading
        ? {
            type: 'info' as const,
            message: voiceCloneProgressMessage ?? '正在加载 XTTS-v2 模型，请稍候。',
          }
      : voiceClonePlaybackStatus === 'preparing'
        ? {
            type: 'info' as const,
          message: voiceClonePlaybackState?.progress_message ?? '正在生成当前文案的人声。',
          }
      : voiceClonePlaybackStatus === 'playing'
          ? {
              type: 'success' as const,
              message: '当前文案人声正在播放，播放结束后自动恢复原音轨。',
            }
        : voiceClonePlaybackStatus === 'failed'
          ? {
              type: 'error' as const,
              message: voiceClonePlaybackState?.error ?? '当前文案人声生成失败。',
            }
        : voiceClonePlaybackStatus === 'cancelled'
          ? {
              type: 'warning' as const,
              message: voiceClonePlaybackState?.error ?? '当前文案人声生成已取消。',
            }
      : voiceCloneStatus === 'preparing'
        ? {
            type: 'info' as const,
            message: voiceCloneProgressMessage ?? '正在准备当前 MP4 的参考人声（自动去除背景音乐）与话术索引。',
          }
        : voiceCloneStatus === 'ready'
          ? {
              type: 'success' as const,
              message: '参考人声与 XTTS-v2 模型已准备完成，可以播放当前文案。',
            }
      : voiceCloneStatus === 'generating'
            ? {
                type: 'info' as const,
                message: voiceCloneProgressMessage ?? '固定话术正在生成完整替换音轨。',
              }
            : voiceCloneStatus === 'playing'
              ? { type: 'success' as const, message: '固定话术替换音轨正在当前轮次生效。' }
              : voiceCloneStatus === 'failed'
                ? {
                    type: 'error' as const,
                    message: voiceCloneState?.error ?? '固定话术替换失败',
                  }
                : voiceCloneStatus === 'cancelled'
                  ? {
                      type: 'warning' as const,
                      message: voiceCloneState?.error ?? '固定话术操作已取消',
                    }
                : {
                    type: 'info' as const,
                    message:
                      voiceCloneAutoPreparePhase === 'delay'
                        ? '最终效果窗口已打开，4 秒后开始自动准备人声。'
                        : getVoiceCloneIdleNotice(),
                  };
  const voiceCloneStatusLabel = voiceCloneModelLoading
    ? '加载模型中'
    : voiceClonePlaybackStatus === 'preparing'
      ? '当前文案生成中'
      : voiceClonePlaybackStatus === 'playing'
        ? '当前文案播放中'
        : getVoiceCloneStatusLabel(voiceCloneStatus);

  useEffect(() => {
    voiceClonePreGenerationStartedKeyRef.current = null;
  }, [snapshot?.playback_generation]);

  useEffect(() => {
    const sourceGeneration = snapshot?.playback_generation;
    if (
      !canStartVoiceClonePreGenerationForRuntime(
        voiceRuntimeReady,
        runtimeResourceBusy,
        runtimeResourceClearInFlightRef.current,
      )
      || !shouldStartVoiceClonePreGeneration({
        sourceReady: voiceCloneStatus === 'ready',
        sourceGeneration,
        presetCount: voiceClonePresets.length,
        presetTextRevision: voiceClonePresetTextRevision,
        batchStatus: preGeneration.status,
        replacementStatus: voiceCloneStatus,
        playbackStatus: voiceClonePlaybackStatus,
        lastStartedKey: voiceClonePreGenerationStartedKeyRef.current,
      }) ||
      voiceClonePreGenerationInFlightRef.current
    ) {
      return;
    }

    const key = getVoiceClonePreGenerationTriggerKey(sourceGeneration, voiceClonePresetTextRevision);
    if (key === null) return;
    let cancelled = false;
    let retryAfterFailure = false;
    voiceClonePreGenerationStartedKeyRef.current = key;
    voiceClonePreGenerationInFlightRef.current = true;
    void invoke<PlaybackSnapshot>('start_voice_clone_pre_generation', {
      request: { items: voiceClonePresets.map(({ id, text }) => ({ preset_id: id, text })) },
    })
      .then((nextSnapshot) => {
        if (!shouldAcceptVoiceClonePreGenerationResult({
          cancelled,
          mounted: voiceClonePreGenerationMountedRef.current,
          requestedGeneration: sourceGeneration,
          currentGeneration: voiceClonePreGenerationCurrentGenerationRef.current,
          resultGeneration: nextSnapshot.playback_generation,
        })) return;
        setVoiceCloneFormError(null);
        setSnapshot(nextSnapshot);
      })
      .catch((cause) => {
        if (
          !voiceClonePreGenerationMountedRef.current ||
          key !== voiceClonePreGenerationCurrentKeyRef.current
        ) return;
        setVoiceCloneFormError(getDisplayErrorMessage(cause, '批量准备文案人声失败'));
        if (!shouldRetryVoiceClonePreGeneration({
          failedKey: key,
          currentKey: voiceClonePreGenerationCurrentKeyRef.current,
          lastRetriedKey: voiceClonePreGenerationRetriedKeyRef.current,
        })) return;
        voiceClonePreGenerationRetriedKeyRef.current = key;
        voiceClonePreGenerationStartedKeyRef.current = null;
        retryAfterFailure = true;
      })
      .finally(() => {
        voiceClonePreGenerationInFlightRef.current = false;
        if ((cancelled || retryAfterFailure) && voiceClonePreGenerationMountedRef.current) {
          setVoiceClonePreGenerationCompletionVersion((value) => value + 1);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    snapshot?.playback_generation,
    voiceCloneStatus,
    preGeneration.status,
    voiceClonePlaybackStatus,
    voiceClonePresets,
    voiceClonePresetTextRevision,
    voiceClonePreGenerationCompletionVersion,
    voiceRuntimeReady,
    runtimeResourceBusy,
  ]);

  function applyVoiceClonePresetSelection(presetId: string | null) {
    setSelectedVoiceClonePresetId(presetId);
    if (!presetId) return;
    const preset = voiceClonePresets.find((item) => item.id === presetId);
    if (!preset) return;
    setVoiceClonePresetTitle(preset.title);
    setVoiceCloneText(preset.text);
    setVoiceCloneFormError(null);
  }

  async function prepareVoiceCloneSource(options: { automatic?: boolean } = {}) {
    const automatic = options.automatic === true;
    try {
      await ensureRuntimeResources('voice', async () => {
        if (voiceClonePrepareInFlightRef.current) return;
        voiceClonePrepareInFlightRef.current = true;
        setVoiceCloneActionBusy('prepare');
        setVoiceCloneFormError(null);
        if (!automatic) setError(null);
        try {
          const nextSnapshot = await invoke<PlaybackSnapshot>('prepare_voice_clone_source', { request: {} });
          setSnapshot(nextSnapshot);
        } catch (cause) {
          const message = getDisplayErrorMessage(cause, '固定话术准备失败');
          if (automatic) {
            setVoiceCloneFormError(message);
          } else {
            setError(message);
          }
        } finally {
          voiceClonePrepareInFlightRef.current = false;
          setVoiceCloneActionBusy(null);
        }
      });
    } catch (cause) {
      const message = getDisplayErrorMessage(cause, '固定话术运行资源准备失败');
      if (automatic) {
        setVoiceCloneFormError(message);
      } else {
        setError(message);
      }
    }
  }

  async function playCurrentVoiceCloneText() {
    const validationError = getVoiceCloneTextError(voiceCloneText);
    if (validationError) {
      setVoiceCloneFormError(validationError);
      return;
    }
    if (voiceClonePositionMs === null) {
      setVoiceCloneFormError('播放器位置尚未同步，请稍后再试');
      return;
    }
    // 在点击事件的同步阶段通知播放器恢复 Web Audio，避免等 IPC 返回后才错过用户手势。
    playbackChannelRef.current?.postMessage({
      version: 1,
      type: 'playback-control',
      action: 'resume',
    } satisfies PlaybackControlMessage);
    setVoiceCloneActionBusy('play');
    setVoiceCloneFormError(null);
    setError(null);
    try {
      const nextSnapshot = await invoke<PlaybackSnapshot>('start_voice_clone_playback', {
        request: {
          text: trimmedVoiceCloneText,
          position_ms: voiceClonePositionMs,
        },
      });
      setSnapshot(nextSnapshot);
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '当前文案人声播放失败'));
    } finally {
      setVoiceCloneActionBusy(null);
    }
  }

  async function cancelVoiceCloneOperation() {
    voiceCloneAutoPrepareControllerRef.current?.abort();
    setVoiceCloneActionBusy('cancel');
    setError(null);
    try {
      setSnapshot(await invoke<PlaybackSnapshot>('cancel_voice_clone_operation'));
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '取消固定话术操作失败'));
    } finally {
      setVoiceCloneActionBusy(null);
    }
  }

  async function clearVoiceCloneReplacement() {
    setVoiceCloneActionBusy('clear');
    setError(null);
    try {
      setSnapshot(await invoke<PlaybackSnapshot>('clear_voice_clone_replacement'));
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '清空固定话术替换失败'));
    } finally {
      setVoiceCloneActionBusy(null);
    }
  }

  function saveVoiceClonePresetFromForm() {
    const validationError = getVoiceClonePresetError(voiceClonePresetTitle, voiceCloneText);
    if (validationError) {
      setVoiceCloneFormError(validationError);
      return;
    }
    try {
      const previousText = selectedVoiceClonePreset?.text;
      const nextPresets = selectedVoiceClonePreset
        ? updateVoiceClonePreset(window.localStorage, voiceClonePresets, {
            id: selectedVoiceClonePreset.id,
            title: trimmedVoiceClonePresetTitle,
            text: trimmedVoiceCloneText,
          })
        : addVoiceClonePreset(window.localStorage, voiceClonePresets, {
            title: trimmedVoiceClonePresetTitle,
            text: trimmedVoiceCloneText,
          });
      setVoiceClonePresets(nextPresets);
      const activePreset =
        selectedVoiceClonePreset
          ? nextPresets.find((preset) => preset.id === selectedVoiceClonePreset.id) ?? null
          : nextPresets[nextPresets.length - 1] ?? null;
      setSelectedVoiceClonePresetId(activePreset?.id ?? null);
      setVoiceClonePresetTitle(activePreset?.title ?? trimmedVoiceClonePresetTitle);
      setVoiceCloneText(activePreset?.text ?? trimmedVoiceCloneText);
      if (!selectedVoiceClonePreset || previousText !== activePreset?.text) {
        setVoiceClonePresetTextRevision((value) => value + 1);
      }
      setVoiceCloneFormError(null);
    } catch (cause) {
      setVoiceCloneFormError(getDisplayErrorMessage(cause, '添加或更新文案失败'));
    }
  }

  function deleteSelectedVoiceClonePreset() {
    if (!selectedVoiceClonePreset) return;
    try {
      const nextPresets = removeVoiceClonePreset(window.localStorage, voiceClonePresets, selectedVoiceClonePreset.id);
      setVoiceClonePresets(nextPresets);
      setSelectedVoiceClonePresetId(null);
      setVoiceClonePresetTitle('');
      setVoiceCloneText('');
      setVoiceCloneFormError(null);
    } catch (cause) {
      setVoiceCloneFormError(getDisplayErrorMessage(cause, '删除预制文本失败'));
    }
  }

  async function openFinalEffectWindowFromHome(): Promise<boolean> {
    setPlayerWindowBusy(true);
    setError(null);
    try {
      await invoke('open_final_effect_window');
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

  async function runPlaybackAction(action: 'pause' | 'resume' | 'stop', command: 'pause_playback' | 'resume_playback' | 'stop_playback') {
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

  async function updateProcessingSwitches(next: {
    video_processing_enabled: boolean;
    audio_processing_enabled: boolean;
    realtime_audio_variant_enabled: boolean;
  }) {
    setVideoProcessingEnabled(next.video_processing_enabled);
    setAudioProcessingEnabled(next.audio_processing_enabled);
    setRealtimeAudioVariantEnabled(next.realtime_audio_variant_enabled);
    try {
      const nextSnapshot = await invoke<PlaybackSnapshot>('set_processing_switches', {
        request: next,
      });
      let effectiveSnapshot = nextSnapshot;
      if (next.audio_processing_enabled && researchParams) {
        effectiveSnapshot = await invoke<PlaybackSnapshot>('set_audio_processing_profile', {
          request: {
            profile: {
              parameters_version: 'audio_processing_v1',
              params: researchParams.audio,
            },
          },
        });
      }
      setSnapshot(effectiveSnapshot);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '更新处理开关失败');
    }
  }

  async function applyMediaProcessing() {
    if (!researchParams || !snapshot?.source_media || (!videoProcessingEnabled && !audioProcessingEnabled)) return;
    setError(null);
    setMediaProcessingBusy(true);
    try {
      await ensureRuntimeResources('media', async () => {
        setMediaProcessingBusy(true);
        try {
          let effectiveSnapshot = snapshot;
          if (audioProcessingEnabled) {
            effectiveSnapshot = await invoke<PlaybackSnapshot>('set_audio_processing_profile', {
              request: {
                profile: {
                  parameters_version: 'audio_processing_v1',
                  params: researchParams.audio,
                },
              },
            });
          }
          const nextSnapshot = await invoke<PlaybackSnapshot>('start_media_processing', {
            request: { params: researchParams },
          });
          setSnapshot(nextSnapshot ?? effectiveSnapshot);
        } finally {
          setMediaProcessingBusy(false);
        }
      });
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '启动本地媒体处理失败'));
    } finally {
      setMediaProcessingBusy(false);
    }
  }

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
    setCacheCleanupBusy(true);
    try {
      setCacheCleanup(await invoke<CacheCleanupResult>('cleanup_local_caches_command'));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '清理本地缓存失败');
    } finally {
      setCacheCleanupBusy(false);
    }
  }

  async function prepareVoiceCloneAfterImport(playbackGeneration: number) {
    voiceCloneAutoPrepareControllerRef.current?.abort();
    const controller = new AbortController();
    voiceCloneAutoPrepareControllerRef.current = controller;
    setVoiceCloneAutoPreparePhase('delay');
    try {
      await waitForAbortableDelay(VOICE_CLONE_AUTO_PREPARE_DELAY_MS, controller.signal);
      const nextSnapshot = await invoke<PlaybackSnapshot>('get_snapshot');
      if (controller.signal.aborted) return;
      setSnapshot(nextSnapshot);
      if (nextSnapshot.playback_generation !== playbackGeneration) return;

      const source = nextSnapshot.source_media;
      const sourcePath = source?.source_path?.trim();
      const sourceKey = getVoiceCloneAutoPrepareKey(sourcePath, nextSnapshot.playback_generation);
      if (
        !shouldAutoPrepareVoiceCloneSource({
          sourcePath,
          playbackGeneration: nextSnapshot.playback_generation,
          status: nextSnapshot.voice_clone_replacement.status,
          triggeredKey: voiceCloneAutoPrepareKeyRef.current,
        })
      ) {
        return;
      }
      if (importVideoInFlightRef.current) return;
      voiceCloneAutoPrepareKeyRef.current = sourceKey;
      await prepareVoiceCloneSource({ automatic: true });
    } catch (cause) {
      if (!controller.signal.aborted) {
        setVoiceCloneFormError(getDisplayErrorMessage(cause, '自动准备人声失败'));
      }
    } finally {
      if (voiceCloneAutoPrepareControllerRef.current === controller) {
        setVoiceCloneAutoPreparePhase(null);
        voiceCloneAutoPrepareControllerRef.current = null;
      }
    }
  }

  async function importVideo(selectedSourcePath?: string) {
    if (importVideoInFlightRef.current) return;
    const restoreAutoPrepareGeneration = selectedSourcePath === undefined
      && voiceCloneAutoPrepareControllerRef.current !== null
      && voiceCloneAutoPrepareKeyRef.current === null
      ? voiceClonePreGenerationCurrentGenerationRef.current
      : null;
    let selected: string | null = null;
    importVideoInFlightRef.current = true;
    voiceCloneAutoPrepareControllerRef.current?.abort();
    voiceCloneAutoPrepareKeyRef.current = null;
    setImportVideoBusy(true);
    try {
      const selection =
        selectedSourcePath ??
        (await open({ multiple: false, filters: [{ name: 'MP4 视频', extensions: ['mp4'] }] }));
      if (typeof selection !== 'string') return;
      selected = selection;
      setVoiceCloneFormError(null);
      setError(null);
      await ensureRuntimeResources('media', async () => {
        importVideoInFlightRef.current = true;
        setImportVideoBusy(true);
        try {
          const result = await invoke<MediaProbeResult>('probe_local_mp4', { request: { path: selected } });
          setProbe(result);
          window.localStorage.setItem('autolive.source.path', result.canonical_path);
          const startedSnapshot = await invoke<PlaybackSnapshot>('start_playback');
          setSnapshot(startedSnapshot);
          setSnapshotLoading(false);
          setSnapshotFetchError(null);
          const finalEffectWindowOpened = await openFinalEffectWindowFromHome();
          if (finalEffectWindowOpened) {
            void prepareVoiceCloneAfterImport(startedSnapshot.playback_generation);
          }
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
      if (
        restoreAutoPrepareGeneration !== null
        && shouldRestoreVoiceCloneAutoPrepareAfterPicker({
          interactivePicker: selectedSourcePath === undefined,
          hadPendingAutoPrepare: true,
          selectedPath: selected,
          playbackGenerationBefore: restoreAutoPrepareGeneration,
          playbackGenerationAfter: voiceClonePreGenerationCurrentGenerationRef.current,
        })
      ) {
        void prepareVoiceCloneAfterImport(restoreAutoPrepareGeneration);
      }
    }
  }

  async function retryRuntimeResources() {
    const pending = pendingRuntimeActionRef.current;
    if (!pending) runtimeResourceActionTokenRef.current += 1;
    const token = pending?.token ?? runtimeResourceActionTokenRef.current;
    const component = pending?.component ?? runtimeResourceStatus?.component ?? 'media';
    try {
      const status = await invoke<RuntimeResourceStatus>('install_runtime_resources', { component });
      await applyRuntimeResourceStatus(status, token);
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '运行资源重试失败'));
    }
  }

  async function cancelRuntimeResources() {
    try {
      const pending = pendingRuntimeActionRef.current;
      runtimeResourcePollGenerationRef.current += 1;
      const status = await invoke<RuntimeResourceStatus>('cancel_runtime_resource_install');
      await applyRuntimeResourceStatus(status, pending?.token);
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '取消运行资源安装失败'));
      setRuntimeResourceStatus((current) => current ? { ...current } : current);
    }
  }

  async function chooseRuntimeResourceDirectory() {
    try {
      const sourceRoot = await open({ directory: true, multiple: false });
      if (typeof sourceRoot !== 'string') return;
      const pending = pendingRuntimeActionRef.current;
      if (!pending) runtimeResourceActionTokenRef.current += 1;
      const token = pending?.token ?? runtimeResourceActionTokenRef.current;
      const component = pending?.component ?? runtimeResourceStatus?.component ?? 'media';
      const status = await invoke<RuntimeResourceStatus>('import_runtime_resource_directory', {
        component,
        sourceRoot,
      });
      await applyRuntimeResourceStatus(status, token);
    } catch (cause) {
      setError(getDisplayErrorMessage(cause, '导入本地运行资源失败'));
    }
  }

  function confirmClearRuntimeResources() {
    const blockedReason = runtimeResourceBusyRef.current
      ? '运行资源正在安装、导入、校验或清理，请等待完成后再清理。'
      : voiceClonePreGenerationInFlightRef.current
        ? '本地媒体或语音任务正在使用运行资源，请先完成或取消后再清理。'
      : resourceConsumersBusyReasonRef.current;
    if (blockedReason) {
      setError(blockedReason);
      return;
    }
    Modal.confirm({
      title: '清理运行资源？',
      content: '将删除当前版本的 FFmpeg、固定话术运行环境和模型，后续使用时需要重新下载。',
      okText: '确认清理',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: async () => {
        const latestBlockedReason = runtimeResourceBusyRef.current
          ? '运行资源正在安装、导入、校验或清理，请等待完成后再清理。'
          : voiceClonePreGenerationInFlightRef.current
            ? '本地媒体或语音任务正在使用运行资源，请先完成或取消后再清理。'
          : resourceConsumersBusyReasonRef.current;
        if (latestBlockedReason) {
          setError(latestBlockedReason);
          return;
        }
        runtimeResourceActionTokenRef.current += 1;
        const token = runtimeResourceActionTokenRef.current;
        runtimeResourcePollGenerationRef.current += 1;
        pendingRuntimeActionRef.current = null;
        runtimeResourceClearInFlightRef.current = true;
        runtimeResourceBusyRef.current = true;
        const previousStatus = runtimeResourceStatus;
        setRuntimeResourceStatus({
          state: 'checking',
          component: null,
          current_file: null,
          downloaded_bytes: 0,
          total_bytes: 0,
          bytes_per_second: 0,
          installed_bytes: previousStatus?.installed_bytes ?? 0,
          resource_root: previousStatus?.resource_root ?? '',
          error: null,
        });
        try {
          const status = await invoke<RuntimeResourceStatus>('clear_runtime_resources');
          await applyRuntimeResourceStatus(status, token);
        } catch (cause) {
          runtimeResourceClearInFlightRef.current = false;
          runtimeResourceBusyRef.current = previousStatus !== null && isRuntimeResourceBusy(previousStatus);
          setRuntimeResourceStatus(previousStatus);
          refreshRuntimeResourceCapabilities(['media', 'voice'], token);
          setError(getDisplayErrorMessage(cause, '清理运行资源失败'));
        }
      },
    });
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
    : Math.max(0, runtimePeriodMs - (runtimeNowMs - runtimeLastChangeMs));
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
                导入视频并播放
              </Button>
              <Button size="large" onClick={() => void openFinalEffectWindowFromHome()} loading={playerWindowBusy}>
                打开/聚焦播放器
              </Button>
            </Space>
            {error ? <Alert type="error" showIcon message={error} /> : null}
            {runtimeResourceStatus ? (
              <Card title="运行资源">
                <Alert
                  type={runtimeResourceStatus.state === 'failed' ? 'error' : runtimeResourceStatus.state === 'ready' ? 'success' : 'info'}
                  showIcon
                  message={runtimeResourceMessage(runtimeResourceStatus)}
                  description={(
                    <div className="runtime-resource-progress">
                      <Progress
                        percent={runtimeResourcePercent(runtimeResourceStatus)}
                        status={runtimeResourceStatus.state === 'failed' ? 'exception' : runtimeResourceStatus.state === 'ready' ? 'success' : 'active'}
                      />
                      <Typography.Text type="secondary">
                        {runtimeResourceComponentDescription(runtimeResourceStatus.component)}
                      </Typography.Text>
                      <Typography.Text type="secondary">
                        {runtimeResourceProgressDetails(runtimeResourceStatus)}
                      </Typography.Text>
                      {runtimeResourcePollError ? (
                        <Typography.Text type="danger">{runtimeResourcePollError}</Typography.Text>
                      ) : null}
                    </div>
                  )}
                  action={(
                    <Space wrap className="runtime-resource-actions">
                      <Button disabled={runtimeResourceBusy || runtimeResourceStatus.state === 'ready'} onClick={() => void retryRuntimeResources()}>
                        {runtimeResourceStatus.state === 'not-installed' ? '安装' : '重试'}
                      </Button>
                      <Button disabled={!runtimeResourceBusy} onClick={() => void cancelRuntimeResources()}>
                        取消
                      </Button>
                      <Button disabled={runtimeResourceBusy} onClick={() => void chooseRuntimeResourceDirectory()}>
                        选择本地资源目录
                      </Button>
                      <Button
                        danger
                        disabled={runtimeResourceBusy || resourceConsumersBusy}
                        title={runtimeResourceClearDisabledReason ?? undefined}
                        onClick={confirmClearRuntimeResources}
                      >
                        清理运行资源
                      </Button>
                    </Space>
                  )}
                />
              </Card>
            ) : null}
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
                  <Descriptions.Item label="声音处理">{snapshot?.audio_processing_enabled ? '开启' : '关闭'}</Descriptions.Item>
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
                    {currentSource?.file_name.split('.').pop()?.toUpperCase() ?? 'MP4'}
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
                <Space align="center">
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
                  <Typography.Text>声音处理</Typography.Text>
                </Space>
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
                  <Tag color={voiceCloneWorkerCapabilities?.available ? 'green' : 'orange'}>
                    Worker：{voiceCloneWorkerCapabilities?.available ? '可用' : '不可用'}
                  </Tag>
                  <Tag color={voiceCloneModelLoading ? 'orange' : voiceClonePlaybackStatus === 'playing' ? 'green' : voiceCloneStatus === 'ready' || voiceCloneStatus === 'playing' ? 'green' : 'blue'}>
                    状态：{voiceCloneStatusLabel}
                  </Tag>
                  <Tag>预制：{voiceClonePresets.length} / 10</Tag>
                  {voiceCloneState?.model ? <Tag>{voiceCloneState.model}</Tag> : null}
                </Space>
                <Alert type={voiceCloneNotice.type} showIcon message={voiceCloneNotice.message} />
                <Progress
                  percent={voiceCloneProgressPercent}
                  status={voiceCloneProgressStatus}
                  showInfo={false}
                />
                <Alert
                  type={preGeneration.failed > 0 ? 'warning' : preGeneration.status === 'ready' ? 'success' : 'info'}
                  showIcon
                  message={getVoiceClonePreGenerationSummary(preGeneration)}
                />
                <Progress
                  percent={preGeneration.total === 0 ? 0 : Math.round(preGeneration.completed * 100 / preGeneration.total)}
                  status={preGeneration.failed > 0 ? 'exception' : preGeneration.status === 'ready' ? 'success' : 'active'}
                />
                <Space direction="vertical" size="small">
                  {voiceClonePresets.map((preset) => {
                    const item = preGeneration.items.find(({ preset_id }) => preset_id === preset.id);
                    return (
                      <Space key={preset.id} wrap>
                        <Typography.Text>{preset.title}</Typography.Text>
                        <Tag color={item?.status === 'failed' ? 'error' : item?.status === 'cached' || item?.status === 'generated' ? 'success' : 'processing'}>
                          {getVoiceClonePreGenerationItemStatusLabel(item?.status ?? 'pending')}
                        </Tag>
                      </Space>
                    );
                  })}
                </Space>
                {realtimeAudioBusy ? <Alert type="warning" showIcon message="当前实时音频正在占用" /> : null}
                {voiceCloneFormError ? <Alert type="error" showIcon message={voiceCloneFormError} /> : null}
                <Select
                  allowClear
                  placeholder="选择一条本地预制文本"
                  value={selectedVoiceClonePresetId ?? undefined}
                  options={voiceClonePresets.map((preset) => ({
                    label: preset.title,
                    value: preset.id,
                  }))}
                  onChange={(value) => applyVoiceClonePresetSelection(typeof value === 'string' ? value : null)}
                />
                <Input
                  placeholder="预制标题"
                  value={voiceClonePresetTitle}
                  maxLength={80}
                  onChange={(event) => {
                    setVoiceClonePresetTitle(event.target.value);
                    setVoiceCloneFormError(null);
                  }}
                />
                <Input.TextArea
                  value={voiceCloneText}
                  rows={5}
                  maxLength={500}
                  placeholder="输入或编辑要替换当前话术的文本"
                  onChange={(event) => {
                    setVoiceCloneText(event.target.value);
                    setVoiceCloneFormError(null);
                  }}
                />
                <Space wrap style={{ width: '100%', justifyContent: 'space-between' }}>
                  <Typography.Text type={voiceCloneTextCount > 500 ? 'danger' : undefined}>
                    文本字数：{voiceCloneTextCount} / 500
                  </Typography.Text>
                  {voiceCloneState?.replace_at_ms !== null && voiceCloneState?.replace_at_ms !== undefined ? (
                    <Typography.Text type="secondary">
                      最近替换位置：{formatMediaTime((voiceCloneState.replace_at_ms ?? 0) / 1000)}
                    </Typography.Text>
                  ) : null}
                </Space>
                <Space wrap>
                  <Button
                    onClick={() => void prepareVoiceCloneSource()}
                    loading={voiceCloneActionBusy === 'prepare'}
                    disabled={voiceClonePrepareDisabledReason !== null || runtimeResourceBusy}
                  >
                    准备人声
                  </Button>
                  <Button
                    type="primary"
                    onClick={() => void playCurrentVoiceCloneText()}
                    loading={voiceCloneActionBusy === 'play'}
                    disabled={voiceCloneReplaceDisabledReason !== null || runtimeResourceBusy}
                    title={voiceCloneReplaceDisabledReason ?? undefined}
                  >
                    播放当前文案
                  </Button>
                  <Button onClick={saveVoiceClonePresetFromForm} disabled={voiceCloneSaveDisabledReason !== null}>
                    {selectedVoiceClonePreset ? '更新文案' : '添加文案'}
                  </Button>
                  <Button danger onClick={deleteSelectedVoiceClonePreset} disabled={!selectedVoiceClonePreset}>
                    删除预制文本
                  </Button>
                  <Button
                    onClick={() => void cancelVoiceCloneOperation()}
                    loading={voiceCloneActionBusy === 'cancel'}
                    disabled={!voiceCloneCanCancel}
                  >
                    取消
                  </Button>
                  <Button
                    onClick={() => void clearVoiceCloneReplacement()}
                    loading={voiceCloneActionBusy === 'clear'}
                    disabled={!voiceCloneCanClear}
                  >
                    清空当前替换
                  </Button>
                </Space>
                {voiceCloneReplaceDisabledReason ? (
                  <Typography.Text type="secondary">{voiceCloneReplaceDisabledReason}</Typography.Text>
                ) : null}
                <Typography.Text type="secondary">
                  点击后从当前播放器位置播放这一条文案；播放期间原音轨静音，播放结束恢复原音轨。视频循环不会自动重复文案，下一次需要再次点击。
                </Typography.Text>
              </Space>
            </Card>
            <Card title="音频实时参数预览">
              <Space direction="vertical" size="middle" style={{ width: '100%' }}>
                <Space wrap>
                  <Tag color={audioProcessingEnabled ? 'green' : 'default'}>
                    声音动态：{audioProcessingEnabled ? '开启' : '关闭'}
                  </Tag>
                  <Tag>周期：{runtimePeriodMs} ms</Tag>
                  <Tag>第 {runtimeCycle} 次变化</Tag>
                  <Tag>下一次：{runtimeRemainingMs === null ? '未启动' : `${runtimeRemainingMs} ms`}</Tag>
                </Space>
                {runtimeActive && runtimeBaseParameters && runtimePreview ? (
                  <Descriptions column={1} size="small">
                    <Descriptions.Item label="音频增益（配置 / 当前）">
                      {runtimeBaseParameters.audio_gain_db.toFixed(1)} / {runtimePreview.audio_gain_db.toFixed(1)} dB
                    </Descriptions.Item>
                  </Descriptions>
                ) : (
                  <Alert type="info" showIcon message="打开声音处理或视频处理开关后，运行时预览会按周期变化；关闭后恢复配置基线。" />
                )}
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
                      : '参数先由 Rust 校验；点击“应用当前处理参数”后在后台生成当前源视频的预览缓存'
                }
                  description={researchValidation.slice(0, 3).map((item) => `${item.field}：${item.message}`).join('；') || undefined}
                />
                <Typography.Title level={5} style={{ marginTop: 24 }}>
                  音频研究参数
                </Typography.Title>
                {researchParams ? (
                <Space wrap style={{ marginTop: 16 }}>
                  <InputNumber aria-label="音频动态周期" addonBefore="动态周期" addonAfter="ms" value={researchParams.audio.random_change_period_ms} min={500} max={60_000} step={500} onChange={(value) => updateResearchParam('audio', 'random_change_period_ms', value)} />
                  <InputNumber aria-label="音频音高微移" addonBefore="音高微移" addonAfter="半音" value={researchParams.audio.pitch_shift_semitones} min={-2} max={2} step={0.1} onChange={(value) => updateResearchParam('audio', 'pitch_shift_semitones', value)} />
                  <InputNumber aria-label="音频 MFCC 偏移" addonBefore="MFCC" addonAfter="%" value={researchParams.audio.mfcc_shift_percent} min={-20} max={20} onChange={(value) => updateResearchParam('audio', 'mfcc_shift_percent', value)} />
                  <InputNumber aria-label="音频 SNR 浮动" addonBefore="SNR 浮动" addonAfter="dB" value={researchParams.audio.snr_variation_db} min={-6} max={6} onChange={(value) => updateResearchParam('audio', 'snr_variation_db', value)} />
                  <InputNumber aria-label="音频输入增益" addonBefore="输入增益" addonAfter="dB" value={researchParams.audio.input_gain_db} min={-6} max={6} step={0.1} onChange={(value) => updateResearchParam('audio', 'input_gain_db', value)} />
                  <InputNumber aria-label="音频输出增益" addonBefore="输出增益" addonAfter="dB" value={researchParams.audio.output_gain_db} min={-6} max={6} step={0.1} onChange={(value) => updateResearchParam('audio', 'output_gain_db', value)} />
                  <InputNumber aria-label="音频响度调整" addonBefore="响度" addonAfter="dB" value={researchParams.audio.loudness_adjustment_db} min={-6} max={6} step={0.1} onChange={(value) => updateResearchParam('audio', 'loudness_adjustment_db', value)} />
                  <Select
                    aria-label="音频采样率"
                    value={researchParams.audio.sample_rate_hz ?? 'source'}
                    options={[
                      { label: '采样率：跟随源素材', value: 'source' },
                      { label: '采样率：44100 Hz', value: 44100 },
                      { label: '采样率：48000 Hz', value: 48000 },
                    ]}
                    onChange={updateAudioSampleRate}
                  />
                </Space>
              ) : null}
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
                  <Button
                    type="primary"
                    onClick={() => void applyMediaProcessing()}
                    loading={mediaProcessingBusy}
                    disabled={
                      !snapshot?.source_media ||
                      !researchParams ||
                      (!videoProcessingEnabled && !audioProcessingEnabled) ||
                      snapshot.video_processing_status === 'processing' ||
                      snapshot.audio_processing_status === 'processing' ||
                      mediaProcessingBusy ||
                      runtimeResourceBusy
                    }
                  >
                    应用当前处理参数
                  </Button>
                  <Tag color={mediaEngineCapabilities?.available ? 'green' : 'orange'}>
                    媒体引擎：{mediaEngineCapabilities?.available ? '可用' : '不可用'}
                  </Tag>
                </Space>
                <Alert
                  type={mediaEngineCapabilities?.available ? 'success' : 'warning'}
                  showIcon
                  message={
                    mediaEngineCapabilities?.available
                      ? 'FFmpeg/FFprobe 已就绪，处理结果将在当前视频下一轮开始时切换'
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
                  <Tag>周期：{runtimePeriodMs} ms</Tag>
                  <Tag>第 {runtimeCycle} 次变化</Tag>
                  <Tag>下一次：{runtimeRemainingMs === null ? '未启动' : `${runtimeRemainingMs} ms`}</Tag>
                </Space>
                {runtimeActive && runtimeBaseParameters && runtimePreview ? (
                  <Descriptions column={1} size="small">
                    <Descriptions.Item label="亮度（配置 / 当前）">
                      {runtimeBaseParameters.video_brightness_percent.toFixed(1)} / {runtimePreview.video_brightness_percent.toFixed(1)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="对比度（配置 / 当前）">
                      {runtimeBaseParameters.video_contrast_percent.toFixed(1)} / {runtimePreview.video_contrast_percent.toFixed(1)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="饱和度（配置 / 当前）">
                      {runtimeBaseParameters.video_saturation_percent.toFixed(1)} / {runtimePreview.video_saturation_percent.toFixed(1)}%
                    </Descriptions.Item>
                    <Descriptions.Item label="色相 / 模糊">
                      {runtimePreview.video_hue_rotation_degrees.toFixed(1)}° / {runtimePreview.video_blur_radius_px.toFixed(1)} px
                    </Descriptions.Item>
                    <Descriptions.Item label="画面缩放 / 位移">
                      {runtimePreview.video_pixel_scale_percent.toFixed(1)}% / {runtimePreview.video_space_x_offset_px.toFixed(1)}, {runtimePreview.video_space_y_offset_px.toFixed(1)} px
                    </Descriptions.Item>
                  </Descriptions>
                ) : (
                  <Alert type="info" showIcon message="打开视频处理或声音处理开关后，运行时预览会按周期变化；关闭后恢复配置基线。" />
                )}
                {runtimeChannelError ? <Alert type="warning" showIcon message={runtimeChannelError} /> : null}
              </Space>
            </Card>
            <Card title="视频研究参数">
              {researchParams ? (
                <Space wrap>
                  <InputNumber aria-label="视频亮度" addonBefore="亮度" addonAfter="%" value={researchParams.video.brightness_percent} min={-100} max={100} onChange={(value) => updateResearchParam('video', 'brightness_percent', value)} />
                  <InputNumber aria-label="视频对比度" addonBefore="对比度" addonAfter="%" value={researchParams.video.contrast_percent} min={0} max={200} onChange={(value) => updateResearchParam('video', 'contrast_percent', value)} />
                  <InputNumber aria-label="视频饱和度" addonBefore="饱和度" addonAfter="%" value={researchParams.video.saturation_percent} min={0} max={200} onChange={(value) => updateResearchParam('video', 'saturation_percent', value)} />
                  <InputNumber aria-label="视频色相" addonBefore="色相" addonAfter="°" value={researchParams.video.hue_rotation_degrees} min={-180} max={180} onChange={(value) => updateResearchParam('video', 'hue_rotation_degrees', value)} />
                  <InputNumber aria-label="视频像素缩放" addonBefore="像素缩放" addonAfter="%" value={researchParams.video.pixel_scale_percent} min={95} max={105} step={0.1} onChange={(value) => updateResearchParam('video', 'pixel_scale_percent', value)} />
                  <InputNumber aria-label="视频模糊半径" addonBefore="模糊" addonAfter="px" value={researchParams.video.blur_radius_px} min={0} max={8} step={0.1} onChange={(value) => updateResearchParam('video', 'blur_radius_px', value)} />
                  <InputNumber aria-label="视频锐化" addonBefore="锐化" addonAfter="%" value={researchParams.video.sharpen_percent} min={0} max={100} onChange={(value) => updateResearchParam('video', 'sharpen_percent', value)} />
                  <InputNumber aria-label="视频噪点" addonBefore="噪点" addonAfter="%" value={researchParams.video.noise_percent} min={0} max={8} step={0.1} onChange={(value) => updateResearchParam('video', 'noise_percent', value)} />
                  <InputNumber aria-label="视频细节增强" addonBefore="细节增强" addonAfter="%" value={researchParams.video.detail_enhancement_percent} min={0} max={50} step={0.1} onChange={(value) => updateResearchParam('video', 'detail_enhancement_percent', value)} />
                  <InputNumber aria-label="视频动态裁剪" addonBefore="动态裁剪" addonAfter="%/边" value={researchParams.video.dynamic_crop_percent} min={0} max={4} step={0.1} onChange={(value) => updateResearchParam('video', 'dynamic_crop_percent', value)} />
                  <InputNumber aria-label="视频像素扰动" addonBefore="像素扰动" addonAfter="px" value={researchParams.video.pixel_jitter_px} min={0} max={2} step={0.1} onChange={(value) => updateResearchParam('video', 'pixel_jitter_px', value)} />
                  <InputNumber aria-label="视频 X 轴偏移" addonBefore="X 偏移" addonAfter="px" value={researchParams.video.space_x_offset_px} min={-4} max={4} step={0.1} onChange={(value) => updateResearchParam('video', 'space_x_offset_px', value)} />
                  <InputNumber aria-label="视频 Y 轴偏移" addonBefore="Y 偏移" addonAfter="px" value={researchParams.video.space_y_offset_px} min={-4} max={4} step={0.1} onChange={(value) => updateResearchParam('video', 'space_y_offset_px', value)} />
                  <InputNumber aria-label="研究切片间隔" addonBefore="切片间隔" addonAfter="ms" value={researchParams.research.slice_trigger_interval_ms} min={5_000} max={120_000} onChange={(value) => updateResearchParam('research', 'slice_trigger_interval_ms', value)} />
                </Space>
              ) : (
                <Alert type="info" showIcon message="正在读取视频研究参数…" />
              )}
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
                  <Button loading={cacheCleanupBusy} onClick={() => void cleanupLocalCaches()} disabled={cacheCleanupBusy}>清理本地缓存</Button>
                  {cacheCleanup ? (
                    <Tag>
                      已清理 {cacheCleanup.removed_files} 个文件 / {(cacheCleanup.removed_bytes / 1024 / 1024).toFixed(1)} MB
                    </Tag>
                  ) : null}
                </Space>
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
    <ConfigProvider>
      <AntApp>{isFinalEffectWindow ? <FinalEffectWindow /> : <DesktopApp />}</AntApp>
    </ConfigProvider>
  );
}
