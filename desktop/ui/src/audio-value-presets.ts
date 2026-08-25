export type NaturalVoiceMode = 'original' | 'natural_dynamic';

/** 单套预设的 35 个可写声音效果字段；不含全局周期和只读共振峰诊断值。 */
export type SubtleAudioSample = {
  natural_voice_mode: NaturalVoiceMode;
  pitch_shift_semitones: number;
  spectral_perturbation_percent: number;
  environment_noise_percent: number;
  environment_noise_dbfs: number;
  mfcc_shift_percent: number;
  phase_perturbation_percent: number;
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
  filter_q: number;
  sample_rate_hz: number | null;
  output_bitrate_kbps: number;
  voice_library_id: string | null;
  high_frequency_perturbation_enabled: boolean;
  high_frequency_perturbation_interval_ms: number;
  high_frequency_perturbation_strength_percent: number;
  high_frequency_perturbation_level_db: number;
};

export const AUDIO_PRESET_FIELDS = [
  'natural_voice_mode',
  'pitch_shift_semitones',
  'spectral_perturbation_percent',
  'environment_noise_percent',
  'environment_noise_dbfs',
  'mfcc_shift_percent',
  'phase_perturbation_percent',
  'loudness_adjustment_db',
  'input_gain_db',
  'output_gain_db',
  'playback_speed',
  'low_eq_db',
  'mid_eq_db',
  'high_eq_db',
  'noise_reduction_percent',
  'ambient_sound_mix_percent',
  'fade_in_ms',
  'fade_out_ms',
  'dry_wet_percent',
  'reverb_wet_percent',
  'mfcc_dimensions',
  'snr_variation_db',
  'formant_shift_percent',
  'vibrato_frequency_hz',
  'vibrato_depth_percent',
  'spectrum_blind_spot_percent',
  'snr_target_db',
  'filter_q',
  'sample_rate_hz',
  'output_bitrate_kbps',
  'voice_library_id',
  'high_frequency_perturbation_enabled',
  'high_frequency_perturbation_interval_ms',
  'high_frequency_perturbation_strength_percent',
  'high_frequency_perturbation_level_db',
] as const satisfies readonly (keyof SubtleAudioSample)[];

export type AudioValuePreset = {
  id: string;
  label: string;
  values: SubtleAudioSample;
};

function finiteClamp(value: number, fallback: number, minimum: number, maximum: number): number {
  return Number.isFinite(value) ? Math.min(maximum, Math.max(minimum, value)) : fallback;
}

function optionalClamp(value: number | null, minimum: number, maximum: number): number | null {
  return value !== null && Number.isFinite(value)
    ? Math.min(maximum, Math.max(minimum, value))
    : null;
}

function sanitizeVoiceLibraryId(value: string | null): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 && new TextEncoder().encode(trimmed).length <= 128 ? trimmed : null;
}

/** 预设进入 IPC 前的完整安全边界；显式重建对象以拒绝额外字段。 */
export function sanitizeAudioPresetValues(values: SubtleAudioSample): SubtleAudioSample {
  return {
    natural_voice_mode: values.natural_voice_mode === 'natural_dynamic' ? 'natural_dynamic' : 'original',
    pitch_shift_semitones: finiteClamp(values.pitch_shift_semitones, 0, -2, 2),
    spectral_perturbation_percent: finiteClamp(values.spectral_perturbation_percent, 0, 0, 10),
    environment_noise_percent: finiteClamp(values.environment_noise_percent, 0, 0, 100),
    environment_noise_dbfs: finiteClamp(values.environment_noise_dbfs, -40, -60, -20),
    mfcc_shift_percent: finiteClamp(values.mfcc_shift_percent, 0, -20, 20),
    phase_perturbation_percent: finiteClamp(values.phase_perturbation_percent, 0, -20, 20),
    loudness_adjustment_db: finiteClamp(values.loudness_adjustment_db, 0, -6, 6),
    input_gain_db: finiteClamp(values.input_gain_db, 0, -6, 6),
    output_gain_db: finiteClamp(values.output_gain_db, 0, -6, 6),
    playback_speed: finiteClamp(values.playback_speed, 1, 0.5, 2),
    low_eq_db: finiteClamp(values.low_eq_db, 0, -12, 12),
    mid_eq_db: finiteClamp(values.mid_eq_db, 0, -12, 12),
    high_eq_db: finiteClamp(values.high_eq_db, 0, -12, 12),
    noise_reduction_percent: finiteClamp(values.noise_reduction_percent, 0, 0, 100),
    ambient_sound_mix_percent: finiteClamp(values.ambient_sound_mix_percent, 0, 0, 100),
    fade_in_ms: Math.round(finiteClamp(values.fade_in_ms, 0, 0, 10_000)),
    fade_out_ms: Math.round(finiteClamp(values.fade_out_ms, 0, 0, 10_000)),
    dry_wet_percent: finiteClamp(values.dry_wet_percent, 0, 0, 100),
    reverb_wet_percent: finiteClamp(values.reverb_wet_percent, 0, 0, 20),
    mfcc_dimensions: Math.round(finiteClamp(values.mfcc_dimensions, 13, 1, 40)),
    snr_variation_db: finiteClamp(values.snr_variation_db, 0, -6, 6),
    formant_shift_percent: finiteClamp(values.formant_shift_percent, 0, -5, 5),
    vibrato_frequency_hz: finiteClamp(values.vibrato_frequency_hz, 5, 3, 8),
    vibrato_depth_percent: finiteClamp(values.vibrato_depth_percent, 0, 0, 3),
    spectrum_blind_spot_percent: finiteClamp(values.spectrum_blind_spot_percent, 0, 0, 5),
    snr_target_db: optionalClamp(values.snr_target_db, 0, 60),
    filter_q: finiteClamp(values.filter_q, 1, 0.3, 10),
    sample_rate_hz: values.sample_rate_hz === 44_100 || values.sample_rate_hz === 48_000
      ? values.sample_rate_hz
      : null,
    output_bitrate_kbps: Math.round(finiteClamp(values.output_bitrate_kbps, 192, 64, 320)),
    voice_library_id: sanitizeVoiceLibraryId(values.voice_library_id),
    high_frequency_perturbation_enabled: values.high_frequency_perturbation_enabled === true,
    high_frequency_perturbation_interval_ms: Math.round(finiteClamp(
      values.high_frequency_perturbation_interval_ms,
      12_000,
      500,
      60_000,
    )),
    high_frequency_perturbation_strength_percent: finiteClamp(
      values.high_frequency_perturbation_strength_percent,
      0,
      0,
      20,
    ),
    high_frequency_perturbation_level_db: finiteClamp(
      values.high_frequency_perturbation_level_db,
      -32,
      -60,
      0,
    ),
  };
}

