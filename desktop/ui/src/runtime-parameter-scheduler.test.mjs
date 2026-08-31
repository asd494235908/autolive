import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const presetsSource = await readFile(new URL('./audio-value-presets.ts', import.meta.url), 'utf8');
const presetsCompiled = ts.transpileModule(presetsSource, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const presetsModuleUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(presetsCompiled)}`;
const source = await readFile(new URL('./runtime-parameter-scheduler.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText.replaceAll("'./audio-value-presets'", JSON.stringify(presetsModuleUrl));
const runtimeModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  getAudioValuePreset,
  normalizeAudioMixPickMax,
  normalizeAudioMixPickMin,
  normalizeAudioPeriodRange,
  normalizeVideoPeriodRange,
  updatePeriodRangeEndpoint,
  samplePeriodMsInRange,
  normalizeAudioVariationPeriod,
  normalizeVideoVariationPeriod,
  normalizeRuntimeVariationPeriod,
  isRuntimeVariationDue,
  AUDIO_PARAM_CROSSFADE_SEC,
  buildAudioFxSignature,
  buildAudioVariantsFromCycle,
  equalMixWeights,
  loadAudioMixSession,
  loadVideoPeriodRange,
  pickAudioPresetIds,
  sampleAudioCycle,
  saveAudioMixSession,
  AUDIO_MIX_SESSION_STORAGE_KEY,
  sampleSubtleAudioParams,
  sanitizeAudioPresetValues,
  toRuntimePreviewParameters,
} = runtimeModule;

test('运行时消息使用已提交媒体参数而不是旧默认视频值', () => {
  const params = {
    audio: {
      input_gain_db: 1,
      output_gain_db: 2,
      loudness_adjustment_db: 3,
      low_eq_db: 4,
      mid_eq_db: 5,
      high_eq_db: 6,
    },
    video: {
      brightness_percent: 2.5,
      contrast_percent: 104,
      saturation_percent: 96,
      hue_rotation_degrees: -3,
      blur_radius_px: 0.1,
      pixel_scale_percent: 100.2,
      space_x_offset_px: 0.5,
      space_y_offset_px: -0.5,
    },
  };

  const payload = toRuntimePreviewParameters(params);
  assert.equal(payload.video_brightness_percent, 2.5);
  assert.equal(payload.video_contrast_percent, 104);
  assert.equal(payload.video_saturation_percent, 96);
  assert.equal(payload.video_hue_rotation_degrees, -3);
});

test('音视频周期区间归一化并保证 min≤max', () => {
  assert.deepEqual(normalizeAudioPeriodRange(3_000, 6_000), { minMs: 3_000, maxMs: 6_000 });
  assert.deepEqual(normalizeAudioPeriodRange(6_000, 3_000), { minMs: 3_000, maxMs: 6_000 });
  assert.deepEqual(normalizeAudioPeriodRange(500, 90_000), { minMs: 1_000, maxMs: 60_000 });
  assert.deepEqual(normalizeVideoPeriodRange(8_000, 15_000), { minMs: 8_000, maxMs: 15_000 });
  assert.deepEqual(normalizeAudioPeriodRange(undefined, undefined), { minMs: 3_000, maxMs: 5_000 });
  assert.deepEqual(normalizeVideoPeriodRange(undefined, undefined), { minMs: 5_000, maxMs: 8_000 });
});

test('周期端点编辑只更新当前输入且不交换另一端点', () => {
  const range = { minMs: 8_000, maxMs: 15_000 };

  assert.deepEqual(updatePeriodRangeEndpoint(range, 'min', 10_000), { minMs: 10_000, maxMs: 15_000 });
  assert.deepEqual(updatePeriodRangeEndpoint(range, 'max', 12_000), { minMs: 8_000, maxMs: 12_000 });
  assert.deepEqual(updatePeriodRangeEndpoint(range, 'min', 20_000), { minMs: 15_000, maxMs: 15_000 });
  assert.deepEqual(updatePeriodRangeEndpoint(range, 'max', 3_000), { minMs: 8_000, maxMs: 8_000 });
});

test('视频周期缺省使用 5–8 秒且保留已有合法本地值', () => {
  assert.deepEqual(loadVideoPeriodRange({ getItem: () => null }), { minMs: 5_000, maxMs: 8_000 });
  assert.deepEqual(
    loadVideoPeriodRange({ getItem: () => JSON.stringify({ minMs: 1_000, maxMs: 60_000 }) }),
    { minMs: 1_000, maxMs: 60_000 },
  );
  assert.deepEqual(loadVideoPeriodRange({ getItem: () => '12000' }), { minMs: 12_000, maxMs: 12_000 });
});

test('周期区间闭区间随机含端点', () => {
  assert.equal(samplePeriodMsInRange({ minMs: 3_000, maxMs: 3_000 }), 3_000);
  assert.equal(samplePeriodMsInRange({ minMs: 3_000, maxMs: 6_000 }, () => 0), 3_000);
  assert.equal(samplePeriodMsInRange({ minMs: 3_000, maxMs: 6_000 }, () => 0.999999), 6_000);
  for (let i = 0; i < 50; i += 1) {
    const value = samplePeriodMsInRange({ minMs: 3_000, maxMs: 6_000 });
    assert.ok(value >= 3_000 && value <= 6_000);
  }
});

test('周期未到时不触发，达到周期时触发', () => {
  assert.equal(isRuntimeVariationDue(2_999, 0, 3_000), false);
  assert.equal(isRuntimeVariationDue(3_000, 0, 3_000), true);
  assert.equal(isRuntimeVariationDue(5_999, 0, 6_000), false);
  assert.equal(isRuntimeVariationDue(6_000, 0, 6_000), true);
});

test('声音值预设有 22 套完整参数，p21/p22 不进入默认随机池', () => {
  assert.equal(AUDIO_VALUE_PRESETS.length, 22);
  assert.equal(DEFAULT_AUDIO_VALUE_PRESET_IDS.length, 20);
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p21'));
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p22'));
  assert.equal(Object.keys(AUDIO_VALUE_PRESETS[0].values).length, 35);
  const obvious = AUDIO_VALUE_PRESETS.find((preset) => preset.id === 'p21');
  assert.ok(obvious);
  assert.equal(obvious.values.reverb_wet_percent, 20);
  assert.equal(obvious.values.environment_noise_percent, 30);
  const manual = AUDIO_VALUE_PRESETS.find((preset) => preset.id === 'p22');
  assert.ok(manual);
  assert.equal(manual.label, '22. 明显变调与音色（手动）');
  assert.equal(manual.values.pitch_shift_semitones, 2);
  assert.equal(manual.values.playback_speed, 1.18);
  assert.equal(manual.values.formant_shift_percent, 5);
});

test('预设参数均处于 Rust 参数安全范围', () => {
  const ranges = {
    pitch_shift_semitones: [-2, 2],
    spectral_perturbation_percent: [0, 10],
    environment_noise_percent: [0, 100],
    environment_noise_dbfs: [-60, -20],
    mfcc_shift_percent: [-20, 20],
    input_gain_db: [-6, 6],
    output_gain_db: [-6, 6],
    loudness_adjustment_db: [-6, 6],
    playback_speed: [0.5, 2],
    low_eq_db: [-12, 12],
    mid_eq_db: [-12, 12],
    high_eq_db: [-12, 12],
    filter_q: [0.3, 10],
    phase_perturbation_percent: [-20, 20],
    vibrato_frequency_hz: [3, 8],
    vibrato_depth_percent: [0, 3],
    reverb_wet_percent: [0, 20],
    noise_reduction_percent: [0, 100],
    ambient_sound_mix_percent: [0, 100],
    dry_wet_percent: [0, 100],
    mfcc_dimensions: [1, 40],
    snr_variation_db: [-6, 6],
    formant_shift_percent: [-5, 5],
    spectrum_blind_spot_percent: [0, 5],
    fade_in_ms: [0, 10_000],
    fade_out_ms: [0, 10_000],
    high_frequency_perturbation_interval_ms: [500, 60_000],
    high_frequency_perturbation_strength_percent: [0, 20],
    high_frequency_perturbation_level_db: [-60, 0],
  };

  for (const preset of AUDIO_VALUE_PRESETS) {
    for (const [field, [minimum, maximum]] of Object.entries(ranges)) {
      const value = preset.values[field];
      assert.ok(Number.isFinite(value), `${preset.id}.${field}`);
      assert.ok(value >= minimum && value <= maximum, `${preset.id}.${field}=${value}`);
    }
  }

  const sanitized = sanitizeAudioPresetValues({
    ...AUDIO_VALUE_PRESETS[0].values,
    pitch_shift_semitones: Number.NaN,
    filter_q: 999,
    environment_noise_dbfs: -999,
    dry_wet_percent: 12,
  });
  assert.equal(sanitized.pitch_shift_semitones, 0);
  assert.equal(sanitized.filter_q, 10);
  assert.equal(sanitized.environment_noise_dbfs, -60);
  assert.equal(sanitized.dry_wet_percent, 12);
});

test('第 1–20 套的 MFCC、SNR、频谱盲区、干湿与环境声均真实参加', () => {
  for (const preset of AUDIO_VALUE_PRESETS.slice(0, 20)) {
    assert.notEqual(preset.values.mfcc_shift_percent, 0, preset.id);
    assert.notEqual(preset.values.snr_variation_db, 0, preset.id);
    assert.ok(preset.values.spectrum_blind_spot_percent > 0, preset.id);
    assert.ok(preset.values.dry_wet_percent > 0, preset.id);
    assert.ok(preset.values.ambient_sound_mix_percent > 0, preset.id);
    assert.equal(preset.values.snr_target_db, null, preset.id);
  }
});

test('第21、22项都可显式抽样和持久化，但不进入默认池', () => {
  assert.deepEqual(pickAudioPresetIds(['p21'], 1, 1, () => 0), ['p21']);
  assert.deepEqual(pickAudioPresetIds(['p22'], 1, 1, () => 0), ['p22']);
  const store = new Map();
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
  };
  saveAudioMixSession(
    { selectedPresetIds: ['p21', 'p22'], mixEnabled: true, pickMin: 1, pickMax: 2 },
    storage,
  );
  assert.deepEqual(loadAudioMixSession(storage).selectedPresetIds, ['p21', 'p22']);
});

test('普通声音与插话共用选择规则，并继续限制四轨、不放回和相邻周期避重', () => {
  const allPresetIds = AUDIO_VALUE_PRESETS.map((preset) => preset.id);
  const mixed = sampleAudioCycle(allPresetIds, {
    mixEnabled: true,
    pickMin: 4,
    pickMax: 99,
    random: () => 0,
  });
  assert.equal(mixed.presetIds.length, 4);
  assert.equal(new Set(mixed.presetIds).size, 4);

  const first = sampleAudioCycle(['p21', 'p22'], {
    random: () => 0,
  });
  const second = sampleAudioCycle(['p21', 'p22'], {
    random: () => 0,
    previousPresetIds: first.presetIds,
  });
  assert.notDeepEqual(second.presetIds, first.presetIds);
});

test('第22项可显式产生完整夸张快照', () => {
  assert.deepEqual(pickAudioPresetIds(['p22'], 1, 1, () => 0), ['p22']);
  const cycle = sampleAudioCycle(['p22'], { mixEnabled: false, random: () => 0 });
  assert.deepEqual(cycle.presetIds, ['p22']);
  assert.equal(cycle.values.pitch_shift_semitones, 2);
  assert.equal(cycle.values.reverb_wet_percent, 12);
});

test('周期随机从勾选预设中抽一整套参数值', () => {
  const first = sampleAudioCycle(['p02'], { mixEnabled: false, random: () => 0 });
  assert.deepEqual(first.presetIds, ['p02']);
  assert.equal(first.values.input_gain_db, getAudioValuePreset('p02').values.input_gain_db);
  const second = sampleAudioCycle(['p07', 'p11'], { mixEnabled: false, random: () => 0 });
  assert.equal(second.presetIds.length, 1);
  assert.ok(['p07', 'p11'].includes(second.presetIds[0]));
  assert.equal(second.values.pitch_shift_semitones, getAudioValuePreset(second.presetIds[0]).values.pitch_shift_semitones);
});

test('相邻周期不重复预设，隔周期可以再次出现，单候选允许重复', () => {
  const first = sampleAudioCycle(['p01', 'p02'], { mixEnabled: false, random: () => 0 });
  const second = sampleAudioCycle(['p01', 'p02'], {
    mixEnabled: false,
    random: () => 0,
    previousPresetIds: first.presetIds,
  });
  const third = sampleAudioCycle(['p01', 'p02'], {
    mixEnabled: false,
    random: () => 0,
    previousPresetIds: second.presetIds,
  });
  assert.notDeepEqual(second.presetIds, first.presetIds);
  assert.deepEqual(third.presetIds, first.presetIds);

  const onlyPreset = sampleAudioCycle(['p01'], {
    mixEnabled: false,
    previousPresetIds: ['p01'],
    random: () => 0,
  });
  assert.deepEqual(onlyPreset.presetIds, ['p01']);
});

test('未勾选预设时回落到第 1 套完整低感知值', () => {
  const sample = sampleSubtleAudioParams([], () => 0);
  assert.deepEqual(sample, getAudioValuePreset('p01').values);
});

test('mix_pick 范围归一化到 1-4', () => {
  assert.equal(normalizeAudioMixPickMax(0), 1);
  assert.equal(normalizeAudioMixPickMax(9), 4);
  assert.equal(normalizeAudioMixPickMin(3, 2), 2);
  assert.equal(normalizeAudioMixPickMin(1, 4), 1);
});

test('开启多轨合并时可抽多套预设 ID', () => {
  const ids = pickAudioPresetIds(['p01', 'p02', 'p03', 'p04'], 2, 3, () => 0);
  assert.equal(ids.length, 2);
  const cycle = sampleAudioCycle(['p01', 'p02', 'p03'], { mixEnabled: true, pickMin: 2, pickMax: 2, random: () => 0 });
  assert.equal(cycle.presetIds.length, 2);
  assert.equal(cycle.values.input_gain_db, getAudioValuePreset(cycle.presetIds[0]).values.input_gain_db);
  assert.equal(cycle.variants.length, 2);
  assert.deepEqual(cycle.weights, equalMixWeights(2));
  assert.equal(typeof cycle.seed, 'number');
});

test('同 seed 可复现抽样', () => {
  const a = sampleAudioCycle(['p01', 'p02', 'p03', 'p04'], {
    mixEnabled: true,
    pickMin: 2,
    pickMax: 2,
    seed: 42,
  });
  const b = sampleAudioCycle(['p01', 'p02', 'p03', 'p04'], {
    mixEnabled: true,
    pickMin: 2,
    pickMax: 2,
    seed: 42,
  });
  assert.deepEqual(a.presetIds, b.presetIds);
  assert.equal(a.seed, 42);
});

test('音频混合会话可读写 localStorage', () => {
  const store = new Map();
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
  };
  saveAudioMixSession(
    { selectedPresetIds: ['p02', 'p02', 'missing'], mixEnabled: true, pickMin: 1, pickMax: 3 },
    storage,
  );
  assert.ok(store.has(AUDIO_MIX_SESSION_STORAGE_KEY));
  const loaded = loadAudioMixSession(storage);
  assert.deepEqual(loaded, {
    selectedPresetIds: ['p02'],
    mixEnabled: true,
    pickMin: 1,
    pickMax: 3,
  });
});

test('参数交叉淡化签名在关闭时为 off，开启时随增益变化', () => {
  assert.equal(AUDIO_PARAM_CROSSFADE_SEC, 0.03);
  assert.equal(buildAudioFxSignature(null, true), 'off');
  assert.equal(buildAudioFxSignature({ audio_input_gain_db: 0.1 }, false), 'off');
  const a = buildAudioFxSignature({
    audio_input_gain_db: 0.1,
    audio_low_eq_db: 0.2,
    audio_reverb_wet_percent: 18,
    audio_environment_noise_percent: 22,
  }, true);
  const b = buildAudioFxSignature({
    audio_input_gain_db: 0.2,
    audio_low_eq_db: 0.2,
    audio_reverb_wet_percent: 18,
    audio_environment_noise_percent: 22,
  }, true);
  const c = buildAudioFxSignature({
    audio_input_gain_db: 0.1,
    audio_low_eq_db: 0.2,
    audio_reverb_wet_percent: 1,
    audio_environment_noise_percent: 22,
  }, true);
  assert.notEqual(a, c);
  assert.notEqual(a, b);
});

test('buildAudioVariantsFromCycle 只共享共同时间轴与混音后总线字段', () => {
  const base = {
    natural_voice_mode: 'original',
    random_change_period_ms: 15000,
    pitch_shift_semitones: 0.75,
    formant_shift_percent: 3,
    playback_speed: 1.1,
    sample_rate_hz: 48_000,
    output_bitrate_kbps: 192,
    input_gain_db: 0,
    mfcc_shift_percent: 4,
    mfcc_dimensions: 20,
    snr_target_db: 30,
    snr_variation_db: 1,
    dry_wet_percent: 12,
    ambient_sound_mix_percent: 8,
    spectral_perturbation_percent: 2,
    voice_library_id: 'local-a',
  };
  const cycle = sampleAudioCycle(['p02', 'p05'], {
    mixEnabled: true,
    pickMin: 2,
    pickMax: 2,
    random: () => 0,
  });
  const variants = buildAudioVariantsFromCycle(base, cycle);
  assert.equal(variants.length, 2);
  assert.ok(variants.every((variant) => variant.playback_speed === 1.1));
  assert.ok(variants.every((variant) => variant.sample_rate_hz === 48_000));
  assert.ok(variants.every((variant) => variant.output_bitrate_kbps === 192));
  assert.ok(variants.every((variant) => variant.random_change_period_ms === 15000));
  assert.ok(variants.every((variant) => variant.pitch_shift_semitones === 0.75));
  assert.ok(variants.every((variant) => variant.formant_shift_percent === 3));
  assert.ok(variants.every((variant) => variant.mfcc_shift_percent === 4));
  assert.ok(variants.every((variant) => variant.mfcc_dimensions === 20));
  assert.ok(variants.every((variant) => variant.snr_target_db === 30));
  assert.ok(variants.every((variant) => variant.snr_variation_db === 1));
  assert.ok(variants.every((variant) => variant.ambient_sound_mix_percent === 8));
  assert.ok(variants.every((variant) => variant.current_formant_hz === null));
  assert.equal(variants[0].input_gain_db, getAudioValuePreset(cycle.presetIds[0]).values.input_gain_db);
  assert.equal(variants[0].dry_wet_percent, getAudioValuePreset(cycle.presetIds[0]).values.dry_wet_percent);
  assert.equal(variants[0].spectral_perturbation_percent, getAudioValuePreset(cycle.presetIds[0]).values.spectral_perturbation_percent);
  assert.equal(variants[0].voice_library_id, getAudioValuePreset(cycle.presetIds[0]).values.voice_library_id);
});
