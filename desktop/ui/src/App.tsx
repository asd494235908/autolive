import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Alert, Button, Card, Checkbox, Descriptions, InputNumber, Layout, Select, Slider, Space, Switch, Tag, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { buildRuntimePreviewParameters, isRuntimeVariationDue, normalizeRuntimeVariationPeriod } from './运行时参数自动调度';
import type { RuntimeBaseParameters, RuntimePreviewParameters } from './运行时参数自动调度';
import { shouldRestartPlayback } from './播放循环';
import { buildFinalEffectWindowResizeKey } from './最终效果窗口尺寸';
import { clampMediaTime, clampVolume, formatMediaTime, isPlaybackMediaControlMessage, isPlaybackMediaStateMessage } from './播放控制消息';
import type { PlaybackMediaControlMessage, PlaybackMediaStateMessage } from './播放控制消息';

const PLAYBACK_CHANNEL_NAME = 'autolive-playback-ui-v1';

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

function FinalEffectWindow() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const videoGainNodeRef = useRef<GainNode | null>(null);
  const audioGainNodeRef = useRef<GainNode | null>(null);
  const suppressMediaEventRef = useRef(false);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const userMutedRef = useRef(false);
  const userVolumeRef = useRef(1);
  const audioUrlRef = useRef<string | null>(null);
  const audioDiagnosticsReadyRef = useRef(false);
  const loopSourceKeyRef = useRef<string | null>(null);
  const loopGenerationRef = useRef<number | null>(null);
  const loopSequenceRef = useRef(0);
  const lastRestartTokenRef = useRef<string | number | null>(null);
  const [sourceUrl, setSourceUrl] = useState<string | null>(() => {
    const path = window.localStorage.getItem('autolive.source.path');
    return toAssetUrl(path);
  });
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
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

  function syncUserAudioSettings() {
    const video = videoRef.current;
    const audio = audioRef.current;
    const volume = userVolumeRef.current;
    const muted = userMutedRef.current;
    const candidateAudioActive = Boolean(audioUrlRef.current) && audioDiagnosticsReadyRef.current;
    if (video) {
      video.volume = volume;
      video.muted = candidateAudioActive || muted;
    }
    if (audio) {
      audio.volume = volume;
      audio.muted = muted;
    }
  }

  function publishMediaState() {
    const channel = playbackChannelRef.current;
    const video = videoRef.current;
    if (!channel || !video) return;
    const duration = Number.isFinite(video.duration) && video.duration >= 0 ? video.duration : 0;
    const currentTime = clampMediaTime(video.currentTime, duration);
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
        } else if (event.data.action === 'pause') {
          video?.pause();
          audio?.pause();
        } else if (video) {
          void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
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
        waveform: Array.from(waveform.slice(0, 64), (sample) => (sample - 128) / 128),
        spectrum: Array.from(spectrum.slice(0, 64), (sample) => sample / 255),
        sent_at_ms: Date.now(),
        error: playbackError,
      } satisfies DiagnosticMessage);
    }, 250);
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
    if (!sourceUrl || !videoRef.current || !audioRef.current || audioContextRef.current) return;
    try {
      const context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 2_048;
      analyser.smoothingTimeConstant = 0.75;
      const videoSource = context.createMediaElementSource(videoRef.current);
      const audioSource = context.createMediaElementSource(audioRef.current);
      const videoGain = context.createGain();
      const audioGain = context.createGain();
      videoSource.connect(videoGain).connect(analyser);
      audioSource.connect(audioGain).connect(analyser);
      analyser.connect(context.destination);
      audioContextRef.current = context;
      analyserRef.current = analyser;
      videoGainNodeRef.current = videoGain;
      audioGainNodeRef.current = audioGain;
      audioDiagnosticsReadyRef.current = true;
      setAudioDiagnosticsReady(true);
    } catch {
      setAudioDiagnosticsReady(false);
    }

    return () => {
      audioContextRef.current?.close().catch(() => undefined);
      audioContextRef.current = null;
      analyserRef.current = null;
      videoGainNodeRef.current = null;
      audioGainNodeRef.current = null;
      audioDiagnosticsReadyRef.current = false;
    };
  }, [sourceUrl]);

  useEffect(() => {
    const gainDb = runtimeAudioProcessingEnabled && runtimeParameters
      ? runtimeParameters.audio_gain_db
      : snapshot?.audio_processing_runtime
        ? snapshot.audio_processing_gain_db ?? 0
        : 0;
    const gain =
      Math.pow(10, gainDb / 20);
    const hasVariant = Boolean(audioUrl);
    if (videoGainNodeRef.current) {
      videoGainNodeRef.current.gain.value = hasVariant ? 0 : gain;
    }
    if (audioGainNodeRef.current) {
      audioGainNodeRef.current.gain.value = hasVariant ? gain : 0;
    }
  }, [audioUrl, runtimeAudioProcessingEnabled, runtimeParameters?.audio_gain_db, snapshot?.audio_processing_gain_db, snapshot?.audio_processing_runtime]);

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
    if (!video || !sourceUrl) return;
    if (snapshot?.playback_state === 'stopped') {
      video.pause();
      video.currentTime = 0;
      audio?.pause();
      if (audio) audio.currentTime = 0;
      return;
    }
    if (snapshot?.playback_state === 'paused') {
      video.pause();
      audio?.pause();
      return;
    }
    if (snapshot?.playback_state === 'playing' && video.paused) {
      void video.play().catch(() => setPlaybackError('播放已恢复，但系统阻止了自动播放，请点击视频播放。'));
      if (audioUrl && audioDiagnosticsReady && audio) void audio.play().catch(() => undefined);
    }
  }, [audioDiagnosticsReady, audioUrl, snapshot?.playback_state, sourceUrl]);

  useEffect(() => {
    if (!snapshot || !sourceUrl) return;
    const reference = snapshot.current_audio_source === 'realtime_variant'
      ? snapshot.current_audio_reference
      : null;
    setAudioUrl(reference ? convertFileSrc(reference.replace(/^file:\/\//, '')) : null);
  }, [snapshot?.current_audio_source, snapshot?.current_audio_reference, sourceUrl]);

  useEffect(() => {
    const video = videoRef.current;
    const audio = audioRef.current;
    if (!video) return;
    audioUrlRef.current = audioUrl;
    audioDiagnosticsReadyRef.current = audioDiagnosticsReady;
    if (!audioUrl || !audio) {
      if (audio) {
        audio.pause();
        audio.removeAttribute('src');
        audio.load();
      }
      syncUserAudioSettings();
      return;
    }
    if (!audioDiagnosticsReady) {
      audio.pause();
      syncUserAudioSettings();
      return;
    }
    audio.src = audioUrl;
    audio.currentTime = Math.max(0, video.currentTime - (snapshot?.current_audio_start_at_ms ?? 0) / 1000);
    syncUserAudioSettings();
    if (!video.paused) void audio.play().catch(() => undefined);
  }, [audioDiagnosticsReady, audioUrl, snapshot?.current_audio_start_at_ms]);

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
      const safetyLeadMs = 3_000;
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
            timeout_ms: 2_000,
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

    suppressMediaEventRef.current = true;
    void video.play().catch(() => {
      suppressMediaEventRef.current = false;
      setPlaybackError('视频已回到开头，但自动播放失败，请点击视频播放。');
    });
    if (audioRef.current && audioUrl && audioDiagnosticsReady) {
      audioRef.current.currentTime = 0;
      void audioRef.current.play().catch(() => undefined);
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
                muted={Boolean(audioUrl) && audioDiagnosticsReady ? true : userMuted}
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
                muted={userMuted}
                onEnded={() => {
                  const currentSnapshot = snapshotRef.current;
                  if (
                    currentSnapshot?.current_audio_source === 'realtime_variant' &&
                    !currentSnapshot.pending_audio_candidate
                  ) {
                    void invoke<PlaybackSnapshot>('restore_original_audio')
                      .then(applyPlayerSnapshot)
                      .catch(() => undefined);
                  }
                }}
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
  const [playbackActionBusy, setPlaybackActionBusy] = useState<'pause' | 'resume' | 'stop' | null>(null);
  const [videoProcessingEnabled, setVideoProcessingEnabled] = useState(false);
  const [audioProcessingEnabled, setAudioProcessingEnabled] = useState(false);
  const [realtimeAudioVariantEnabled, setRealtimeAudioVariantEnabled] = useState(false);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
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
  const playbackActionRequestRef = useRef(0);
  const playbackChannelRef = useRef<BroadcastChannel | null>(null);
  const pictureInPictureVideoRef = useRef<HTMLVideoElement | null>(null);
  const runtimeMessageRef = useRef<RuntimeParameterMessage | null>(null);
  const runtimeSchedulerRef = useRef({ cycle: 0, lastChangeMs: null as number | null });
  const waveformCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const spectrumCanvasRef = useRef<HTMLCanvasElement | null>(null);
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
  }, []);

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
    drawDiagnosticCanvas(waveformCanvasRef.current, diagnosticMessage?.waveform ?? [], '#22d3ee', 'rgba(34, 211, 238, 0.12)');
    drawDiagnosticCanvas(spectrumCanvasRef.current, diagnosticMessage?.spectrum ?? [], '#a78bfa', 'rgba(167, 139, 250, 0.12)');
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
    void invoke<MediaEngineCapabilities>('get_media_engine_capabilities')
      .then(setMediaEngineCapabilities)
      .catch(() => setMediaEngineCapabilities(null));
  }, []);

  useEffect(() => {
    void invoke<ResearchWorkerCapabilities>('get_research_worker_capabilities')
      .then(setResearchWorkerCapabilities)
      .catch(() => setResearchWorkerCapabilities(null));
    void invoke<ResearchStatus>('get_research_status')
      .then(setResearchStatus)
      .catch(() => setResearchStatus(null));
    const timer = window.setInterval(() => {
      void invoke<ResearchStatus>('get_research_status')
        .then(setResearchStatus)
        .catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void invoke<ResearchParams>('get_default_local_research_params')
      .then(setResearchParams)
      .catch(() => setResearchParams(null));
  }, []);

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

  useEffect(() => {
    void invoke<SpeechToSpeechWorkerCapabilities>('get_speech_to_speech_worker_capabilities')
      .then(setWorkerCapabilities)
      .catch(() => setWorkerCapabilities(null));
  }, []);

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

  async function openFinalEffectWindowFromHome() {
    setPlayerWindowBusy(true);
    setError(null);
    try {
      await invoke('open_final_effect_window');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '打开独立播放器失败');
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
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '启动本地媒体处理失败');
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

  async function importVideo() {
    const selected = await open({ multiple: false, filters: [{ name: 'MP4 视频', extensions: ['mp4'] }] });
    if (typeof selected !== 'string') return;
    setError(null);
    try {
      const result = await invoke<MediaProbeResult>('probe_local_mp4', { request: { path: selected } });
      setProbe(result);
      window.localStorage.setItem('autolive.source.path', result.canonical_path);
      const startedSnapshot = await invoke<PlaybackSnapshot>('start_playback');
      setSnapshot(startedSnapshot);
      setSnapshotLoading(false);
      setSnapshotFetchError(null);
      await openFinalEffectWindowFromHome();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '导入视频失败');
    }
  }

  const runtimeRemainingMs = runtimeLastChangeMs === null
    ? null
    : Math.max(0, runtimePeriodMs - (runtimeNowMs - runtimeLastChangeMs));
  const diagnosticFresh = diagnosticMessage !== null && Date.now() - diagnosticMessage.sent_at_ms < 1_500;
  const mediaDuration = mediaState?.duration ?? 0;
  const mediaCurrentTime = clampMediaTime(mediaState?.current_time ?? 0, mediaDuration);
  const pictureInPictureDocument = document as PictureInPictureDocument;
  const pictureInPictureSupported =
    Boolean(pictureInPictureSourceUrl) &&
    pictureInPictureDocument.pictureInPictureEnabled === true &&
    typeof (pictureInPictureVideoRef.current as PictureInPictureVideo | null)?.requestPictureInPicture === 'function';

    return (
    <Layout style={{ minHeight: '100vh' }}>
      <Layout.Content style={{ maxWidth: 1120, width: '100%', margin: '0 auto', padding: 32 }}>
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
        <Space direction="vertical" size="large" style={{ width: '100%' }}>
          <Typography.Title>autoLive 桌面端</Typography.Title>
          <Alert
            type="info"
            showIcon
            message="单源循环播放"
            description="导入一个 MP4 后，在同一个最终效果窗口内持续循环；不生成 N 个离线视频，不创建版本队列。"
          />
          <Space wrap>
            <Button type="primary" size="large" onClick={() => void importVideo()}>
              导入视频并播放
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
                <Descriptions.Item label="当前音轨">{snapshot?.current_audio_source ?? '—'}</Descriptions.Item>
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
                <Descriptions.Item label="时长">
                  {snapshot?.source_media?.duration_ms ?? probe?.source.duration_ms ?? '-'} ms
                </Descriptions.Item>
                <Descriptions.Item label="SHA-256">
                  {snapshot?.source_media?.mp4_sha256 ??
                    `计算中（${snapshot?.source_media?.mp4_hash_status ?? probe?.source.mp4_hash_status}）`}
                </Descriptions.Item>
              </Descriptions>
            </Card>
          ) : null}
          <Card title="处理开关">
            <Space direction="vertical" size="middle">
              <Switch
                checked={videoProcessingEnabled}
                onChange={(checked) =>
                  void updateProcessingSwitches({
                    video_processing_enabled: checked,
                    audio_processing_enabled: audioProcessingEnabled,
                    realtime_audio_variant_enabled: realtimeAudioVariantEnabled,
                  })
                }
              />{' '}
              视频处理
              <Switch
                checked={audioProcessingEnabled}
                onChange={(checked) =>
                  void updateProcessingSwitches({
                    video_processing_enabled: videoProcessingEnabled,
                    audio_processing_enabled: checked,
                    realtime_audio_variant_enabled: realtimeAudioVariantEnabled,
                  })
                }
              />{' '}
              声音处理
              <Switch
                checked={realtimeAudioVariantEnabled}
                onChange={(checked) =>
                  void updateProcessingSwitches({
                    video_processing_enabled: videoProcessingEnabled,
                    audio_processing_enabled: audioProcessingEnabled,
                    realtime_audio_variant_enabled: checked,
                  })
                }
              />{' '}
              实时话术幻化
            </Space>
            <Space wrap style={{ marginTop: 16 }}>
              <Button
                type="primary"
                onClick={() => void applyMediaProcessing()}
                loading={mediaProcessingBusy}
                disabled={
                  !snapshot?.source_media ||
                  !researchParams ||
                  (!videoProcessingEnabled && !audioProcessingEnabled) ||
                  snapshot.video_processing_status === 'processing' ||
                  snapshot.audio_processing_status === 'processing'
                  || mediaProcessingBusy
                }
              >
                应用当前处理参数
              </Button>
              <Tag color={mediaEngineCapabilities?.available ? 'green' : 'orange'}>
                媒体引擎：{mediaEngineCapabilities?.available ? '可用' : '不可用'}
              </Tag>
            </Space>
            <Alert
              style={{ marginTop: 16 }}
              type={mediaEngineCapabilities?.available ? 'success' : 'warning'}
              showIcon
              message={
                mediaEngineCapabilities?.available
                  ? 'FFmpeg/FFprobe 已就绪，处理结果将在当前视频下一轮开始时切换'
                  : `本地媒体引擎不可用：${mediaEngineCapabilities?.reason ?? '未完成能力探测'}`
              }
            />
            <Alert
              style={{ marginTop: 16 }}
              type={workerCapabilities?.available ? 'success' : 'warning'}
              showIcon
              message={
                workerCapabilities?.available
                  ? `本地话术 Worker：${workerCapabilities.provider}/${workerCapabilities.model}`
                  : `本地话术 Worker 不可用：${workerCapabilities?.reason ?? '未完成能力探测'}`
              }
            />
          </Card>
          <Card title="运行时参数预览">
            <Space direction="vertical" size="middle" style={{ width: '100%' }}>
              <Space wrap>
                <Tag color={videoProcessingEnabled ? 'green' : 'default'}>
                  视频动态：{videoProcessingEnabled ? '开启' : '关闭'}
                </Tag>
                <Tag color={audioProcessingEnabled ? 'green' : 'default'}>
                  声音动态：{audioProcessingEnabled ? '开启' : '关闭'}
                </Tag>
                <Tag>周期：{runtimePeriodMs} ms</Tag>
                <Tag>第 {runtimeCycle} 次变化</Tag>
                <Tag>下一次：{runtimeRemainingMs === null ? '未启动' : `${runtimeRemainingMs} ms`}</Tag>
              </Space>
              {runtimeActive && runtimeBaseParameters && runtimePreview ? (
                <Descriptions column={2} size="small">
                  <Descriptions.Item label="音频增益（配置 / 当前）">
                    {runtimeBaseParameters.audio_gain_db.toFixed(1)} / {runtimePreview.audio_gain_db.toFixed(1)} dB
                  </Descriptions.Item>
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
            title="本地研究参数"
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
            {researchParams ? (
              <Space wrap style={{ marginTop: 16 }}>
                <InputNumber
                  addonBefore="动态周期"
                  addonAfter="ms"
                  value={researchParams.audio.random_change_period_ms}
                  min={500}
                  max={60_000}
                  step={500}
                  onChange={(value) => updateResearchParam('audio', 'random_change_period_ms', value)}
                />
                <InputNumber
                  addonBefore="音高微移"
                  addonAfter="半音"
                  value={researchParams.audio.pitch_shift_semitones}
                  min={-2}
                  max={2}
                  step={0.1}
                  onChange={(value) => updateResearchParam('audio', 'pitch_shift_semitones', value)}
                />
                <InputNumber
                  addonBefore="MFCC"
                  addonAfter="%"
                  value={researchParams.audio.mfcc_shift_percent}
                  min={-20}
                  max={20}
                  onChange={(value) => updateResearchParam('audio', 'mfcc_shift_percent', value)}
                />
                <InputNumber
                  addonBefore="SNR 浮动"
                  addonAfter="dB"
                  value={researchParams.audio.snr_variation_db}
                  min={-6}
                  max={6}
                  onChange={(value) => updateResearchParam('audio', 'snr_variation_db', value)}
                />
                <InputNumber
                  addonBefore="输入增益"
                  addonAfter="dB"
                  value={researchParams.audio.input_gain_db}
                  min={-6}
                  max={6}
                  step={0.1}
                  onChange={(value) => updateResearchParam('audio', 'input_gain_db', value)}
                />
                <InputNumber
                  addonBefore="输出增益"
                  addonAfter="dB"
                  value={researchParams.audio.output_gain_db}
                  min={-6}
                  max={6}
                  step={0.1}
                  onChange={(value) => updateResearchParam('audio', 'output_gain_db', value)}
                />
                <InputNumber
                  addonBefore="响度"
                  addonAfter="dB"
                  value={researchParams.audio.loudness_adjustment_db}
                  min={-6}
                  max={6}
                  step={0.1}
                  onChange={(value) => updateResearchParam('audio', 'loudness_adjustment_db', value)}
                />
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
                <InputNumber
                  addonBefore="亮度"
                  addonAfter="%"
                  value={researchParams.video.brightness_percent}
                  min={-100}
                  max={100}
                  onChange={(value) => updateResearchParam('video', 'brightness_percent', value)}
                />
                <InputNumber
                  addonBefore="对比度"
                  addonAfter="%"
                  value={researchParams.video.contrast_percent}
                  min={0}
                  max={200}
                  onChange={(value) => updateResearchParam('video', 'contrast_percent', value)}
                />
                <InputNumber
                  addonBefore="饱和度"
                  addonAfter="%"
                  value={researchParams.video.saturation_percent}
                  min={0}
                  max={200}
                  onChange={(value) => updateResearchParam('video', 'saturation_percent', value)}
                />
                <InputNumber
                  addonBefore="色相"
                  addonAfter="°"
                  value={researchParams.video.hue_rotation_degrees}
                  min={-180}
                  max={180}
                  onChange={(value) => updateResearchParam('video', 'hue_rotation_degrees', value)}
                />
                <InputNumber
                  addonBefore="像素缩放"
                  addonAfter="%"
                  value={researchParams.video.pixel_scale_percent}
                  min={95}
                  max={105}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'pixel_scale_percent', value)}
                />
                <InputNumber
                  addonBefore="模糊"
                  addonAfter="px"
                  value={researchParams.video.blur_radius_px}
                  min={0}
                  max={8}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'blur_radius_px', value)}
                />
                <InputNumber
                  addonBefore="锐化"
                  addonAfter="%"
                  value={researchParams.video.sharpen_percent}
                  min={0}
                  max={100}
                  onChange={(value) => updateResearchParam('video', 'sharpen_percent', value)}
                />
                <InputNumber
                  addonBefore="噪点"
                  addonAfter="%"
                  value={researchParams.video.noise_percent}
                  min={0}
                  max={8}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'noise_percent', value)}
                />
                <InputNumber
                  addonBefore="细节增强"
                  addonAfter="%"
                  value={researchParams.video.detail_enhancement_percent}
                  min={0}
                  max={50}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'detail_enhancement_percent', value)}
                />
                <InputNumber
                  addonBefore="动态裁剪"
                  addonAfter="%/边"
                  value={researchParams.video.dynamic_crop_percent}
                  min={0}
                  max={4}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'dynamic_crop_percent', value)}
                />
                <InputNumber
                  addonBefore="像素扰动"
                  addonAfter="px"
                  value={researchParams.video.pixel_jitter_px}
                  min={0}
                  max={2}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'pixel_jitter_px', value)}
                />
                <InputNumber
                  addonBefore="X 偏移"
                  addonAfter="px"
                  value={researchParams.video.space_x_offset_px}
                  min={-4}
                  max={4}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'space_x_offset_px', value)}
                />
                <InputNumber
                  addonBefore="Y 偏移"
                  addonAfter="px"
                  value={researchParams.video.space_y_offset_px}
                  min={-4}
                  max={4}
                  step={0.1}
                  onChange={(value) => updateResearchParam('video', 'space_y_offset_px', value)}
                />
                <InputNumber
                  addonBefore="切片间隔"
                  addonAfter="ms"
                  value={researchParams.research.slice_trigger_interval_ms}
                  min={5_000}
                  max={120_000}
                  onChange={(value) => updateResearchParam('research', 'slice_trigger_interval_ms', value)}
                />
              </Space>
            ) : null}
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
        </Space>
      </Layout.Content>
    </Layout>
  );
}

export default function App() {
  return isFinalEffectWindow ? <FinalEffectWindow /> : <DesktopApp />;
}
