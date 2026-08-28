import {
  AUDIO_PRESET_FIELDS,
  AUDIO_VALUE_PRESETS,
} from '../../../../desktop/ui/src/audio-value-presets.ts';
import { AUDIO_PARAMETERS } from './prototype-data.js';

const LABELS = new Map(AUDIO_PARAMETERS.map(({ key, label }) => [key, label]));
const INTEGER_FIELDS = new Set([
  'fade_in_ms',
  'fade_out_ms',
  'mfcc_dimensions',
  'sample_rate_hz',
  'output_bitrate_kbps',
  'high_frequency_perturbation_interval_ms',
]);
const SIGNED_FIELDS = new Set([
  'pitch_shift_semitones',
  'mfcc_shift_percent',
  'phase_perturbation_percent',
  'loudness_adjustment_db',
  'input_gain_db',
  'output_gain_db',
  'low_eq_db',
  'mid_eq_db',
  'high_eq_db',
  'snr_variation_db',
  'formant_shift_percent',
]);

function unitFor(key) {
  if (key === 'pitch_shift_semitones') return '半音';
  if (key === 'environment_noise_dbfs') return 'dBFS';
  if (key === 'playback_speed') return '倍';
  if (key === 'mfcc_dimensions') return '阶';
  if (key.endsWith('_percent')) return '%';
  if (key.endsWith('_db')) return 'dB';
  if (key.endsWith('_ms')) return 'ms';
  if (key.endsWith('_hz')) return 'Hz';
  if (key.endsWith('_kbps')) return 'kbps';
  return '';
}

function formatNumber(key, value, includeSign = true) {
  const digits = INTEGER_FIELDS.has(key) ? 0 : key === 'playback_speed' ? 4 : 3;
  const formatted = value.toFixed(digits).replace(/\.0+$|(\.\d*?)0+$/, '$1');
  const prefix = includeSign && SIGNED_FIELDS.has(key) && value > 0 ? '+' : '';
  const unit = unitFor(key);
  return `${prefix}${formatted}${unit ? ` ${unit}` : ''}`;
}

function formatScalar(key, value) {
  if (value === null) return key === 'snr_target_db' ? '自动（源素材基线）' : '跟随源素材';
  if (typeof value === 'boolean') return value ? '开启' : '关闭';
  if (typeof value === 'string') {
    if (key === 'natural_voice_mode') return value === 'natural_dynamic' ? '自然动态' : '保持原声';
    return value;
  }
  return formatNumber(key, value);
}

function presetLabel(preset) {
  return `${preset.id} · ${preset.label.replace(/^\d+\.\s*/, '')}`;
}

export function getInterruptionPresetChanges(interruptionRuntime = {}) {
  const currentAudioPresetId = interruptionRuntime.currentAudioPresetId ?? null;
  const preset = AUDIO_VALUE_PRESETS.find(({ id }) => id === currentAudioPresetId) ?? null;
  const parameters = AUDIO_PRESET_FIELDS.map((key) => ({
    key,
    label: LABELS.get(key) ?? key,
    value: preset ? formatScalar(key, preset.values[key]) : '—',
  }));

  return {
    currentPreset: preset ? presetLabel(preset) : '等待首轮插话抽样',
    summary: preset
      ? `${parameters.length} 项 · 当前单值 · ${presetLabel(preset)}`
      : `${parameters.length} 项 · 等待当前值`,
    parameters,
  };
}
