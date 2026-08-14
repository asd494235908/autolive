import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Alert, Button, Card, Checkbox, Descriptions, InputNumber, Layout, Select, Space, Switch, Tag, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';

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

const isFinalEffectWindow =
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('view') === 'final-effect';

function toAssetUrl(path: string | null | undefined) {
  if (!path) return null;
  return convertFileSrc(path.replace(/^file:\/\//, ''));
}

function FinalEffectWindow() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const waveformCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const spectrumCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const videoSourceNodeRef = useRef<MediaElementAudioSourceNode | null>(null);
  const audioSourceNodeRef = useRef<MediaElementAudioSourceNode | null>(null);
  const videoGainNodeRef = useRef<GainNode | null>(null);
  const audioGainNodeRef = useRef<GainNode | null>(null);
  const diagnosticFrameRef = useRef<number | null>(null);
  const suppressMediaEventRef = useRef(false);
  const [sourceUrl, setSourceUrl] = useState<string | null>(() => {
    const path = window.localStorage.getItem('autolive.source.path');
    return toAssetUrl(path);
  });
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [workerCapabilities, setWorkerCapabilities] = useState<SpeechToSpeechWorkerCapabilities | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const nextSegmentStartRef = useRef<number | null>(null);
  const scheduleGenerationRef = useRef<number | null>(null);
  const scheduleLoopRef = useRef<number | null>(null);
  const snapshotRef = useRef<PlaybackSnapshot | null>(null);
  const workerAvailableRef = useRef(false);
  const [audioDiagnosticsReady, setAudioDiagnosticsReady] = useState(false);

  function resumeAudioDiagnostics() {
    void audioContextRef.current?.resume().catch(() => undefined);
  }

  useEffect(() => {
    snapshotRef.current = snapshot;
  }, [snapshot]);

  useEffect(() => {
    workerAvailableRef.current = Boolean(workerCapabilities?.available);
  }, [workerCapabilities?.available]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void invoke<PlaybackSnapshot>('get_snapshot').then(setSnapshot).catch(() => undefined);
    }, 500);
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
      videoSourceNodeRef.current = videoSource;
      audioSourceNodeRef.current = audioSource;
      videoGainNodeRef.current = videoGain;
      audioGainNodeRef.current = audioGain;
      setAudioDiagnosticsReady(true);
    } catch {
      setAudioDiagnosticsReady(false);
    }

    return () => {
      if (diagnosticFrameRef.current !== null) {
        window.cancelAnimationFrame(diagnosticFrameRef.current);
        diagnosticFrameRef.current = null;
      }
      audioContextRef.current?.close().catch(() => undefined);
      audioContextRef.current = null;
      analyserRef.current = null;
      videoSourceNodeRef.current = null;
      audioSourceNodeRef.current = null;
      videoGainNodeRef.current = null;
      audioGainNodeRef.current = null;
    };
  }, [sourceUrl]);

  useEffect(() => {
    const analyser = analyserRef.current;
    const waveformCanvas = waveformCanvasRef.current;
    const spectrumCanvas = spectrumCanvasRef.current;
    if (!analyser || !waveformCanvas || !spectrumCanvas) return;
    const waveformContext = waveformCanvas.getContext('2d');
    const spectrumContext = spectrumCanvas.getContext('2d');
    if (!waveformContext || !spectrumContext) return;

    const waveform = new Uint8Array(analyser.fftSize);
    const spectrum = new Uint8Array(analyser.frequencyBinCount);
    const draw = () => {
      analyser.getByteTimeDomainData(waveform);
      analyser.getByteFrequencyData(spectrum);
      const drawCanvas = (canvas: HTMLCanvasElement, context: CanvasRenderingContext2D) => {
        const ratio = window.devicePixelRatio || 1;
        const width = Math.max(1, Math.floor(canvas.clientWidth * ratio));
        const height = Math.max(1, Math.floor(canvas.clientHeight * ratio));
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        context.clearRect(0, 0, width, height);
        context.fillStyle = '#0b1220';
        context.fillRect(0, 0, width, height);
        return { width, height };
      };

      const waveformSize = drawCanvas(waveformCanvas, waveformContext);
      waveformContext.strokeStyle = '#22d3ee';
      waveformContext.lineWidth = Math.max(1, window.devicePixelRatio || 1);
      waveformContext.beginPath();
      waveform.forEach((value, index) => {
        const x = (index / (waveform.length - 1)) * waveformSize.width;
        const y = (value / 255) * waveformSize.height;
        if (index === 0) waveformContext.moveTo(x, y);
        else waveformContext.lineTo(x, y);
      });
      waveformContext.stroke();

      const spectrumSize = drawCanvas(spectrumCanvas, spectrumContext);
      const barWidth = spectrumSize.width / spectrum.length;
      spectrum.forEach((value, index) => {
        const barHeight = (value / 255) * spectrumSize.height;
        spectrumContext.fillStyle = `hsl(${185 + (index / spectrum.length) * 90} 85% 55%)`;
        spectrumContext.fillRect(index * barWidth, spectrumSize.height - barHeight, Math.max(1, barWidth), barHeight);
      });
      diagnosticFrameRef.current = window.requestAnimationFrame(draw);
    };
    draw();
    return () => {
      if (diagnosticFrameRef.current !== null) {
        window.cancelAnimationFrame(diagnosticFrameRef.current);
        diagnosticFrameRef.current = null;
      }
    };
  }, [audioDiagnosticsReady, sourceUrl]);

  useEffect(() => {
    const gain = snapshot?.audio_processing_runtime
      ? Math.pow(10, (snapshot.audio_processing_gain_db ?? 0) / 20)
      : 1;
    const hasVariant = Boolean(audioUrl);
    if (videoGainNodeRef.current) {
      videoGainNodeRef.current.gain.value = hasVariant ? 0 : gain;
    }
    if (audioGainNodeRef.current) {
      audioGainNodeRef.current.gain.value = hasVariant ? gain : 0;
    }
  }, [audioUrl, snapshot?.audio_processing_gain_db, snapshot?.audio_processing_runtime]);

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
    const video = videoRef.current;
    if (!video || !sourceUrl) return;
    video.currentTime = 0;
    if (snapshot?.playback_state === 'playing') {
      void video.play().catch(() => undefined);
    }
  }, [sourceUrl]);

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
    if (!audioUrl || !audio) {
      if (audio) {
        audio.pause();
        audio.removeAttribute('src');
        audio.load();
      }
      video.muted = false;
      return;
    }
    audio.src = audioUrl;
    audio.currentTime = Math.max(0, video.currentTime - (snapshot?.current_audio_start_at_ms ?? 0) / 1000);
    video.muted = true;
    if (!video.paused) void audio.play().catch(() => undefined);
  }, [audioUrl, snapshot?.current_audio_start_at_ms]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !snapshot?.pending_audio_candidate) return;
    const commitIfDue = () => {
      void invoke<PlaybackSnapshot>('commit_audio_variant_candidate_if_due', {
        request: { position_ms: Math.max(0, Math.round(video.currentTime * 1000)) },
      }).then(setSnapshot).catch(() => undefined);
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
      }).then((result) => setSnapshot(result.snapshot)).catch(() => {
        nextSegmentStartRef.current = startAtMs;
      });
    }, 250);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <Layout style={{ minHeight: '100vh', background: '#111827' }}>
      <Layout.Content style={{ padding: 24 }}>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          <Space style={{ width: '100%', justifyContent: 'space-between' }}>
            <Typography.Title level={3} style={{ color: '#fff', margin: 0 }}>
              最终效果
            </Typography.Title>
            <Tag color={snapshot?.playback_state === 'playing' ? 'green' : 'default'}>
              {snapshot?.playback_state ?? '等待播放'} · 第 {snapshot?.loop_index ?? 0} 轮
            </Tag>
          </Space>
          <Descriptions size="small" column={2} style={{ color: '#fff' }}>
            <Descriptions.Item label="当前音轨">
              {snapshot?.current_audio_source ?? '未设置'}
              {snapshot?.current_audio_reference ? ` · ${snapshot.current_audio_reference}` : ''}
            </Descriptions.Item>
            <Descriptions.Item label="幻化 Worker">
              {workerCapabilities?.available
                ? `${workerCapabilities.provider}/${workerCapabilities.model} · ${snapshot?.worker_status ?? 'idle'}`
                : `不可用：${workerCapabilities?.reason ?? '未完成本地能力探测'}`}
            </Descriptions.Item>
            <Descriptions.Item label="音频决策">{snapshot?.audio_decision ?? 'keep_original'}</Descriptions.Item>
            <Descriptions.Item label="回退原因">{snapshot?.fallback_reason ?? '无'}</Descriptions.Item>
            <Descriptions.Item label="候选音轨">{snapshot?.pending_audio_candidate ? '待切换' : '无待切换候选'}</Descriptions.Item>
            <Descriptions.Item label="声音处理">
              {snapshot?.audio_processing_status ?? 'disabled'} · {snapshot?.audio_processing_parameters_version ?? 'audio_processing_v1'}
            </Descriptions.Item>
            <Descriptions.Item label="视频处理">{snapshot?.video_processing_status ?? 'disabled'}</Descriptions.Item>
          </Descriptions>
          <Space wrap>
            <Button
              onClick={() => {
                suppressMediaEventRef.current = true;
                void invoke<PlaybackSnapshot>('pause_playback')
                  .then((nextSnapshot) => {
                    videoRef.current?.pause();
                    audioRef.current?.pause();
                    setSnapshot(nextSnapshot);
                  })
                  .catch(() => {
                    suppressMediaEventRef.current = false;
                  });
              }}
              disabled={snapshot?.playback_state !== 'playing'}
            >
              暂停
            </Button>
            <Button
              onClick={() => {
                suppressMediaEventRef.current = true;
                void invoke<PlaybackSnapshot>('resume_playback')
                  .then((nextSnapshot) => {
                    resumeAudioDiagnostics();
                    void videoRef.current?.play();
                    void audioRef.current?.play();
                    setSnapshot(nextSnapshot);
                  })
                  .catch(() => {
                    suppressMediaEventRef.current = false;
                  });
              }}
              disabled={!['paused', 'ready'].includes(snapshot?.playback_state ?? '')}
            >
              继续
            </Button>
            <Button
              danger
              onClick={() => {
                suppressMediaEventRef.current = true;
                videoRef.current?.pause();
                audioRef.current?.pause();
                if (videoRef.current) videoRef.current.currentTime = 0;
                void invoke<PlaybackSnapshot>('stop_playback')
                  .then(setSnapshot)
                  .catch(() => {
                    suppressMediaEventRef.current = false;
                  });
              }}
              disabled={snapshot?.playback_state === 'stopped' || snapshot === null}
            >
              停止
            </Button>
          </Space>
          <Card>
            {sourceUrl ? (
              <>
                <video
                  ref={videoRef}
                  src={sourceUrl}
                  controls
                  autoPlay
                  playsInline
                  muted={Boolean(audioUrl) && !audioDiagnosticsReady}
                  onClick={resumeAudioDiagnostics}
                  onPlay={() => {
                    resumeAudioDiagnostics();
                    if (suppressMediaEventRef.current) {
                      suppressMediaEventRef.current = false;
                      return;
                    }
                    if (snapshot?.playback_state === 'paused' || snapshot?.playback_state === 'ready') {
                      void invoke<PlaybackSnapshot>('resume_playback').then(setSnapshot).catch(() => undefined);
                    }
                  }}
                  onPause={() => {
                    if (suppressMediaEventRef.current) {
                      suppressMediaEventRef.current = false;
                      return;
                    }
                    if (snapshot?.playback_state === 'playing') {
                      void invoke<PlaybackSnapshot>('pause_playback').then(setSnapshot).catch(() => undefined);
                    }
                  }}
                  onEnded={(event) => {
                    const previousVideoReference = snapshotRef.current?.current_video_reference;
                    void invoke<PlaybackSnapshot>('complete_playback_loop')
                      .then((nextSnapshot) => {
                        setSnapshot(nextSnapshot);
                        if (nextSnapshot.current_video_reference === previousVideoReference) {
                          event.currentTarget.currentTime = 0;
                          void event.currentTarget.play().catch(() => undefined);
                          if (audioRef.current) {
                            audioRef.current.currentTime = 0;
                            void audioRef.current.play().catch(() => undefined);
                          }
                        }
                      })
                      .catch(() => undefined);
                  }}
                  style={{ width: '100%', maxHeight: 'calc(100vh - 180px)', background: '#000' }}
                />
                <audio
                  ref={audioRef}
                  src={audioUrl ?? undefined}
                  onEnded={() => {
                    const currentSnapshot = snapshotRef.current;
                    if (
                      currentSnapshot?.current_audio_source === 'realtime_variant' &&
                      !currentSnapshot.pending_audio_candidate
                    ) {
                      void invoke<PlaybackSnapshot>('restore_original_audio')
                        .then(setSnapshot)
                        .catch(() => undefined);
                    }
                  }}
                  hidden
                />
              </>
            ) : (
              <Alert type="info" showIcon message="请从主窗口导入视频后开始播放。" />
            )}
          </Card>
          <Card size="small" title="实时音频诊断">
            <Space direction="vertical" style={{ width: '100%' }}>
              <Tag color={audioDiagnosticsReady ? 'green' : 'orange'}>
                {audioDiagnosticsReady ? '波形/频谱已连接当前播放音频' : '浏览器音频分析不可用'}
              </Tag>
              <canvas ref={waveformCanvasRef} aria-label="实时波形" style={{ display: 'block', width: '100%', height: 96 }} />
              <canvas ref={spectrumCanvasRef} aria-label="频谱分析" style={{ display: 'block', width: '100%', height: 96 }} />
            </Space>
          </Card>
        </Space>
      </Layout.Content>
    </Layout>
  );
}

function DesktopApp() {
  const [probe, setProbe] = useState<MediaProbeResult | null>(null);
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
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

  useEffect(() => {
    const timer = window.setInterval(() => {
      void invoke<PlaybackSnapshot>('get_snapshot').then(setSnapshot).catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, []);

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
      await invoke('open_final_effect_window');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '导入视频失败');
    }
  }

  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Layout.Content style={{ maxWidth: 960, width: '100%', margin: '0 auto', padding: 32 }}>
        <Space direction="vertical" size="large" style={{ width: '100%' }}>
          <Typography.Title>autoLive 桌面端</Typography.Title>
          <Alert
            type="info"
            showIcon
            message="单源循环播放"
            description="导入一个 MP4 后，在同一个最终效果窗口内持续循环；不生成 N 个离线视频，不创建版本队列。"
          />
          <Button type="primary" size="large" onClick={() => void importVideo()}>
            导入视频并播放
          </Button>
          {error ? <Alert type="error" showIcon message={error} /> : null}
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
