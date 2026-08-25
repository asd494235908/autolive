import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./audio-value-presets.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const presetsModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUDIO_PRESET_FIELDS,
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  getAudioValuePreset,
  sanitizeAudioPresetValues,
} = presetsModule;

const READ_ONLY_RANDOMIZED_AUDIO_FIELDS = [
  'pitch_shift_semitones',
  'playback_speed',
  'formant_shift_percent',
  'mfcc_shift_percent',
  'mfcc_dimensions',
  'snr_variation_db',
  'spectrum_blind_spot_percent',
  'dry_wet_percent',
  'ambient_sound_mix_percent',
];

test('22 套声音预设逐套精确包含 35 个效果字段', () => {
  assert.equal(AUDIO_PRESET_FIELDS.length, 35);
  assert.equal(new Set(AUDIO_PRESET_FIELDS).size, 35);
  assert.equal(AUDIO_VALUE_PRESETS.length, 22);
  for (const preset of AUDIO_VALUE_PRESETS) {
    assert.deepEqual(Object.keys(preset.values).sort(), [...AUDIO_PRESET_FIELDS].sort(), preset.id);
  }
});

test('第 1–20 套完整低感知预设进入默认池且目标 SNR 为自动', () => {
  assert.deepEqual(DEFAULT_AUDIO_VALUE_PRESET_IDS, AUDIO_VALUE_PRESETS.slice(0, 20).map(({ id }) => id));
  for (const preset of AUDIO_VALUE_PRESETS.slice(0, 20)) {
    const values = preset.values;
    assert.equal(values.natural_voice_mode, 'natural_dynamic', preset.id);
    assert.equal(values.snr_target_db, null, preset.id);
    assert.equal(values.high_frequency_perturbation_enabled, true, preset.id);
    assert.ok(values.voice_library_id, preset.id);
    assert.ok(Math.abs(values.pitch_shift_semitones) <= 0.05, preset.id);
    assert.ok(Math.abs(values.playback_speed - 1) <= 0.003, preset.id);
    assert.ok(Math.abs(values.formant_shift_percent) <= 0.4, preset.id);
    const combinedGainDb = values.input_gain_db
      + values.output_gain_db
      + values.loudness_adjustment_db;
    assert.ok(Math.abs(combinedGainDb) <= 0.2, `${preset.id} combined gain ${combinedGainDb}dB`);
    assert.ok(values.mfcc_shift_percent !== 0, preset.id);
    assert.ok(values.snr_variation_db !== 0, preset.id);
    assert.ok(values.spectrum_blind_spot_percent > 0, preset.id);
    assert.ok(values.dry_wet_percent > 0, preset.id);
    assert.ok(values.ambient_sound_mix_percent > 0, preset.id);
  }
});

test('九项只读声音参数仍由默认随机预设产生不同值', () => {
  const defaultPresets = AUDIO_VALUE_PRESETS.filter(({ id }) => (
    DEFAULT_AUDIO_VALUE_PRESET_IDS.includes(id)
  ));

  for (const field of READ_ONLY_RANDOMIZED_AUDIO_FIELDS) {
    assert.ok(AUDIO_PRESET_FIELDS.includes(field), `${field} 必须保留在完整预设快照中`);
    assert.ok(
      new Set(defaultPresets.map(({ values }) => values[field])).size > 1,
      `${field} 必须随默认声音预设随机化`,
    );
  }
});

test('第 1–20 套除自动目标 SNR 外，每个效果字段都脱离中性值', () => {
  const neutral = {
    natural_voice_mode: 'original',
    pitch_shift_semitones: 0,
    spectral_perturbation_percent: 0,
    environment_noise_percent: 0,
    environment_noise_dbfs: -40,
    mfcc_shift_percent: 0,
    phase_perturbation_percent: 0,
    loudness_adjustment_db: 0,
    input_gain_db: 0,
    output_gain_db: 0,
    playback_speed: 1,
    low_eq_db: 0,
    mid_eq_db: 0,
    high_eq_db: 0,
    noise_reduction_percent: 0,
    ambient_sound_mix_percent: 0,
    fade_in_ms: 0,
    fade_out_ms: 0,
    dry_wet_percent: 0,
    reverb_wet_percent: 0,
    mfcc_dimensions: 13,
    snr_variation_db: 0,
    formant_shift_percent: 0,
    vibrato_frequency_hz: 5,
    vibrato_depth_percent: 0,
    spectrum_blind_spot_percent: 0,
    snr_target_db: null,
    filter_q: 1,
    sample_rate_hz: null,
    output_bitrate_kbps: 192,
    voice_library_id: null,
    high_frequency_perturbation_enabled: false,
    high_frequency_perturbation_interval_ms: 12_000,
    high_frequency_perturbation_strength_percent: 0,
    high_frequency_perturbation_level_db: -32,
  };
  for (const preset of AUDIO_VALUE_PRESETS.slice(0, 20)) {
    for (const field of AUDIO_PRESET_FIELDS) {
      if (field === 'snr_target_db') continue;
      assert.notEqual(preset.values[field], neutral[field], `${preset.id}.${field}`);
    }
  }
});

test('第 21、22 套明显可听但不进入默认池', () => {
  const p21 = getAudioValuePreset('p21');
  const p22 = getAudioValuePreset('p22');
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p21'));
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p22'));
  assert.ok(p21.values.reverb_wet_percent >= 15);
  assert.ok(p21.values.environment_noise_percent >= 20);
  assert.ok(p22.values.pitch_shift_semitones >= 1.5);
  assert.ok(p22.values.playback_speed >= 1.1);
  assert.ok(Math.abs(p22.values.formant_shift_percent) >= 3);
});

test('声音预设信任边界覆盖新增字段并丢弃额外字段', () => {
  const sanitized = sanitizeAudioPresetValues({
    ...AUDIO_VALUE_PRESETS[0].values,
    playback_speed: 99,
    mfcc_dimensions: 99,
    snr_target_db: Number.NaN,
    sample_rate_hz: 96_000,
    output_bitrate_kbps: 999,
    high_frequency_perturbation_interval_ms: 1,
    high_frequency_perturbation_strength_percent: 99,
    extra: 'drop-me',
  });
  assert.equal(sanitized.playback_speed, 2);
  assert.equal(sanitized.mfcc_dimensions, 40);
  assert.equal(sanitized.snr_target_db, null);
  assert.equal(sanitized.sample_rate_hz, null);
  assert.equal(sanitized.output_bitrate_kbps, 320);
  assert.equal(sanitized.high_frequency_perturbation_interval_ms, 500);
  assert.equal(sanitized.high_frequency_perturbation_strength_percent, 20);
  assert.equal('extra' in sanitized, false);
});
