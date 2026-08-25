import { AUDIO_MIX_PICK_HARD_MAX } from './runtime-parameter-scheduler';

export type AudioCapabilityStatus = 'disabled' | 'configured' | 'runtime' | 'ready' | 'unsupported';

export type AudioCapabilityValues = {
  input_gain_db?: number | null;
  output_gain_db?: number | null;
  loudness_adjustment_db?: number | null;
  low_eq_db?: number | null;
  mid_eq_db?: number | null;
  high_eq_db?: number | null;
  pitch_shift_semitones?: number | null;
  formant_shift_percent?: number | null;
  playback_speed?: number | null;
  fade_in_ms?: number | null;
  fade_out_ms?: number | null;
  reverb_wet_percent?: number | null;
  noise_reduction_percent?: number | null;
  phase_perturbation_percent?: number | null;
  vibrato_frequency_hz?: number | null;
  vibrato_depth_percent?: number | null;
  environment_noise_percent?: number | null;
  environment_noise_dbfs?: number | null;
  filter_q?: number | null;
  dry_wet_percent?: number | null;
  ambient_sound_mix_percent?: number | null;
  spectral_perturbation_percent?: number | null;
  high_frequency_perturbation_enabled?: boolean | null;
  high_frequency_perturbation_interval_ms?: number | null;
  high_frequency_perturbation_strength_percent?: number | null;
  high_frequency_perturbation_level_db?: number | null;
  sample_rate_hz?: number | null;
  output_bitrate_kbps?: number | null;
};

export type AudioCapabilityRow = {
  key: string;
  label: string;
  value: number | null;
  unit: string;
  status: AudioCapabilityStatus;
  description: string;
};

const REQUIRED_AUDIO_CAPABILITY_VALUE_FIELDS = [
  'input_gain_db',
  'output_gain_db',
  'loudness_adjustment_db',
  'low_eq_db',
  'mid_eq_db',
  'high_eq_db',
  'pitch_shift_semitones',
  'formant_shift_percent',
  'playback_speed',
  'fade_in_ms',
  'fade_out_ms',
  'reverb_wet_percent',
  'noise_reduction_percent',
  'phase_perturbation_percent',
  'vibrato_frequency_hz',
  'vibrato_depth_percent',
  'environment_noise_percent',
  'environment_noise_dbfs',
  'filter_q',
  'dry_wet_percent',
  'ambient_sound_mix_percent',
  'spectral_perturbation_percent',
  'output_bitrate_kbps',
] as const satisfies readonly (keyof AudioCapabilityValues)[];

const CONFIGURED_DESCRIPTION = '参数已保存，等待应用本地声音处理链';
const RUNTIME_DESCRIPTION = '当前声音处理链已确认生效';
const SIGNALSMITH_CONFIGURED_DESCRIPTION = 'Signalsmith Stretch 参数已保存，等待应用';
const SIGNALSMITH_RUNTIME_DESCRIPTION = 'Signalsmith Stretch 已在当前混音 PCM 生效';
const ATEMPO_CONFIGURED_DESCRIPTION = 'FFmpeg atempo 参数已保存，等待应用';
const ATEMPO_RUNTIME_DESCRIPTION = 'FFmpeg atempo 已在当前声音处理链生效';

function finiteOrNull(value: number | null | undefined): number | null {
  return value !== null && value !== undefined && Number.isFinite(value) ? value : null;
}

