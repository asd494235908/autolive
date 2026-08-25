import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./video-parameter-randomizer.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const { AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION, sampleAutomaticVideoParameters } = module;

const expectedVideoFields = [
  'blur_radius_px',
  'brightness_percent',
  'color_space_conversion_enabled',
  'color_space_conversion_strength_percent',
  'contrast_percent',
  'crop_edge_smoothing',
  'detail_enhancement_percent',
  'dynamic_crop_percent',
  'edge_softness_percent',
  'frame_inner_perturbation_percent',
  'frame_inter_perturbation_percent',
  'frame_rate_jitter_percent',
  'frame_rate_lock_enabled',
  'frame_rate_perturbation_amplitude_fps',
  'frame_rate_perturbation_frequency_hz',
  'highlights_percent',
  'horizontal_flip_enabled',
  'hue_rotation_degrees',
  'image_repair_enabled',
  'image_repair_strength_percent',
  'noise_percent',
  'pixel_jitter_px',
  'pixel_scale_percent',
  'red_channel_lock_enabled',
  'rotation_degrees',
  'saturation_percent',
  'shadows_percent',
  'sharpen_percent',
  'space_x_offset_px',
  'space_y_offset_px',
  'vertical_flip_enabled',
  'vignette_percent',
];

const expectedAdvancedFields = [
  'abstract_face_count',
  'abstract_face_opacity_percent',
  'abstract_face_size_percent',
  'asynchronous_rotation_enabled',
  'asynchronous_rotation_max_degrees',
  'asynchronous_rotation_min_degrees',
  'band_weights',
  'channel_offset_percent',
  'core_frequency_hz',
  'dynamic_eq_threshold',
  'edge_feather_percent',
  'edge_fill_enabled',
  'frame_perturbation_probability_percent',
  'frequency_space_x_offset_px',
  'frequency_space_y_offset_px',
  'highlight_perturbation_enabled',
  'highlight_perturbation_interval_ms',
  'local_blur_enabled',
  'local_blur_interval_ms',
  'local_blur_radius_px',
  'local_blur_region_percent',
  'overlay_offset_px',
  'picture_in_picture_enabled',
  'picture_in_picture_opacity_percent',
  'picture_in_picture_pixel_jitter_px',
  'picture_in_picture_rotation_degrees',
  'picture_in_picture_scale_percent',
  'picture_in_picture_timeline_locked',
  'random_graphic_count',
  'random_graphic_enabled',
  'random_graphic_opacity_percent',
  'random_graphic_size_px',
  'slice_length_ms',
  'slice_min_length_ms',
  'slice_trigger_interval_ms',
  'space_dimension',
  'target_frequency_hz',
  'transform_smoothing_duration_ms',
  'transform_smoothing_enabled',
  'wave_grain_count',
  'wave_intensity',
  'wave_level',
];

function sortedKeys(value) {
  return Object.keys(value).sort();
}

test('同一 seed 生成可复现的完整 video + advanced 快照', () => {
  const first = sampleAutomaticVideoParameters(20260825);
  const repeated = sampleAutomaticVideoParameters(20260825);
  const next = sampleAutomaticVideoParameters(20260826);

  assert.deepEqual(first, repeated);
  assert.notDeepEqual(first, next);
  assert.deepEqual(sortedKeys(first.video), expectedVideoFields);
  assert.deepEqual(sortedKeys(first.advanced), expectedAdvancedFields);
});

test('字段分类无遗漏、无重复且只排除水平和垂直翻转', () => {
  const videoClassification = AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.video;
  const classifiedVideo = [
    ...videoClassification.independentContinuous,
    ...videoClassification.switchWithDependent,
    ...videoClassification.dependent,
    ...videoClassification.excluded,
  ];
  const advancedClassification = AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.advanced;
  const classifiedAdvanced = [
    ...advancedClassification.independentContinuous,
    ...advancedClassification.switchWithDependent,
    ...advancedClassification.dependent,
  ];

  assert.deepEqual([...new Set(classifiedVideo)].sort(), expectedVideoFields);
  assert.equal(new Set(classifiedVideo).size, classifiedVideo.length);
  assert.deepEqual(videoClassification.excluded, [
    'horizontal_flip_enabled',
    'vertical_flip_enabled',
  ]);
  assert.deepEqual([...new Set(classifiedAdvanced)].sort(), expectedAdvancedFields);
  assert.equal(new Set(classifiedAdvanced).size, classifiedAdvanced.length);
});

test('自动模式保持强效果开关真实启用且使用低感知非零从属值', () => {
  for (let seed = 0; seed < 100; seed += 1) {
    const { video, advanced } = sampleAutomaticVideoParameters(seed);
    assert.equal(video.horizontal_flip_enabled, false);
    assert.equal(video.vertical_flip_enabled, false);
    assert.equal(video.space_x_offset_px, 0.5);
    assert.equal(video.space_y_offset_px, -0.5);
    assert.equal(video.color_space_conversion_enabled, true);
    assert.equal(video.image_repair_enabled, true);
    assert.equal(advanced.picture_in_picture_enabled, true);
    assert.equal(advanced.random_graphic_enabled, true);
    assert.equal(advanced.local_blur_enabled, true);
    assert.equal(advanced.edge_fill_enabled, true);
    assert.equal(advanced.highlight_perturbation_enabled, true);
    assert.equal(advanced.asynchronous_rotation_enabled, true);
    assert.ok(advanced.picture_in_picture_opacity_percent >= 0.8);
    assert.ok(advanced.picture_in_picture_opacity_percent <= 1.2);
    assert.ok(advanced.random_graphic_opacity_percent >= 0.8);
    assert.ok(advanced.random_graphic_opacity_percent <= 1.2);
    assert.ok(advanced.abstract_face_count >= 1);
    assert.ok(advanced.abstract_face_opacity_percent >= 0.8);
    assert.ok(advanced.abstract_face_opacity_percent <= 1.2);
    assert.ok(advanced.slice_trigger_interval_ms >= advanced.slice_length_ms);
    assert.ok(Object.values(advanced.band_weights).every((weight) => weight !== 1));
  }
});