function roundTo(value: number, digits: number): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

/** 1–20 使用同一低感知标定曲线；返回对象逐项写出全部 35 个字段，不做默认值合并。 */
function buildLowPerceptionValues(index: number): SubtleAudioSample {
  const direction = index % 2 === 0 ? 1 : -1;
  const band = ((index - 1) % 5) + 1;
  const cycle = ((index - 1) % 4) + 1;
  const mfccDimensions = [12, 14, 15, 16][cycle - 1];
  return sanitizeAudioPresetValues({
    natural_voice_mode: 'natural_dynamic',
    pitch_shift_semitones: roundTo(direction * (0.012 + band * 0.006), 3),
    spectral_perturbation_percent: roundTo(0.08 + band * 0.025, 3),
    environment_noise_percent: roundTo(0.12 + cycle * 0.06, 2),
    environment_noise_dbfs: -58 + cycle,
    mfcc_shift_percent: roundTo(direction * (0.12 + band * 0.04), 2),
    phase_perturbation_percent: roundTo(-direction * (0.1 + cycle * 0.08), 2),
    loudness_adjustment_db: roundTo(direction * (0.025 + band * 0.012), 3),
    input_gain_db: roundTo(direction * (0.04 + cycle * 0.02), 2),
    output_gain_db: roundTo(-direction * (0.03 + cycle * 0.015), 3),
    playback_speed: roundTo(1 + direction * (0.0008 + cycle * 0.0004), 4),
    low_eq_db: roundTo(direction * (0.08 + band * 0.035), 3),
    mid_eq_db: roundTo(-direction * (0.06 + cycle * 0.03), 2),
    high_eq_db: roundTo(direction * (0.07 + cycle * 0.025), 2),
    noise_reduction_percent: roundTo(0.2 + band * 0.12, 2),
    ambient_sound_mix_percent: roundTo(0.08 + cycle * 0.06, 2),
    fade_in_ms: 8 + index,
    fade_out_ms: 12 + index,
    dry_wet_percent: roundTo(0.2 + band * 0.11, 2),
    reverb_wet_percent: roundTo(0.15 + cycle * 0.16, 2),
    mfcc_dimensions: mfccDimensions,
    snr_variation_db: roundTo(direction * (0.06 + cycle * 0.04), 2),
    formant_shift_percent: roundTo(-direction * (0.06 + band * 0.045), 3),
    vibrato_frequency_hz: roundTo(4.8 + cycle * 0.08, 2),
    vibrato_depth_percent: roundTo(0.04 + band * 0.025, 3),
    spectrum_blind_spot_percent: roundTo(0.04 + cycle * 0.025, 3),
    snr_target_db: null,
    filter_q: roundTo(1 + direction * (0.01 + cycle * 0.005), 3),
    sample_rate_hz: index % 2 === 0 ? 48_000 : 44_100,
    output_bitrate_kbps: 184 + band,
    voice_library_id: `gpal-subtle-p${String(index).padStart(2, '0')}`,
    high_frequency_perturbation_enabled: true,
    high_frequency_perturbation_interval_ms: 8_100 + index * 237,
    high_frequency_perturbation_strength_percent: roundTo(0.06 + band * 0.04, 2),
    high_frequency_perturbation_level_db: -58 + cycle,
  });
}