export function buildAudioCapabilityRows(
  audio: AudioCapabilityValues | null,
  enabled: boolean,
  runtimeActive: boolean,
): AudioCapabilityRow[] {
  const runtimeStatus: AudioCapabilityStatus = !enabled ? 'disabled' : runtimeActive ? 'ready' : 'configured';
  const configuredStatus: AudioCapabilityStatus = enabled ? 'configured' : 'disabled';
  const values = audio ?? {};
  const gainParts = [values.input_gain_db, values.output_gain_db, values.loudness_adjustment_db]
    .filter((value): value is number => value !== null && value !== undefined && Number.isFinite(value));
  const totalGain = gainParts.length > 0 ? gainParts.reduce((sum, value) => sum + value, 0) : null;

  return [
    { key: 'total_gain_db', label: '总增益', value: totalGain, unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'input_gain_db', label: '输入增益', value: finiteOrNull(values.input_gain_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'output_gain_db', label: '输出增益', value: finiteOrNull(values.output_gain_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'loudness_adjustment_db', label: '响度调整', value: finiteOrNull(values.loudness_adjustment_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'low_eq_db', label: '低频 EQ', value: finiteOrNull(values.low_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'mid_eq_db', label: '中频 EQ', value: finiteOrNull(values.mid_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'high_eq_db', label: '高频 EQ', value: finiteOrNull(values.high_eq_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'pitch_shift_semitones', label: '高质量音高', value: finiteOrNull(values.pitch_shift_semitones), unit: '半音', status: runtimeStatus, description: runtimeStatus === 'ready' ? SIGNALSMITH_RUNTIME_DESCRIPTION : SIGNALSMITH_CONFIGURED_DESCRIPTION },
    { key: 'playback_speed', label: '播放速度', value: finiteOrNull(values.playback_speed), unit: 'x', status: runtimeStatus, description: runtimeStatus === 'ready' ? ATEMPO_RUNTIME_DESCRIPTION : ATEMPO_CONFIGURED_DESCRIPTION },
    { key: 'formant_shift_percent', label: '共振峰偏移', value: finiteOrNull(values.formant_shift_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? SIGNALSMITH_RUNTIME_DESCRIPTION : SIGNALSMITH_CONFIGURED_DESCRIPTION },
    { key: 'fade_in_ms', label: '淡入', value: finiteOrNull(values.fade_in_ms), unit: 'ms', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'fade_out_ms', label: '淡出', value: finiteOrNull(values.fade_out_ms), unit: 'ms', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'reverb_wet_percent', label: '轻混响', value: finiteOrNull(values.reverb_wet_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'noise_reduction_percent', label: '降噪', value: finiteOrNull(values.noise_reduction_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'phase_perturbation_percent', label: '相位扰动', value: finiteOrNull(values.phase_perturbation_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'vibrato_frequency_hz', label: '颤音频率', value: finiteOrNull(values.vibrato_frequency_hz), unit: 'Hz', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'vibrato_depth_percent', label: '颤音深度', value: finiteOrNull(values.vibrato_depth_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'environment_noise_percent', label: '环境噪声', value: finiteOrNull(values.environment_noise_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'environment_noise_dbfs', label: '环境噪声电平', value: finiteOrNull(values.environment_noise_dbfs), unit: 'dBFS', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'filter_q', label: '滤波 Q', value: finiteOrNull(values.filter_q), unit: '', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'dry_wet_percent', label: '干湿比', value: finiteOrNull(values.dry_wet_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'ambient_sound_mix_percent', label: '环境声混合', value: finiteOrNull(values.ambient_sound_mix_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? '真实环境声素材已在当前声音总线生效' : '比例已保存；非零时还需选择真实环境声素材' },
    { key: 'spectral_perturbation_percent', label: '频谱微扰', value: finiteOrNull(values.spectral_perturbation_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'high_frequency_perturbation_strength_percent', label: '高频扰动强度', value: finiteOrNull(values.high_frequency_perturbation_strength_percent), unit: '%', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'high_frequency_perturbation_level_db', label: '高频扰动电平', value: finiteOrNull(values.high_frequency_perturbation_level_db), unit: 'dB', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'high_frequency_perturbation_interval_ms', label: '高频扰动间隔', value: finiteOrNull(values.high_frequency_perturbation_interval_ms), unit: 'ms', status: runtimeStatus, description: runtimeStatus === 'ready' ? RUNTIME_DESCRIPTION : CONFIGURED_DESCRIPTION },
    { key: 'sample_rate_hz', label: '采样率', value: finiteOrNull(values.sample_rate_hz), unit: 'Hz', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
    { key: 'output_bitrate_kbps', label: '输出码率', value: finiteOrNull(values.output_bitrate_kbps), unit: 'kbps', status: configuredStatus, description: CONFIGURED_DESCRIPTION },
  ];
}

function isAudioCapabilityValues(value: unknown): value is AudioCapabilityValues {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const hasFiniteNumber = (field: keyof AudioCapabilityValues) => (
    Object.prototype.hasOwnProperty.call(record, field)
    && typeof record[field] === 'number'
    && Number.isFinite(record[field])
  );
  const sampleRate = record.sample_rate_hz;
  return REQUIRED_AUDIO_CAPABILITY_VALUE_FIELDS.every(hasFiniteNumber)
    && Object.prototype.hasOwnProperty.call(record, 'sample_rate_hz')
    && (sampleRate === null || (typeof sampleRate === 'number' && Number.isFinite(sampleRate)));
}

export function resolveAudioStreamBranches(
  params: AudioCapabilityValues | null | undefined,
  variants: readonly AudioCapabilityValues[] | null | undefined,
  runtimeActive: boolean,
): readonly AudioCapabilityValues[] {
  if (!runtimeActive || !isAudioCapabilityValues(params)) return [];
  if (variants === null || variants === undefined) return [params];
  if (!Array.isArray(variants)) return [];
  if (variants.length === 0) return [params];
  if (variants.length > AUDIO_MIX_PICK_HARD_MAX || !variants.every(isAudioCapabilityValues)) return [];
  return variants;
}
