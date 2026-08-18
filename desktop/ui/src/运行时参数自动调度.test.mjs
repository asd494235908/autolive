import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./运行时参数自动调度.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const runtimeModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUDIO_VALUE_PRESETS,
  DEFAULT_AUDIO_VALUE_PRESET_IDS,
  buildRuntimePreviewParameters,
  getAudioValuePreset,
  normalizeRuntimeVariationPeriod,
  isRuntimeVariationDue,
  sampleSubtleAudioParams,
  sampleSubtleVideoParams,
  sampleSubtleResearchParams,
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

test('默认周期、下限和上限会被稳定归一化', () => {
  assert.equal(typeof normalizeRuntimeVariationPeriod, 'function');
  assert.equal(normalizeRuntimeVariationPeriod(undefined), 15_000);
  assert.equal(normalizeRuntimeVariationPeriod(0), 15_000);
  assert.equal(normalizeRuntimeVariationPeriod(499), 500);
  assert.equal(normalizeRuntimeVariationPeriod(500), 500);
  assert.equal(normalizeRuntimeVariationPeriod(5_000), 5_000);
  assert.equal(normalizeRuntimeVariationPeriod(15_000), 15_000);
  assert.equal(normalizeRuntimeVariationPeriod(60_000), 60_000);
  assert.equal(normalizeRuntimeVariationPeriod(60_001), 60_000);
});

test('周期未到时不触发，达到周期时触发', () => {
  assert.equal(isRuntimeVariationDue(4_999, 0, 5_000), false);
  assert.equal(isRuntimeVariationDue(5_000, 0, 5_000), true);
  assert.equal(isRuntimeVariationDue(14_999, 0, 0), false);
  assert.equal(isRuntimeVariationDue(15_000, 0, 0), true);
  assert.equal(isRuntimeVariationDue(100, 100, 5_000), false);
  assert.equal(isRuntimeVariationDue(5_100, 100, 5_000), true);
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

test('声音值预设有 20 套完整参数', () => {
  assert.equal(AUDIO_VALUE_PRESETS.length, 20);
  assert.equal(DEFAULT_AUDIO_VALUE_PRESET_IDS.length, 20);
  assert.equal(Object.keys(AUDIO_VALUE_PRESETS[0].values).length, 20);
  assert.equal(getAudioValuePreset('missing').id, 'p01');
});

test('周期随机从勾选预设中抽一整套参数值', () => {
  const first = sampleSubtleAudioParams(['p02'], () => 0);
  assert.equal(first.input_gain_db, getAudioValuePreset('p02').values.input_gain_db);
  assert.equal(first.output_gain_db, getAudioValuePreset('p02').values.output_gain_db);
  const second = sampleSubtleAudioParams(['p07', 'p11'], () => 0.9);
  assert.equal(second.pitch_shift_semitones, getAudioValuePreset('p11').values.pitch_shift_semitones);
  assert.equal(second.reverb_wet_percent, getAudioValuePreset('p11').values.reverb_wet_percent);
});

test('未勾选预设时回落到平直默认值', () => {
  const sample = sampleSubtleAudioParams([], () => 0);
  assert.equal(sample.pitch_shift_semitones, 0);
  assert.equal(sample.input_gain_db, 0);
  assert.equal(sample.vibrato_depth_percent, 0);
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
  assert.ok(sample.crop_edge_smoothing >= 0.45 && sample.crop_edge_smoothing <= 0.55);
  assert.ok(sample.frame_rate_jitter_percent >= 0 && sample.frame_rate_jitter_percent <= 0.4);
  assert.ok(sample.frame_rate_perturbation_frequency_hz >= 0.05 && sample.frame_rate_perturbation_frequency_hz <= 0.2);
  assert.ok(sample.frame_inter_perturbation_percent >= 0 && sample.frame_inter_perturbation_percent <= 1);
  assert.ok(sample.color_space_conversion_strength_percent >= 0 && sample.color_space_conversion_strength_percent <= 2);
  const again = sampleSubtleVideoParams(() => 0);
  assert.equal(again.brightness_percent, -3);
  assert.equal(again.contrast_percent, 97);
  assert.equal(again.crop_edge_smoothing, 0.45);
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
