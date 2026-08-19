import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./runtime-parameter-scheduler.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const runtimeModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  buildRuntimePreviewParameters,
  getAudioValuePreset,
  normalizeAudioMixPickMax,
  normalizeAudioMixPickMin,
  normalizeAudioPeriodRange,
  normalizeVideoPeriodRange,
  samplePeriodMsInRange,
  normalizeAudioVariationPeriod,
  normalizeVideoVariationPeriod,
  normalizeRuntimeVariationPeriod,
  isRuntimeVariationDue,
  AUDIO_PARAM_CROSSFADE_SEC,
  buildAudioFxSignature,
  buildAudioVariantsFromCycle,
  getUnsupportedAudioPresetFields,
  equalMixWeights,
  loadAudioMixSession,
  pickAudioPresetIds,
  sampleAudioCycle,
  saveAudioMixSession,
  AUDIO_MIX_SESSION_STORAGE_KEY,
  sampleSubtleAudioParams,
  sampleSubtleVideoParams,
  sampleSubtleResearchParams,
  sanitizeAudioPresetValues,
} = runtimeModule;

const baseParameters = {
  audio_gain_db: 0,
  audio_low_eq_db: 0,
  audio_mid_eq_db: 0,
  audio_high_eq_db: 0,
  video_brightness_percent: 0,
  video_contrast_percent: 100,
  video_saturation_percent: 100,
  video_hue_rotation_degrees: 0,
  video_blur_radius_px: 0,
  video_pixel_scale_percent: 100,
  video_space_x_offset_px: 0,
  video_space_y_offset_px: 0,
};

