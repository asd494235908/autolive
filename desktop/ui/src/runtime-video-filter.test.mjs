import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./runtime-video-filter.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const { buildRuntimeVideoStyle } = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`
);

const params = {
  video_brightness_percent: 3,
  video_contrast_percent: 97.5,
  video_saturation_percent: 102.25,
  video_hue_rotation_degrees: -2,
  video_blur_radius_px: 8,
  video_pixel_scale_percent: 101.25,
  video_space_x_offset_px: 1.5,
  video_space_y_offset_px: -2.25,
  video_rotation_degrees: 0.75,
  video_horizontal_flip_enabled: false,
  video_vertical_flip_enabled: true,
};

test('把可忠实直映的实时视频参数组合为单一 filter + transform', () => {
  assert.deepEqual(buildRuntimeVideoStyle(params, true), {
    filter: 'brightness(103%) contrast(97.5%) saturate(102.25%) hue-rotate(-2deg) blur(8px)',
    transform: 'translate(1.5px, -2.25px) rotate(0.75deg) scale(1.0125, -1.0125)',
  });
});

test('视频处理关闭或参数无效时必须原子移除全部样式', () => {
  const neutral = { filter: 'none', transform: 'none' };
  assert.deepEqual(buildRuntimeVideoStyle(params, false), neutral);
  assert.deepEqual(buildRuntimeVideoStyle(null, true), neutral);
  assert.deepEqual(
    buildRuntimeVideoStyle({ ...params, video_contrast_percent: Number.NaN }, true),
    neutral,
  );
});

test('越界输入按正式参数范围收敛', () => {
  assert.deepEqual(
    buildRuntimeVideoStyle({
      ...params,
      video_brightness_percent: 300,
      video_contrast_percent: -1,
      video_saturation_percent: 250,
      video_hue_rotation_degrees: -300,
      video_blur_radius_px: 20,
      video_pixel_scale_percent: 80,
      video_space_x_offset_px: 20,
      video_space_y_offset_px: -20,
      video_rotation_degrees: 300,
    }, true),
    {
      filter: 'brightness(200%) contrast(0%) saturate(200%) hue-rotate(-180deg) blur(8px)',
      transform: 'translate(4px, -4px) rotate(180deg) scale(0.95, -0.95)',
    },
  );
});
