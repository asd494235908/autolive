import type {
  AdvancedEffectParams,
  VideoEffectParams,
} from './media-parameter-panels/media-parameter-types';

const VISUAL_BAND_FREQUENCIES_HZ = [
  65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20_000,
] as const;

/** 自动池只使用 lowPerceptionMagnitude；visibleMagnitude 仅是人工验收对照，不是跨素材的视觉阈值承诺。 */
export const VIDEO_PARAMETER_PERCEPTION_RANGES = {
  brightness_percent: { lowPerceptionMagnitude: [0.1, 0.35], visibleMagnitude: [2, 4] },
  contrast_percent: { lowPerceptionMagnitude: [0.1, 0.3], visibleMagnitude: [3, 6] },
  saturation_percent: { lowPerceptionMagnitude: [0.1, 0.3], visibleMagnitude: [3, 6] },
  hue_rotation_degrees: { lowPerceptionMagnitude: [0.05, 0.2], visibleMagnitude: [2, 5] },
  blur_radius_px: { lowPerceptionMagnitude: [0.01, 0.05], visibleMagnitude: [0.5, 2] },
  pixel_scale_percent: { lowPerceptionMagnitude: [0.1, 0.2], visibleMagnitude: [0.5, 1.5] },
  space_offset_px: { lowPerceptionMagnitude: [0.5, 0.5], visibleMagnitude: [1, 3] },
  rotation_degrees: { lowPerceptionMagnitude: [0.01, 0.04], visibleMagnitude: [0.2, 0.8] },
} as const;

/**
 * 自动模式字段分类。`dependent` 只有在所属效果启用时才有执行意义；
 * 水平/垂直翻转按产品边界保留在模型中，但不进入自动快照。
 */
export const AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION = {
  video: {
    independentContinuous: [
      'brightness_percent',
      'saturation_percent',
      'blur_radius_px',
      'contrast_percent',
      'hue_rotation_degrees',
      'sharpen_percent',
      'noise_percent',
      'detail_enhancement_percent',
      'frame_rate_jitter_percent',
      'frame_rate_perturbation_amplitude_fps',
      'pixel_scale_percent',
      'pixel_jitter_px',
      'dynamic_crop_percent',
      'frame_inner_perturbation_percent',
      'frame_inter_perturbation_percent',
      'space_x_offset_px',
      'space_y_offset_px',
      'rotation_degrees',
      'vignette_percent',
      'highlights_percent',
      'shadows_percent',
      'edge_softness_percent',
    ],
    switchWithDependent: [
      'color_space_conversion_enabled',
      'red_channel_lock_enabled',
      'image_repair_enabled',
      'frame_rate_lock_enabled',
    ],
    dependent: [
      'crop_edge_smoothing',
      'frame_rate_perturbation_frequency_hz',
      'color_space_conversion_strength_percent',
      'image_repair_strength_percent',
    ],
    excluded: ['horizontal_flip_enabled', 'vertical_flip_enabled'],
  },
  advanced: {
    independentContinuous: [
      'band_weights',
      'target_frequency_hz',
      'core_frequency_hz',
      'wave_intensity',
      'wave_level',
      'channel_offset_percent',
      'frame_perturbation_probability_percent',
    ],
    switchWithDependent: [
      'random_graphic_enabled',
      'picture_in_picture_enabled',
      'local_blur_enabled',
      'edge_fill_enabled',
      'transform_smoothing_enabled',
      'highlight_perturbation_enabled',
      'asynchronous_rotation_enabled',
    ],
    dependent: [
      'wave_grain_count',
      'dynamic_eq_threshold',
      'space_dimension',
      'frequency_space_x_offset_px',
      'frequency_space_y_offset_px',
      'random_graphic_opacity_percent',
      'random_graphic_size_px',
      'abstract_face_count',
      'abstract_face_size_percent',
      'abstract_face_opacity_percent',
      'overlay_offset_px',
      'slice_length_ms',
      'slice_min_length_ms',
      'slice_trigger_interval_ms',
      'random_graphic_count',
      'picture_in_picture_scale_percent',
      'picture_in_picture_opacity_percent',
      'picture_in_picture_rotation_degrees',
      'picture_in_picture_pixel_jitter_px',
      'picture_in_picture_timeline_locked',
      'local_blur_region_percent',
      'local_blur_radius_px',
      'local_blur_interval_ms',
      'edge_feather_percent',
      'transform_smoothing_duration_ms',
      'highlight_perturbation_interval_ms',
      'asynchronous_rotation_min_degrees',
      'asynchronous_rotation_max_degrees',
    ],
  },
} as const;

const automaticVideoModelFields = [
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.video.independentContinuous,
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.video.switchWithDependent,
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.video.dependent,
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.video.excluded,
] as const;
const automaticAdvancedModelFields = [
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.advanced.independentContinuous,
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.advanced.switchWithDependent,
  ...AUTOMATIC_VIDEO_PARAMETER_CLASSIFICATION.advanced.dependent,
] as const;

