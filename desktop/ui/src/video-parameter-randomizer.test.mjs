import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./video-parameter-randomizer.ts', import.meta.url), 'utf8');
const definitionsSource = await readFile(
  new URL('./media-parameter-panels/parameter-definitions.ts', import.meta.url),
  'utf8',
);
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const commandsSource = await readFile(
  new URL('../../src-tauri/src/commands.rs', import.meta.url),
  'utf8',
);
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
const {
  AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX,
  AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION,
  VIDEO_PARAMETER_PERCEPTION_RANGES,
  hasCompleteAutomaticVideoAdmissionEvidence,
  hasCompleteCpuVideoFallbackEvidence,
  sampleAutomaticVideoParameters,
} = module;

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

function matrixValue(snapshot, { uiPath }) {
  const [section, field, band] = uiPath.split('.');
  return field === 'band_weights'
    ? snapshot.advanced.band_weights[band]
    : snapshot[section][field];
}

function implementedFields(section) {
  return [...definitionsSource.matchAll(
    new RegExp(`section: '${section}', field: '([^']+)'[^\\n]*status: 'implemented'`, 'g'),
  )].map((match) => match[1]).sort();
}

test('完整字段清单与参数面板当前 implemented 契约一致', () => {
  assert.deepEqual(expectedVideoFields, implementedFields('video'));
  assert.deepEqual(expectedAdvancedFields, implementedFields('advanced'));
});

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

test('85 UI 行映射到 74 个模型字段，仅排除两个翻转行', () => {
  const admitted = AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX.filter(
    ({ admission }) => admission === 'included',
  );
  const excluded = AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX.filter(
    ({ admission }) => admission === 'excluded',
  );

  assert.equal(AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX.length, 85);
  assert.equal(new Set(AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX.map(({ uiPath }) => uiPath)).size, 85);
  assert.equal(new Set(AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX.map(({ modelPath }) => modelPath)).size, 74);
  assert.equal(admitted.length, 83);
  assert.deepEqual(excluded.map(({ uiPath }) => uiPath).sort(), [
    'video.horizontal_flip_enabled',
    'video.vertical_flip_enabled',
  ]);
  assert.equal(
    admitted.filter(({ modelPath }) => modelPath === 'advanced.band_weights').length,
    12,
  );
});

test('原子证据精确匹配 83 个准入 UI 行', () => {
  const expected = AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX
    .filter(({ admission }) => admission === 'included')
    .map(({ uiPath }) => uiPath);

  assert.equal(hasCompleteAutomaticVideoAdmissionEvidence(expected), true);
  assert.equal(
    hasCompleteAutomaticVideoAdmissionEvidence([...expected, 'advanced.band_weights']),
    true,
  );
  assert.equal(hasCompleteAutomaticVideoAdmissionEvidence(expected.slice(1)), false);
  assert.equal(
    hasCompleteAutomaticVideoAdmissionEvidence([...expected.slice(1), 'unexpected.field']),
    false,
  );
  assert.equal(
    hasCompleteAutomaticVideoAdmissionEvidence([...expected, 'unexpected.field']),
    false,
  );
});

test('CPU 回退证据只精确接受四项基础色彩字段', () => {
  const expected = [
    'video.brightness_percent',
    'video.contrast_percent',
    'video.saturation_percent',
    'video.hue_rotation_degrees',
  ];
  assert.equal(hasCompleteCpuVideoFallbackEvidence(expected), true);
  assert.equal(hasCompleteCpuVideoFallbackEvidence(expected.slice(1)), false);
  assert.equal(hasCompleteCpuVideoFallbackEvidence([...expected, expected[0]]), false);
  assert.equal(hasCompleteCpuVideoFallbackEvidence([...expected, 'video.blur_radius_px']), false);
});