const LOW_PERCEPTION_LABELS = [
  '1. 自然平直',
  '2. 微抬增益',
  '3. 微降增益',
  '4. 暖低频',
  '5. 亮高频',
  '6. 中频突出',
  '7. 轻上移音高',
  '8. 轻下移音高',
  '9. 轻颤音',
  '10. 相位微扰',
  '11. 轻混响',
  '12. 干声收紧',
  '13. 轻降噪',
  '14. 底噪纹理',
  '15. 淡入淡出',
  '16. 自然微变',
  '17. 音色着色',
  '18. 空间感',
  '19. 调制组合',
  '20. 综合微扰',
] as const;

const OBVIOUS_PRESET_21: AudioValuePreset = {
  id: 'p21',
  label: '21. 明显加轨与空间（手动）',
  values: sanitizeAudioPresetValues({
    natural_voice_mode: 'natural_dynamic',
    pitch_shift_semitones: 0.6,
    spectral_perturbation_percent: 3,
    environment_noise_percent: 30,
    environment_noise_dbfs: -28,
    mfcc_shift_percent: 6,
    phase_perturbation_percent: 8,
    loudness_adjustment_db: 3,
    input_gain_db: 4,
    output_gain_db: 3,
    playback_speed: 1.04,
    low_eq_db: 10,
    mid_eq_db: -8,
    high_eq_db: 10,
    noise_reduction_percent: 12,
    ambient_sound_mix_percent: 35,
    fade_in_ms: 180,
    fade_out_ms: 220,
    dry_wet_percent: 45,
    reverb_wet_percent: 20,
    mfcc_dimensions: 20,
    snr_variation_db: 2,
    formant_shift_percent: 2,
    vibrato_frequency_hz: 6.8,
    vibrato_depth_percent: 1.4,
    spectrum_blind_spot_percent: 2.5,
    snr_target_db: 20,
    filter_q: 6,
    sample_rate_hz: 48_000,
    output_bitrate_kbps: 256,
    voice_library_id: 'gpal-obvious-space',
    high_frequency_perturbation_enabled: true,
    high_frequency_perturbation_interval_ms: 1_200,
    high_frequency_perturbation_strength_percent: 10,
    high_frequency_perturbation_level_db: -20,
  }),
};

const OBVIOUS_PRESET_22: AudioValuePreset = {
  id: 'p22',
  label: '22. 明显变调与音色（手动）',
  values: sanitizeAudioPresetValues({
    natural_voice_mode: 'natural_dynamic',
    pitch_shift_semitones: 2,
    spectral_perturbation_percent: 8,
    environment_noise_percent: 12,
    environment_noise_dbfs: -32,
    mfcc_shift_percent: 20,
    phase_perturbation_percent: 15,
    loudness_adjustment_db: 1,
    input_gain_db: 1,
    output_gain_db: 0.5,
    playback_speed: 1.18,
    low_eq_db: -6,
    mid_eq_db: 5,
    high_eq_db: 6,
    noise_reduction_percent: 15,
    ambient_sound_mix_percent: 25,
    fade_in_ms: 80,
    fade_out_ms: 100,
    dry_wet_percent: 35,
    reverb_wet_percent: 12,
    mfcc_dimensions: 28,
    snr_variation_db: 5,
    formant_shift_percent: 5,
    vibrato_frequency_hz: 7.5,
    vibrato_depth_percent: 3,
    spectrum_blind_spot_percent: 5,
    snr_target_db: 8,
    filter_q: 2.2,
    sample_rate_hz: 44_100,
    output_bitrate_kbps: 256,
    voice_library_id: 'gpal-obvious-pitch',
    high_frequency_perturbation_enabled: true,
    high_frequency_perturbation_interval_ms: 700,
    high_frequency_perturbation_strength_percent: 20,
    high_frequency_perturbation_level_db: -10,
  }),
};

const LOW_PERCEPTION_PRESETS = LOW_PERCEPTION_LABELS.map((label, offset): AudioValuePreset => {
  const index = offset + 1;
  return {
    id: `p${String(index).padStart(2, '0')}`,
    label,
    values: buildLowPerceptionValues(index),
  };
});

export const AUDIO_VALUE_PRESETS: readonly AudioValuePreset[] = [
  ...LOW_PERCEPTION_PRESETS,
  OBVIOUS_PRESET_21,
  OBVIOUS_PRESET_22,
];

export const DEFAULT_AUDIO_VALUE_PRESET_IDS: readonly string[] = AUDIO_VALUE_PRESETS
  .slice(0, 20)
  .map(({ id }) => id);

export const SELECTABLE_AUDIO_VALUE_PRESET_IDS: readonly string[] = AUDIO_VALUE_PRESETS
  .map(({ id }) => id);

export function getAudioValuePreset(id: string): AudioValuePreset {
  return AUDIO_VALUE_PRESETS.find((preset) => preset.id === id) ?? AUDIO_VALUE_PRESETS[0];
}

export function isSelectableAudioValuePresetId(id: string): boolean {
  return SELECTABLE_AUDIO_VALUE_PRESET_IDS.includes(id);
}