/** 85 个 UI 行到 74 个模型字段的自动准入唯一矩阵。 */
export const AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX = [
  ...automaticVideoModelFields.map((field) => ({
    uiPath: `video.${field}`,
    modelPath: `video.${field}`,
    admission: field === 'horizontal_flip_enabled' || field === 'vertical_flip_enabled'
      ? 'excluded'
      : 'included',
  } as const)),
  ...VISUAL_BAND_FREQUENCIES_HZ.map((frequencyHz) => ({
    uiPath: `advanced.band_weights.${frequencyHz}`,
    modelPath: 'advanced.band_weights',
    admission: 'included',
  } as const)),
  ...automaticAdvancedModelFields
    .filter((field) => field !== 'band_weights')
    .map((field) => ({
      uiPath: `advanced.${field}`,
      modelPath: `advanced.${field}`,
      admission: 'included',
    } as const)),
] as const;

const AUTOMATIC_VIDEO_APPLIED_UI_PATHS = AUTOMATIC_VIDEO_PARAMETER_ADMISSION_MATRIX
  .filter(({ admission }) => admission === 'included')
  .map(({ uiPath }) => uiPath);

const CPU_VIDEO_FALLBACK_APPLIED_UI_PATHS = [
  'video.brightness_percent',
  'video.contrast_percent',
  'video.saturation_percent',
  'video.hue_rotation_degrees',
] as const;

/** 内部计划可包含 Map 聚合证据；UI 原子门禁只精确比较 83 个准入行。 */
export function hasCompleteAutomaticVideoAdmissionEvidence(
  appliedFields: readonly string[],
): boolean {
  const uiEvidence = new Set(
    appliedFields.filter((field) => field !== 'advanced.band_weights'),
  );
  return uiEvidence.size === AUTOMATIC_VIDEO_APPLIED_UI_PATHS.length
    && AUTOMATIC_VIDEO_APPLIED_UI_PATHS.every((field) => uiEvidence.has(field));
}

/** CPU 回退只接受精确四项基础色彩证据，不把未执行字段提升为已生效。 */
export function hasCompleteCpuVideoFallbackEvidence(
  appliedFields: readonly string[],
): boolean {
  const evidence = new Set(appliedFields);
  return appliedFields.length === CPU_VIDEO_FALLBACK_APPLIED_UI_PATHS.length
    && evidence.size === CPU_VIDEO_FALLBACK_APPLIED_UI_PATHS.length
    && CPU_VIDEO_FALLBACK_APPLIED_UI_PATHS.every((field) => evidence.has(field));
}

export interface AutomaticVideoParameterSnapshot {
  seed: number;
  video: VideoEffectParams;
  advanced: AdvancedEffectParams;
}

function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state += 0x6d2b79f5;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}