test('正式自动视频周期只交给 mpv GPU83 编译入口', () => {
  assert.match(appSource, /const videoEffectsEnabled = videoProcessingEnabledRef\.current/);
  assert.match(appSource, /const sample = videoEffectsEnabled \? sampleAutomaticVideoParameters\(\) : null/);
  assert.match(appSource, /video:\s*sample\s*\?\s*\{\s*\.\.\.base\.video,\s*\.\.\.sample\.video\s*\}\s*:\s*base\.video/);
  assert.match(appSource, /advanced:\s*sample\s*\?\s*\{\s*\.\.\.base\.advanced,\s*\.\.\.sample\.advanced\s*\}\s*:\s*base\.advanced/);
  assert.match(
    appSource,
    /const params = \{[\s\S]*?video: plan\.payload\.video,[\s\S]*?advanced: plan\.payload\.advanced,[\s\S]*?\};/,
  );
  assert.match(
    appSource,
    /prepare_realtime_video_plan[\s\S]*?request:\s*\{\s*params,/,
  );
  assert.match(appSource, /commit_realtime_video_plan/);
  assert.match(commandsSource, /compile_gpu83_realtime_parameters\(&request\.params\)/);
  assert.match(commandsSource, /MpvCommand::SetShaderOptions/);
  assert.doesNotMatch(appSource, /prepare_media_video_stream|read_media_video_stream|ack_media_video_stream|commit_media_video_stream|VideoMse|video_stream/);
});

test('自动变换使用低感知区间，明显可见区间只作返回边界', () => {
  assert.deepEqual(VIDEO_PARAMETER_PERCEPTION_RANGES.brightness_percent.visibleMagnitude, [2, 4]);
  assert.deepEqual(VIDEO_PARAMETER_PERCEPTION_RANGES.contrast_percent.visibleMagnitude, [3, 6]);
  assert.deepEqual(VIDEO_PARAMETER_PERCEPTION_RANGES.saturation_percent.visibleMagnitude, [3, 6]);
  assert.deepEqual(VIDEO_PARAMETER_PERCEPTION_RANGES.hue_rotation_degrees.visibleMagnitude, [2, 5]);
  for (let seed = 0; seed < 100; seed += 1) {
    const { video } = sampleAutomaticVideoParameters(seed);
    assert.ok(Math.abs(video.brightness_percent) >= 0.1);
    assert.ok(Math.abs(video.brightness_percent) <= 0.35);
    assert.ok(Math.abs(video.saturation_percent - 100) >= 0.099);
    assert.ok(Math.abs(video.saturation_percent - 100) <= 0.3);
    assert.ok(Math.abs(video.contrast_percent - 100) >= 0.099);
    assert.ok(Math.abs(video.contrast_percent - 100) <= 0.3);
    assert.ok(Math.abs(video.hue_rotation_degrees) >= 0.05);
    assert.ok(Math.abs(video.hue_rotation_degrees) <= 0.2);
  }
});

test('自动周期激活全部从属开关并提供可执行的非零输入', () => {
  for (let seed = 0; seed < 100; seed += 1) {
    const snapshot = sampleAutomaticVideoParameters(seed);
    const { video, advanced } = snapshot;
    assert.equal(video.horizontal_flip_enabled, false);
    assert.equal(video.vertical_flip_enabled, false);
    assert.equal(video.color_space_conversion_enabled, true);
    assert.equal(video.red_channel_lock_enabled, true);
    assert.equal(video.image_repair_enabled, true);
    assert.equal(video.frame_rate_lock_enabled, true);
    assert.equal(Number.isInteger(video.noise_percent), true);
    assert.ok(video.noise_percent >= 1);
    assert.ok(video.pixel_jitter_px >= 0.125);
    assert.ok(Math.abs(video.pixel_scale_percent - 100) >= 0.1);
    assert.ok(Math.abs(video.space_x_offset_px) >= 0.5);
    assert.ok(Math.abs(video.space_y_offset_px) >= 0.5);
    assert.equal(advanced.picture_in_picture_enabled, true);
    assert.equal(advanced.random_graphic_enabled, true);
    assert.equal(advanced.local_blur_enabled, true);
    assert.equal(advanced.edge_fill_enabled, true);
    assert.equal(advanced.transform_smoothing_enabled, true);
    assert.equal(advanced.highlight_perturbation_enabled, true);
    assert.equal(advanced.asynchronous_rotation_enabled, true);
    assert.equal(advanced.picture_in_picture_timeline_locked, false);
    assert.notEqual(advanced.target_frequency_hz, null);
    assert.notEqual(advanced.core_frequency_hz, null);
    assert.ok(advanced.wave_intensity >= 0.084);
    assert.ok(advanced.wave_level >= 0.251);
    assert.ok(Math.abs(advanced.channel_offset_percent) >= 0.1);
    assert.ok(advanced.picture_in_picture_pixel_jitter_px >= 0.125);
    assert.ok(advanced.picture_in_picture_opacity_percent >= 0.8);
    assert.ok(advanced.random_graphic_opacity_percent >= 0.8);
    assert.ok(advanced.abstract_face_count >= 1);
    assert.ok(advanced.abstract_face_opacity_percent >= 0.8);
    assert.ok(Math.abs(advanced.asynchronous_rotation_min_degrees) >= 0.1);
    assert.ok(Math.abs(advanced.asynchronous_rotation_max_degrees) >= 0.1);
    assert.ok(advanced.slice_trigger_interval_ms >= advanced.slice_length_ms);
    assert.ok(advanced.slice_trigger_interval_ms >= 5_000);
    assert.ok(advanced.slice_trigger_interval_ms <= 8_000);
    assert.ok(Object.values(advanced.band_weights).every((weight) => weight !== 1));

    for (const row of AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX) {
      if (row.admission === 'excluded') continue;
      const value = matrixValue(snapshot, row);
      assert.notEqual(value, undefined, row.uiPath);
      assert.notEqual(value, null, row.uiPath);
      if (typeof value === 'number') assert.notEqual(value, 0, row.uiPath);
      if (typeof value === 'boolean' && row.uiPath !== 'advanced.picture_in_picture_timeline_locked') {
        assert.equal(value, true, row.uiPath);
      }
    }
  }
});