test('音视频周期区间归一化并保证 min≤max', () => {
  assert.deepEqual(normalizeAudioPeriodRange(3_000, 6_000), { minMs: 3_000, maxMs: 6_000 });
  assert.deepEqual(normalizeAudioPeriodRange(6_000, 3_000), { minMs: 3_000, maxMs: 6_000 });
  assert.deepEqual(normalizeAudioPeriodRange(500, 90_000), { minMs: 1_000, maxMs: 60_000 });
  assert.deepEqual(normalizeVideoPeriodRange(8_000, 15_000), { minMs: 8_000, maxMs: 15_000 });
  assert.deepEqual(normalizeVideoPeriodRange(undefined, undefined), { minMs: 8_000, maxMs: 15_000 });
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

test('音频快照保持用户配置，视频预览限制在运行时范围内', () => {
  assert.doesNotThrow(() => buildRuntimePreviewParameters(baseParameters, 3));
  const first = buildRuntimePreviewParameters(baseParameters, 3);
  const second = buildRuntimePreviewParameters(baseParameters, 3);
  assert.deepEqual(first, second);
  for (const field of ['audio_gain_db', 'audio_low_eq_db', 'audio_mid_eq_db', 'audio_high_eq_db']) assert.ok(Number.isFinite(first[field]));
  assert.ok(first.audio_gain_db >= -6 && first.audio_gain_db <= 6);
  assert.ok(first.audio_low_eq_db >= -12 && first.audio_low_eq_db <= 12);
  assert.ok(first.audio_mid_eq_db >= -12 && first.audio_mid_eq_db <= 12);
  assert.ok(first.audio_high_eq_db >= -12 && first.audio_high_eq_db <= 12);
  assert.ok(first.video_brightness_percent >= -100 && first.video_brightness_percent <= 100);
  assert.ok(first.video_contrast_percent >= 0 && first.video_contrast_percent <= 200);
  assert.ok(first.video_saturation_percent >= 0 && first.video_saturation_percent <= 200);
  assert.ok(first.video_hue_rotation_degrees >= -180 && first.video_hue_rotation_degrees <= 180);
  assert.ok(first.video_blur_radius_px >= 0 && first.video_blur_radius_px <= 8);
  assert.ok(first.video_pixel_scale_percent >= 95 && first.video_pixel_scale_percent <= 105);
  assert.ok(first.video_space_x_offset_px >= -4 && first.video_space_x_offset_px <= 4);
  assert.ok(first.video_space_y_offset_px >= -4 && first.video_space_y_offset_px <= 4);
});

test('输入超出范围时会被夹紧到运行时预览边界', () => {
  const preview = buildRuntimePreviewParameters(
    {
      audio_gain_db: 999,
      audio_low_eq_db: 999,
      audio_mid_eq_db: -999,
      audio_high_eq_db: 999,
      video_brightness_percent: 999,
      video_contrast_percent: -999,
      video_saturation_percent: -999,
      video_hue_rotation_degrees: 999,
      video_blur_radius_px: -999,
      video_pixel_scale_percent: 999,
      video_space_x_offset_px: -999,
      video_space_y_offset_px: 999,
    },
    0,
  );

  assert.equal(preview.audio_gain_db, 6);
  assert.equal(preview.audio_low_eq_db, 12);
  assert.equal(preview.audio_mid_eq_db, -12);
  assert.equal(preview.audio_high_eq_db, 12);
  assert.equal(preview.video_brightness_percent, 100);
  assert.equal(preview.video_contrast_percent, 0);
  assert.equal(preview.video_saturation_percent, 0);
  assert.equal(preview.video_hue_rotation_degrees, 180);
  assert.equal(preview.video_blur_radius_px, 0);
  assert.equal(preview.video_pixel_scale_percent, 105);
  assert.equal(preview.video_space_x_offset_px, -4);
  assert.equal(preview.video_space_y_offset_px, 4);
});

test('周期相同则预览相同，周期变化只影响视频字段', () => {
  const first = buildRuntimePreviewParameters(baseParameters, 12);
  const same = buildRuntimePreviewParameters(baseParameters, 12);
  const next = buildRuntimePreviewParameters(baseParameters, 13);

  assert.deepEqual(first, same);
  assert.notDeepEqual(first, next);
});

test('音频字段不会被周期调度修改', () => {
  const first = buildRuntimePreviewParameters(baseParameters, 3);
  const next = buildRuntimePreviewParameters(baseParameters, 4);
  const fields = ['audio_gain_db', 'audio_low_eq_db', 'audio_mid_eq_db', 'audio_high_eq_db'];

  for (const field of fields) {
    assert.equal(first[field], next[field], field);
    assert.ok(Number.isFinite(first[field]), field);
    assert.ok(Number.isFinite(next[field]), field);
    assert.equal(first[field], baseParameters[field], field);
    assert.equal(next[field], baseParameters[field], field);
  }
});

test('非有限音频基线会回落为有限快照值', () => {
  const preview = buildRuntimePreviewParameters(
    {
      ...baseParameters,
      audio_gain_db: Number.NaN,
      audio_low_eq_db: Number.POSITIVE_INFINITY,
      audio_mid_eq_db: Number.NEGATIVE_INFINITY,
      audio_high_eq_db: Number.NaN,
    },
    3,
  );

  for (const field of ['audio_gain_db', 'audio_low_eq_db', 'audio_mid_eq_db', 'audio_high_eq_db']) {
    assert.ok(Number.isFinite(preview[field]), field);
  }
});

test('声音值预设有 22 套完整参数，p21/p22 不进入默认随机池', () => {
  assert.equal(AUDIO_VALUE_PRESETS.length, 22);
  assert.equal(DEFAULT_AUDIO_VALUE_PRESET_IDS.length, 20);
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p21'));
  assert.ok(!DEFAULT_AUDIO_VALUE_PRESET_IDS.includes('p22'));
  assert.equal(Object.keys(AUDIO_VALUE_PRESETS[0].values).length, 20);
  const obvious = AUDIO_VALUE_PRESETS.find((preset) => preset.id === 'p21');
  assert.ok(obvious);
  assert.equal(obvious.values.reverb_wet_percent, 20);
  assert.equal(obvious.values.pitch_shift_semitones, 2.0);
  assert.equal(obvious.values.environment_noise_percent, 55);
  assert.equal(obvious.values.vibrato_depth_percent, 3.0);
  const manual = AUDIO_VALUE_PRESETS.find((preset) => preset.id === 'p22');
  assert.ok(manual);
  assert.equal(manual.label, '22. 明显变调与空间（手动）');
  assert.deepEqual(manual.values, {
    pitch_shift_semitones: 2,
    input_gain_db: 0,
    output_gain_db: 0,
    loudness_adjustment_db: 0,
    low_eq_db: -6,
    mid_eq_db: 5,
    high_eq_db: 6,
    filter_q: 2.2,
    phase_perturbation_percent: 8,
    vibrato_frequency_hz: 6.8,
    vibrato_depth_percent: 2.5,
    reverb_wet_percent: 12,
    noise_reduction_percent: 0,
    environment_noise_percent: 0,
    environment_noise_dbfs: -48,
    fade_in_ms: 0,
    fade_out_ms: 0,
    dry_wet_percent: 0,
    ambient_sound_mix_percent: 0,
    spectral_perturbation_percent: 0,
  });
  assert.deepEqual(getUnsupportedAudioPresetFields(manual.values), []);
});

test('预设参数均处于 FFmpeg 映射安全范围，第21个保持原值', () => {
  const safePresets = AUDIO_VALUE_PRESETS.filter((preset) => preset.id !== 'p21');
  const ranges = {
    pitch_shift_semitones: [-2, 2],
    input_gain_db: [-6, 6],
    output_gain_db: [-6, 6],
    loudness_adjustment_db: [-6, 6],
    low_eq_db: [-12, 12],
    mid_eq_db: [-12, 12],
    high_eq_db: [-12, 12],
    filter_q: [0.3, 10],
    phase_perturbation_percent: [-20, 20],
    vibrato_frequency_hz: [3, 8],
    vibrato_depth_percent: [0, 3],
    reverb_wet_percent: [0, 20],
    noise_reduction_percent: [0, 100],
    environment_noise_percent: [0, 100],
    environment_noise_dbfs: [-60, -20],
    fade_in_ms: [0, 10_000],
    fade_out_ms: [0, 10_000],
  };

  for (const preset of safePresets) {
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
  assert.equal(sanitized.dry_wet_percent, 0);
  assert.equal(AUDIO_VALUE_PRESETS.find((preset) => preset.id === 'p21').values.phase_perturbation_percent, 20);
});

test('预设不含未映射的 dry_wet/ambient_sound_mix/spectral 非零值，脏值不被静默清零', () => {
  for (const preset of AUDIO_VALUE_PRESETS) {
    assert.equal(preset.values.dry_wet_percent, 0, preset.id);
    assert.equal(preset.values.ambient_sound_mix_percent, 0, preset.id);
    assert.equal(preset.values.spectral_perturbation_percent, 0, preset.id);
  }
  const dirty = {
    ...AUDIO_VALUE_PRESETS[0].values,
    dry_wet_percent: 1.2,
    ambient_sound_mix_percent: 0.5,
    spectral_perturbation_percent: 0.3,
  };
  assert.deepEqual(getUnsupportedAudioPresetFields(dirty), [
    'dry_wet_percent',
    'ambient_sound_mix_percent',
    'spectral_perturbation_percent',
  ]);
  assert.equal(dirty.dry_wet_percent, 1.2);
});

test('第21项不进入随机抽样和持久化选择集', () => {
  assert.deepEqual(pickAudioPresetIds(['p21'], 1, 1, () => 0), []);
  const store = new Map();
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
  };
  saveAudioMixSession(
    { selectedPresetIds: ['p21', 'p02'], mixEnabled: true, pickMin: 1, pickMax: 2 },
    storage,
  );
  assert.deepEqual(loadAudioMixSession(storage).selectedPresetIds, ['p02']);
});

test('第22项可显式抽样并保存恢复，但不进入默认随机池', () => {
  assert.deepEqual(pickAudioPresetIds(['p22'], 1, 1, () => 0), ['p22']);
  const cycle = sampleAudioCycle(['p22'], { mixEnabled: false, random: () => 0 });
  assert.deepEqual(cycle.presetIds, ['p22']);
  assert.equal(cycle.values.pitch_shift_semitones, 2);
  assert.equal(cycle.values.reverb_wet_percent, 12);

  const store = new Map();
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
  };
  saveAudioMixSession(
    { selectedPresetIds: ['p22'], mixEnabled: false, pickMin: 1, pickMax: 1 },
    storage,
  );
  assert.deepEqual(loadAudioMixSession(storage).selectedPresetIds, ['p22']);
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

test('未勾选预设时回落到平直默认值', () => {
  const sample = sampleSubtleAudioParams([], () => 0);
  assert.equal(sample.pitch_shift_semitones, 0);
  assert.equal(sample.input_gain_db, 0);
  assert.equal(sample.vibrato_depth_percent, 0);
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

test('buildAudioVariantsFromCycle 叠到完整 audio 并保留结构字段', () => {
  const base = {
    natural_voice_mode: 'original',
    random_change_period_ms: 15000,
    playback_speed: 1.1,
    output_bitrate_kbps: 192,
    input_gain_db: 0,
  };
  const cycle = sampleAudioCycle(['p02', 'p05'], {
    mixEnabled: true,
    pickMin: 2,
    pickMax: 2,
    random: () => 0,
  });
  const variants = buildAudioVariantsFromCycle(base, cycle);
  assert.equal(variants.length, 2);
  assert.equal(variants[0].playback_speed, 1.1);
  assert.equal(variants[0].output_bitrate_kbps, 192);
  assert.equal(variants[0].input_gain_db, getAudioValuePreset(cycle.presetIds[0]).values.input_gain_db);
});

test('微扰视频采样写入研究参数范围且可注入随机源', () => {
  let i = 0;
  const sequence = [0, 0.5, 1, 0.25, 0.75, 0.1, 0.9, 0.4, 0.6, 0.2, 0.8, 0.3, 0.7];
  const sample = sampleSubtleVideoParams(() => sequence[i++ % sequence.length]);
  assert.ok(sample.brightness_percent >= -3 && sample.brightness_percent <= 3);
  assert.ok(sample.contrast_percent >= 97 && sample.contrast_percent <= 103);
  assert.ok(sample.saturation_percent >= 97 && sample.saturation_percent <= 103);
  assert.ok(sample.hue_rotation_degrees >= -3 && sample.hue_rotation_degrees <= 3);
  assert.ok(sample.blur_radius_px >= 0 && sample.blur_radius_px <= 0.4);
  assert.ok(sample.pixel_scale_percent >= 99.5 && sample.pixel_scale_percent <= 100.5);
  assert.ok(sample.sharpen_percent >= 0 && sample.sharpen_percent <= 3);
  assert.ok(sample.noise_percent >= 0 && sample.noise_percent <= 0.5);
  // 未映射字段必须保持契约默认，否则 FFmpeg Worker 会拒渲染
  assert.equal(sample.crop_edge_smoothing, 0.5);
  assert.equal(sample.frame_rate_jitter_percent, 0);
  assert.equal(sample.frame_rate_perturbation_frequency_hz, 0.1);
  assert.equal(sample.frame_rate_perturbation_amplitude_fps, 0);
  assert.equal(sample.frame_inner_perturbation_percent, 0);
  assert.equal(sample.frame_inter_perturbation_percent, 0);
  assert.equal(sample.color_space_conversion_strength_percent, 0);
  const again = sampleSubtleVideoParams(() => 0);
  assert.equal(again.brightness_percent, -3);
  assert.equal(again.contrast_percent, 97);
  assert.equal(again.crop_edge_smoothing, 0.5);
});

test('微扰研究采样覆盖 12 频段与挂件/切片字段', () => {
  const sample = sampleSubtleResearchParams(() => 0.5);
  assert.equal(Object.keys(sample.band_weights).length, 12);
  assert.ok(sample.band_weights['65'] >= 0.95 && sample.band_weights['65'] <= 1.05);
  assert.ok(sample.wave_intensity >= 0 && sample.wave_intensity <= 0.15);
  assert.ok(sample.wave_grain_count >= 10 && sample.wave_grain_count <= 40);
  assert.ok(sample.slice_trigger_interval_ms >= sample.slice_length_ms);
  assert.equal(sample.space_dimension, 2);
});
