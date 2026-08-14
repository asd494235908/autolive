import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import * as ts from 'typescript';

const source = await readFile(new URL('./运行时参数自动调度.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const runtimeModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const { buildRuntimePreviewParameters, normalizeRuntimeVariationPeriod, isRuntimeVariationDue } = runtimeModule;

const baseParameters = {
  audio_gain_db: 0,
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
  assert.equal(normalizeRuntimeVariationPeriod(undefined), 5_000);
  assert.equal(normalizeRuntimeVariationPeriod(0), 5_000);
  assert.equal(normalizeRuntimeVariationPeriod(499), 500);
  assert.equal(normalizeRuntimeVariationPeriod(500), 500);
  assert.equal(normalizeRuntimeVariationPeriod(5_000), 5_000);
  assert.equal(normalizeRuntimeVariationPeriod(60_000), 60_000);
  assert.equal(normalizeRuntimeVariationPeriod(60_001), 60_000);
});

test('周期未到时不触发，达到周期时触发', () => {
  assert.equal(isRuntimeVariationDue(4_999, 0, 5_000), false);
  assert.equal(isRuntimeVariationDue(5_000, 0, 5_000), true);
  assert.equal(isRuntimeVariationDue(4_999, 0, 0), false);
  assert.equal(isRuntimeVariationDue(5_000, 0, 0), true);
  assert.equal(isRuntimeVariationDue(100, 100, 5_000), false);
  assert.equal(isRuntimeVariationDue(5_100, 100, 5_000), true);
});

test('相同周期输入生成稳定参数，并限制在预览范围内', () => {
  assert.doesNotThrow(() => buildRuntimePreviewParameters(baseParameters, 3));
  const first = buildRuntimePreviewParameters(baseParameters, 3);
  const second = buildRuntimePreviewParameters(baseParameters, 3);
  assert.deepEqual(first, second);
  assert.ok(first.audio_gain_db >= -6 && first.audio_gain_db <= 6);
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
  assert.equal(preview.video_brightness_percent, 100);
  assert.equal(preview.video_contrast_percent, 0);
  assert.equal(preview.video_saturation_percent, 0);
  assert.equal(preview.video_hue_rotation_degrees, 180);
  assert.equal(preview.video_blur_radius_px, 0);
  assert.equal(preview.video_pixel_scale_percent, 105);
  assert.equal(preview.video_space_x_offset_px, -4);
  assert.equal(preview.video_space_y_offset_px, 4);
});

test('周期相同则预览相同，周期变化则预览变化', () => {
  const first = buildRuntimePreviewParameters(baseParameters, 12);
  const same = buildRuntimePreviewParameters(baseParameters, 12);
  const next = buildRuntimePreviewParameters(baseParameters, 13);

  assert.deepEqual(first, same);
  assert.notDeepEqual(first, next);
});
