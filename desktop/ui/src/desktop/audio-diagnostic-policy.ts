export const DIAGNOSTIC_PUBLISH_INTERVAL_MS = 250;
export const DIAGNOSTIC_UI_COMMIT_INTERVAL_MS = 1_000;
export const DIAGNOSTIC_STALE_AFTER_MS = 1_500;
export const DIAGNOSTIC_SAMPLE_COUNT = 128;
export const DIAGNOSTIC_LINE_SAMPLE_COUNT = 96;

export type DiagnosticSource = 'portaudio-mixed-pcm' | 'web-audio-analyser';

export type DiagnosticMessage = {
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

export function isDiagnosticMessage(value: unknown): value is DiagnosticMessage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const validSamples = (samples: unknown, maxLength = DIAGNOSTIC_SAMPLE_COUNT): samples is number[] =>
    Array.isArray(samples)
    && samples.length <= maxLength
    && samples.every((sample) => typeof sample === 'number' && Number.isFinite(sample) && sample >= -1 && sample <= 1);
  const nullableFiniteNumber = (sample: unknown): sample is number | null =>
    sample === null || (typeof sample === 'number' && Number.isFinite(sample));
  return (
    record.version === 1
    && record.type === 'diagnostic'
    && (record.source === 'portaudio-mixed-pcm' || record.source === 'web-audio-analyser')
    && (record.sequence === null || (typeof record.sequence === 'number' && Number.isSafeInteger(record.sequence)))
    && typeof record.has_pcm === 'boolean'
    && nullableFiniteNumber(record.sample_rate_hz)
    && nullableFiniteNumber(record.captured_frame_count)
    && nullableFiniteNumber(record.rms_dbfs)
    && nullableFiniteNumber(record.peak_dbfs)
    && nullableFiniteNumber(record.low_band_rms_dbfs)
    && nullableFiniteNumber(record.cutoff_hz)
    && nullableFiniteNumber(record.noise_floor_dbfs)
    && nullableFiniteNumber(record.snr_db)
    && nullableFiniteNumber(record.current_formant_hz)
    && typeof record.mfcc_available === 'boolean'
    && Array.isArray(record.mfcc)
    && record.mfcc.length <= 40
    && record.mfcc.every((coefficient) => typeof coefficient === 'number' && Number.isFinite(coefficient))
    && Array.isArray(record.formants_hz)
    && record.formants_hz.length === 3
    && record.formants_hz.every(nullableFiniteNumber)
    && typeof record.sent_at_ms === 'number'
    && Number.isFinite(record.sent_at_ms)
    && validSamples(record.line, DIAGNOSTIC_LINE_SAMPLE_COUNT)
    && (record.waveform === undefined || validSamples(record.waveform))
    && (record.spectrum === undefined || validSamples(record.spectrum))
    && (record.error === null || typeof record.error === 'string')
  );
}

export function selectDiagnosticMessage(
  current: DiagnosticMessage | null,
  incoming: DiagnosticMessage,
  nowMs: number,
): DiagnosticMessage {
  const currentPortAudioIsFresh = current?.source === 'portaudio-mixed-pcm'
    && current.line.length > 0
    && nowMs - current.sent_at_ms < DIAGNOSTIC_STALE_AFTER_MS;
  return currentPortAudioIsFresh && incoming.source === 'web-audio-analyser'
    ? current
    : incoming;
}

export function getDiagnosticStatus(message: DiagnosticMessage | null, fresh: boolean): string {
  if (message === null) return '无数据：尚未收到诊断消息';
  if (!fresh) return '数据已过期';
  const hasLine = message.line.length > 0;
  const hasPcm = message.source === 'portaudio-mixed-pcm' ? message.has_pcm : hasLine;
  if (!hasPcm) return '无数据：PortAudio 快照和 Web Audio 回退均不可用';
  return message.source === 'portaudio-mixed-pcm'
    ? '实时：来自 PortAudio 最终混音 PCM'
    : '兼容回退：PortAudio 快照不可用，来自 Web Audio analyser';
}