function roundTo(value: number, digits: number): number {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

/** 生成排除两个翻转后、83 个 UI 行同时参加的低感知原子快照。 */
export function sampleAutomaticVideoParameters(
  seed = Math.floor(Math.random() * 0x1_0000_0000),
): AutomaticVideoParameterSnapshot {
  const normalizedSeed = Number.isFinite(seed) ? Math.floor(seed) >>> 0 : 0;
  const random = mulberry32(normalizedSeed);
  const inRange = (minimum: number, maximum: number) => minimum + random() * (maximum - minimum);
  const signed = (minimumMagnitude: number, maximumMagnitude: number) =>
    (random() < 0.5 ? -1 : 1) * inRange(minimumMagnitude, maximumMagnitude);
  const integer = (minimum: number, maximum: number) =>
    minimum + Math.floor(random() * (maximum - minimum + 1));
  const steppedInteger = (minimum: number, maximum: number, step: number) =>
    integer(Math.ceil(minimum / step), Math.floor(maximum / step)) * step;
  const bandWeights = Object.fromEntries(
    VISUAL_BAND_FREQUENCIES_HZ.map((frequencyHz, index) => [
      String(frequencyHz),
      roundTo(1 + (index % 2 === 0 ? -1 : 1) * inRange(0.001, 0.003), 4),
    ]),
  );
  const lowRange = (field: keyof typeof VIDEO_PARAMETER_PERCEPTION_RANGES) =>
    VIDEO_PARAMETER_PERCEPTION_RANGES[field].lowPerceptionMagnitude;
  const signedLow = (field: keyof typeof VIDEO_PARAMETER_PERCEPTION_RANGES) => {
    const [minimum, maximum] = lowRange(field);
    return signed(minimum, maximum);
  };

  return {
    seed: normalizedSeed,
    video: {
      brightness_percent: roundTo(signedLow('brightness_percent'), 3),
      saturation_percent: roundTo(100 + signedLow('saturation_percent'), 3),
      blur_radius_px: roundTo(inRange(
        lowRange('blur_radius_px')[0],
        lowRange('blur_radius_px')[1],
      ), 3),
      contrast_percent: roundTo(100 + signedLow('contrast_percent'), 3),
      hue_rotation_degrees: roundTo(signedLow('hue_rotation_degrees'), 3),
      sharpen_percent: roundTo(inRange(0.1, 0.4), 3),
      // FFmpeg noise 的强度是整数语义，1 是不会被量化为零的最小输入。
      noise_percent: 1,
      detail_enhancement_percent: roundTo(inRange(0.1, 0.35), 3),
      crop_edge_smoothing: roundTo(inRange(0.7, 0.9), 3),
      frame_rate_jitter_percent: roundTo(inRange(0.01, 0.04), 3),
      frame_rate_perturbation_frequency_hz: roundTo(inRange(0.05, 0.18), 3),
      frame_rate_perturbation_amplitude_fps: roundTo(inRange(0.01, 0.04), 3),
      // 后端固定输出画布并强制偶数尺寸，这里只提供非中性缩放意图。
      pixel_scale_percent: roundTo(100 + signedLow('pixel_scale_percent'), 3),
      pixel_jitter_px: roundTo(inRange(0.125, 0.375), 3),
      dynamic_crop_percent: roundTo(inRange(0.03, 0.1), 3),
      frame_inner_perturbation_percent: roundTo(inRange(0.01, 0.04), 3),
      frame_inter_perturbation_percent: roundTo(inRange(0.03, 0.1), 3),
      space_x_offset_px: signed(0.5, 0.5),
      space_y_offset_px: signed(0.5, 0.5),
      color_space_conversion_strength_percent: roundTo(inRange(0.1, 0.4), 3),
      color_space_conversion_enabled: true,
      horizontal_flip_enabled: false,
      vertical_flip_enabled: false,
      rotation_degrees: roundTo(signedLow('rotation_degrees'), 3),
      vignette_percent: roundTo(inRange(0.05, 0.2), 3),
      highlights_percent: roundTo(signed(0.05, 0.2), 3),
      shadows_percent: roundTo(signed(0.05, 0.2), 3),
      red_channel_lock_enabled: true,
      edge_softness_percent: roundTo(inRange(0.05, 0.2), 3),
      image_repair_enabled: true,
      image_repair_strength_percent: roundTo(inRange(0.05, 0.2), 3),
      frame_rate_lock_enabled: true,
    },
    advanced: {
      band_weights: bandWeights,
      target_frequency_hz: integer(65, 20_000),
      core_frequency_hz: integer(65, 20_000),
      // 分别稍高于映射后的 8-bit 单码值阈值，避免量化为零。
      wave_intensity: roundTo(inRange(0.084, 0.1), 4),
      wave_level: roundTo(inRange(0.251, 0.3), 4),
      wave_grain_count: integer(7, 13),
      dynamic_eq_threshold: roundTo(inRange(0.1, 0.3), 3),
      channel_offset_percent: roundTo(signed(0.1, 0.3), 3),
      space_dimension: integer(2, 3),
      frequency_space_x_offset_px: roundTo(signed(0.1, 0.3), 3),
      frequency_space_y_offset_px: roundTo(signed(0.1, 0.3), 3),
      frame_perturbation_probability_percent: roundTo(inRange(0.1, 0.3), 3),
      random_graphic_opacity_percent: roundTo(inRange(0.8, 1.2), 3),
      random_graphic_size_px: roundTo(inRange(1, 2), 3),
      abstract_face_count: 1,
      abstract_face_size_percent: roundTo(inRange(1, 1.5), 3),
      abstract_face_opacity_percent: roundTo(inRange(0.8, 1.2), 3),
      overlay_offset_px: signed(0.5, 0.5),
      slice_length_ms: steppedInteger(500, 800, 100),
      slice_min_length_ms: steppedInteger(1_000, 1_500, 100),
      slice_trigger_interval_ms: steppedInteger(5_000, 8_000, 100),
      random_graphic_enabled: true,
      random_graphic_count: 1,
      picture_in_picture_enabled: true,
      picture_in_picture_scale_percent: roundTo(inRange(10, 12), 3),
      picture_in_picture_opacity_percent: roundTo(inRange(0.8, 1.2), 3),
      picture_in_picture_rotation_degrees: roundTo(signed(0.1, 0.2), 3),
      picture_in_picture_pixel_jitter_px: roundTo(inRange(0.125, 0.375), 3),
      picture_in_picture_timeline_locked: false,
      local_blur_enabled: true,
      local_blur_region_percent: roundTo(inRange(5, 7), 3),
      local_blur_radius_px: roundTo(inRange(0.1, 0.2), 3),
      local_blur_interval_ms: steppedInteger(5_000, 8_000, 100),
      edge_fill_enabled: true,
      edge_feather_percent: roundTo(inRange(0.1, 0.4), 3),
      transform_smoothing_enabled: true,
      transform_smoothing_duration_ms: steppedInteger(100, 250, 10),
      highlight_perturbation_enabled: true,
      highlight_perturbation_interval_ms: steppedInteger(5_000, 8_000, 100),
      asynchronous_rotation_enabled: true,
      asynchronous_rotation_min_degrees: roundTo(-inRange(0.1, 0.2), 3),
      asynchronous_rotation_max_degrees: roundTo(inRange(0.1, 0.2), 3),
    },
  };
}
