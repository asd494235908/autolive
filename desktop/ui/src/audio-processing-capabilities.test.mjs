import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = (await readFile(new URL('./audio-processing-capabilities.ts', import.meta.url), 'utf8')).replace(
  "import { AUDIO_MIX_PICK_HARD_MAX } from './runtime-parameter-scheduler';",
  'const AUDIO_MIX_PICK_HARD_MAX = 4;',
);
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const capabilityModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

const audio = {
  natural_voice_mode: 'original',
  random_change_period_ms: 4_000,
  input_gain_db: 1,
  output_gain_db: 2,
  loudness_adjustment_db: -0.5,
  low_eq_db: 0.1,
  mid_eq_db: -0.2,
  high_eq_db: 0.3,
  pitch_shift_semitones: 0.05,
  playback_speed: 1,
  fade_in_ms: 50,
  fade_out_ms: 100,
  reverb_wet_percent: 10,
  noise_reduction_percent: 0,
  phase_perturbation_percent: 0,
  vibrato_frequency_hz: 5,
  vibrato_depth_percent: 0,
  environment_noise_percent: 0,
  environment_noise_dbfs: -48,
  filter_q: 1,
  dry_wet_percent: 0,
  ambient_sound_mix_percent: 0,
  spectral_perturbation_percent: 0,
  mfcc_shift_percent: 0,
  mfcc_dimensions: 13,
  snr_variation_db: 0,
  formant_shift_percent: 0,
  spectrum_blind_spot_percent: 0,
  snr_target_db: null,
  current_formant_hz: null,
  sample_rate_hz: 48_000,
  output_bitrate_kbps: 192,
  voice_library_id: null,
  high_frequency_perturbation_enabled: false,
  high_frequency_perturbation_interval_ms: 12_000,
  high_frequency_perturbation_strength_percent: 0,
  high_frequency_perturbation_level_db: -32,
};

const requiredAudioMetricFields = [
  'input_gain_db',
  'output_gain_db',
  'loudness_adjustment_db',
  'low_eq_db',
  'mid_eq_db',
  'high_eq_db',
  'pitch_shift_semitones',
  'playback_speed',
  'formant_shift_percent',
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
];

test('基础参数全部展示，并区分本地 DSP 已生效和待应用参数', () => {
  const rows = capabilityModule.buildAudioCapabilityRows(audio, true, true);
  assert.equal(rows.length, 28);
  assert.equal(rows.find((row) => row.key === 'dry_wet_percent').status, 'ready');
  assert.equal(rows.find((row) => row.key === 'ambient_sound_mix_percent').status, 'ready');
  assert.equal(rows.find((row) => row.key === 'spectral_perturbation_percent').status, 'ready');
  assert.equal(rows.find((row) => row.key === 'high_frequency_perturbation_strength_percent').status, 'ready');
  assert.equal(rows.find((row) => row.key === 'total_gain_db').value, 2.5);
  for (const key of ['total_gain_db', 'low_eq_db', 'mid_eq_db', 'high_eq_db']) {
    assert.equal(rows.find((row) => row.key === key).status, 'ready', key);
  }
  for (const key of ['pitch_shift_semitones', 'playback_speed', 'formant_shift_percent']) {
    assert.equal(rows.find((row) => row.key === key).status, 'ready', key);
  }
  for (const key of ['sample_rate_hz', 'output_bitrate_kbps']) {
    assert.equal(rows.find((row) => row.key === key).status, 'configured', key);
  }
  for (const key of ['input_gain_db', 'output_gain_db', 'loudness_adjustment_db', 'fade_in_ms', 'fade_out_ms', 'reverb_wet_percent']) {
    assert.equal(rows.find((row) => row.key === key).status, 'ready', key);
  }
});

test('未启用声音处理时基础参数显示为未启用', () => {
  const rows = capabilityModule.buildAudioCapabilityRows(audio, false, false);
  assert.ok(rows.every((row) => row.status === 'disabled'));
});

test('无效或自动采样率参数不会产生 NaN', () => {
  const rows = capabilityModule.buildAudioCapabilityRows({ sample_rate_hz: null, input_gain_db: Number.NaN }, true, false);
  assert.equal(rows.find((row) => row.key === 'sample_rate_hz').value, null);
  assert.equal(rows.find((row) => row.key === 'input_gain_db').value, null);
  assert.equal(rows.find((row) => row.key === 'total_gain_db').value, null);
});

test('真实 Rust 音频 DTO 的枚举和资源 ID 不阻断运行支路', () => {
  const withVoiceId = {
    ...audio,
    voice_library_id: 'local-voice-1',
  };
  const withAutomaticVoice = {
    ...withVoiceId,
    natural_voice_mode: 'natural_dynamic',
    sample_rate_hz: null,
    voice_library_id: null,
  };

  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches(withVoiceId, [], true),
    [withVoiceId],
  );
  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches(withAutomaticVoice, [], true),
    [withAutomaticVoice],
  );
  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches(withVoiceId, [withVoiceId, withAutomaticVoice], true),
    [withVoiceId, withAutomaticVoice],
  );
  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches({ natural_voice_mode: 'original' }, [], true),
    [],
  );
  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches({ ...withVoiceId, input_gain_db: Number.NaN }, [], true),
    [],
  );
  for (const field of requiredAudioMetricFields) {
    const missing = { ...withVoiceId };
    delete missing[field];
    assert.deepEqual(
      capabilityModule.resolveAudioStreamBranches(missing, [], true),
      [],
      `缺少 ${field} 时必须拒绝真实输出快照`,
    );
  }
  const missingSampleRate = { ...withVoiceId };
  delete missingSampleRate.sample_rate_hz;
  assert.deepEqual(
    capabilityModule.resolveAudioStreamBranches(missingSampleRate, [], true),
    [],
    '缺少 sample_rate_hz 时必须拒绝真实输出快照',
  );
});
