import { createHash } from 'node:crypto';

export const CANDIDATE_DIRECTORY = 'desktop/tools/shader-candidates';
export const CANDIDATE_FILENAMES = Object.freeze([
  'gpu83-baseline-candidate.hook',
  'gpu83-image-repair-candidate.hook',
  'gpu83-abstract-face-candidate.hook',
  'gpu83-same-frame-overlays-candidate.hook',
  'gpu83-local-blur-candidate.hook',
  'gpu83-edge-fill-candidate.hook',
  'gpu83-channel-offset-candidate.hook',
  'gpu83-spatial-modulation-candidate.hook',
  'gpu83-scheduler-gates-candidate.hook',
]);
export const CANDIDATE_PATHS = Object.freeze(
  CANDIDATE_FILENAMES.map((filename) => `${CANDIDATE_DIRECTORY}/${filename}`),
);
export const BASELINE_PARAMETER_COUNT = 20;
export const ADDITIONAL_PARAMETER_COUNT = 41;
export const TOTAL_PARAMETER_COUNT = 61;
export const RUNTIME_OPTION_COUNT = 11;
export const FULL_SNAPSHOT_OPTION_COUNT = TOTAL_PARAMETER_COUNT + RUNTIME_OPTION_COUNT;
export const LOGICAL_SCENARIO_COUNT = TOTAL_PARAMETER_COUNT + RUNTIME_OPTION_COUNT + 1;
export const ISOLATION_SCENARIO_COUNT = 1;

export const RUNTIME_OPTION_NAMES = Object.freeze([
  'al_runtime_frame_inner_active',
  'al_runtime_frame_inter_active',
  'al_runtime_frame_probability_active',
  'al_runtime_slice_active',
  'al_runtime_local_blur_active',
  'al_runtime_highlight_active',
  'al_runtime_pip_jitter_x_px',
  'al_runtime_pip_jitter_y_px',
  'al_runtime_random_graphic_seed',
  'al_runtime_async_rotation_degrees',
  'al_runtime_transform_easing',
]);
export const RUNTIME_NEUTRAL_OPTIONS = Object.freeze(
  Object.fromEntries(RUNTIME_OPTION_NAMES.map((name) => [name, 0])),
);
export const RUNTIME_ACTIVE_OPTIONS = Object.freeze({
  al_runtime_frame_inner_active: 1,
  al_runtime_frame_inter_active: 1,
  al_runtime_frame_probability_active: 1,
  al_runtime_slice_active: 1,
  al_runtime_local_blur_active: 1,
  al_runtime_highlight_active: 1,
  al_runtime_pip_jitter_x_px: 4,
  al_runtime_pip_jitter_y_px: -3,
  al_runtime_random_graphic_seed: 1193046,
  al_runtime_async_rotation_degrees: 6,
  al_runtime_transform_easing: 0.5,
});

const EXPECTED_PARAMETERS_BY_FILENAME = Object.freeze({
  'gpu83-baseline-candidate.hook': Object.freeze([
    'al_brightness_percent',
    'al_saturation_percent',
    'al_blur_radius_px',
    'al_contrast_percent',
    'al_hue_degrees',
    'al_sharpen_percent',
    'al_noise_percent',
    'al_detail_percent',
    'al_crop_edge_smoothing',
    'al_pixel_scale_percent',
    'al_pixel_jitter_px',
    'al_dynamic_crop_percent',
    'al_space_x_px',
    'al_space_y_px',
    'al_rotation_degrees',
    'al_vignette_percent',
    'al_highlights_percent',
    'al_shadows_percent',
    'al_red_lock_enabled',
    'al_edge_softness_percent',
  ]),
  'gpu83-image-repair-candidate.hook': Object.freeze([
    'al_image_repair_enabled',
    'al_image_repair_strength_percent',
  ]),
  'gpu83-abstract-face-candidate.hook': Object.freeze([
    'al_abstract_face_count',
    'al_abstract_face_size_percent',
    'al_abstract_face_opacity_percent',
  ]),
  'gpu83-same-frame-overlays-candidate.hook': Object.freeze([
    'al_random_graphic_enabled',
    'al_random_graphic_count',
    'al_random_graphic_opacity_percent',
    'al_random_graphic_size_px',
    'al_overlay_offset_px',
    'al_pip_enabled',
    'al_pip_scale_percent',
    'al_pip_opacity_percent',
    'al_pip_rotation_degrees',
    'al_runtime_pip_jitter_x_px',
    'al_runtime_pip_jitter_y_px',
    'al_runtime_random_graphic_seed',
  ]),
  'gpu83-local-blur-candidate.hook': Object.freeze([
    'al_local_blur_enabled',
    'al_runtime_local_blur_active',
    'al_local_blur_region_percent',
    'al_local_blur_radius_px',
  ]),
  'gpu83-edge-fill-candidate.hook': Object.freeze([
    'al_edge_fill_enabled',
    'al_edge_feather_percent',
  ]),
  'gpu83-channel-offset-candidate.hook': Object.freeze([
    'al_channel_offset_percent',
  ]),
  'gpu83-spatial-modulation-candidate.hook': Object.freeze([
    'al_target_frequency_hz',
    'al_core_frequency_hz',
    'al_wave_intensity',
    'al_wave_level',
    'al_wave_grain_count',
    'al_dynamic_eq_threshold',
    'al_space_dimension',
    'al_frequency_space_x_px',
    'al_frequency_space_y_px',
    'al_band_65',
    'al_band_92',
    'al_band_131',
    'al_band_188',
    'al_band_267',
    'al_band_381',
    'al_band_544',
    'al_band_777',
    'al_band_1110',
    'al_band_1585',
    'al_band_2263',
    'al_band_20000',
  ]),
  'gpu83-scheduler-gates-candidate.hook': Object.freeze([
    'al_runtime_frame_inner_active',
    'al_runtime_frame_inter_active',
    'al_runtime_frame_probability_active',
    'al_runtime_slice_active',
    'al_runtime_highlight_active',
    'al_runtime_async_rotation_degrees',
    'al_runtime_transform_easing',
  ]),
});
const EXPECTED_PARAMETER_NAMES = Object.freeze(
  CANDIDATE_FILENAMES.flatMap((filename) => EXPECTED_PARAMETERS_BY_FILENAME[filename]),
);
const EXPECTED_PARAMETER_NAME_SET = new Set(EXPECTED_PARAMETER_NAMES);
const RUNTIME_OPTION_NAME_SET = new Set(RUNTIME_OPTION_NAMES);

const hash = (source) => createHash('sha256').update(source, 'utf8').digest('hex');

function productionShaderSource(filename, source) {
  const descriptions = [...source.matchAll(/^\/\/!DESC (.+)$/gm)];
  if (descriptions.length !== 1) throw new Error(`${filename} 必须且只能声明一个 DESC`);
  const description = descriptions[0][1];
  if (filename === CANDIDATE_FILENAMES[0]) {
    if (description !== 'AutoLive verified realtime pixel effects') {
      throw new Error(`${filename} 的生产 DESC 漂移`);
    }
    return source;
  }
  if (!description.endsWith(' candidate')) throw new Error(`${filename} 缺少候选 DESC`);
  const stableDescription = description
    .replace(/^AutoLive Phase 3A /, 'AutoLive verified realtime ')
    .replace(/^GPU83 /, 'AutoLive verified realtime ')
    .replace(/ candidate$/, '');
  if (/candidate|Phase 3A/.test(stableDescription) || stableDescription === description) {
    throw new Error(`${filename} 无法规范化生产 DESC`);
  }
  return source.replace(descriptions[0][0], `//!DESC ${stableDescription}`);
}

export function parseRustGpu83Mappings(source) {
  const mappings = [];
  const standard = /(?:video_value|video_flag|advanced_value|advanced_flag|advanced_optional)!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*[A-Za-z0-9_]+\s*,\s*[A-Za-z0-9_]+\s*,\s*(AVAILABLE|UNVERIFIED|SCHEDULER|HISTORY)\s*\)/g;
  for (const match of String(source).matchAll(standard)) {
    mappings.push({ fieldPath: match[1], shaderOption: match[2], capability: match[3] });
  }
  const visualBand = /visual_band!\(\s*[0-9_]+\s*,\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)/g;
  for (const match of String(source).matchAll(visualBand)) {
    mappings.push({ fieldPath: match[1], shaderOption: match[2], capability: 'AVAILABLE' });
  }
  return mappings;
}

function canonicalCandidatePath(value) {
  if (typeof value !== 'string' || value.length === 0) throw new TypeError('候选路径必须是非空字符串');
  const normalized = value.replaceAll('\\', '/');
  if (!CANDIDATE_PATHS.includes(normalized)) throw new Error(`不允许的候选路径: ${value}`);
  return normalized;
}

export function parseCandidateParameters(source, path = '<candidate>') {
  if (typeof source !== 'string' || source.trim().length === 0) {
    throw new TypeError(`${path} 内容必须是非空字符串`);
  }
  if (!/^\/\/!HOOK MAIN\s*$/m.test(source) || !/^\/\/!BIND HOOKED\s*$/m.test(source)) {
    throw new Error(`${path} 缺少 MAIN/HOOKED 候选入口`);
  }

  const declarations = [...source.matchAll(/^\/\/!PARAM\s+(\S+)\s*$/gm)];
  const parameters = [];
  for (let index = 0; index < declarations.length; index += 1) {
    const declaration = declarations[index];
    const name = declaration[1];
    if (!name.startsWith('al_')) continue;
    if (!/^al_[A-Za-z0-9_]+$/.test(name)) throw new Error(`${path} 包含不安全参数名: ${name}`);

    const blockStart = declaration.index + declaration[0].length;
    const nextDeclaration = declarations[index + 1]?.index ?? source.length;
    const hookIndex = source.indexOf('//!HOOK', blockStart);
    const blockEnd = hookIndex >= 0 && hookIndex < nextDeclaration ? hookIndex : nextDeclaration;
    const lines = source.slice(blockStart, blockEnd).split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
    const metadata = Object.fromEntries(lines
      .filter((line) => line.startsWith('//!'))
      .map((line) => {
        const match = /^\/\/!(\S+)\s+(.+)$/.exec(line);
        return match ? [match[1], match[2]] : [];
      })
      .filter((entry) => entry.length === 2));
    const defaultLine = lines.find((line) => !line.startsWith('//!'));
    const minimum = Number(metadata.MINIMUM);
    const maximum = Number(metadata.MAXIMUM);
    const defaultValue = Number(defaultLine);
    if (metadata.TYPE !== 'float'
      || !Number.isFinite(minimum)
      || !Number.isFinite(maximum)
      || !Number.isFinite(defaultValue)
      || minimum > defaultValue
      || defaultValue > maximum) {
      throw new Error(`${path} 参数契约无效: ${name}`);
    }
    parameters.push({ name, minimum, maximum, defaultValue });
  }
  return parameters;
}

const BANDS = Object.freeze([65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20000]);

const pairScenario = (field, neutralValue, activeValue, context = {}, gateFields = []) => ({
  field,
  neutralValue,
  activeValue,
  context,
  gateFields,
});

const productScenario = (
  field,
  lowValue,
  activeValue,
  context = {},
  gateFields = [],
  mode = 'product_progression',
) => ({ field, lowValue, activeValue, context, gateFields, mode });

const productSwitch = (field, activeValue, context = {}, gateFields = []) => ({
  field,
  activeValue,
  context,
  gateFields,
  mode: 'product_switch',
});

const productResponse = (
  field,
  lowValue,
  activeValue,
  context = {},
  gateFields = [],
) => productScenario(
  field,
  lowValue,
  activeValue,
  context,
  gateFields,
  'product_response',
);

const BASELINE_SCENARIO_SPECS = Object.freeze([
  productScenario('al_brightness_percent', 1, 25),
  productScenario('al_saturation_percent', 101, 150),
  productScenario('al_blur_radius_px', 0.1, 4),
  productScenario('al_contrast_percent', 101, 150),
  productScenario('al_hue_degrees', 1, 45),
  productScenario('al_sharpen_percent', 1, 50),
  productScenario('al_noise_percent', 0.1, 4),
  productScenario('al_detail_percent', 1, 25),
  productScenario('al_crop_edge_smoothing', 0.51, 1,
    { al_dynamic_crop_percent: 3 }, ['al_dynamic_crop_percent']),
  productScenario('al_pixel_scale_percent', 100.1, 103),
  productScenario('al_pixel_jitter_px', 0.1, 1.5),
  productScenario('al_dynamic_crop_percent', 0.1, 3),
  productScenario('al_space_x_px', 0.1, 3),
  productScenario('al_space_y_px', -0.1, -3),
  productScenario('al_rotation_degrees', 0.25, 15),
  productScenario('al_vignette_percent', 1, 50),
  productScenario('al_highlights_percent', 1, 50),
  productScenario('al_shadows_percent', 1, 50),
  productSwitch('al_red_lock_enabled', 1, { al_hue_degrees: 45 }, ['al_hue_degrees']),
  productScenario('al_edge_softness_percent', 1, 50),
]);

const ADDITIONAL_SCENARIO_SPECS = Object.freeze([
  productSwitch('al_image_repair_enabled', 1,
    { al_image_repair_strength_percent: 100 }, ['al_image_repair_strength_percent']),
  productScenario('al_image_repair_strength_percent', 1, 100,
    { al_image_repair_enabled: 1 }, ['al_image_repair_enabled']),
  productScenario('al_abstract_face_count', 1, 3,
    { al_abstract_face_size_percent: 4, al_abstract_face_opacity_percent: 20 },
    ['al_abstract_face_size_percent', 'al_abstract_face_opacity_percent']),
  productScenario('al_abstract_face_size_percent', 1, 6,
    { al_abstract_face_count: 3, al_abstract_face_opacity_percent: 20 },
    ['al_abstract_face_count', 'al_abstract_face_opacity_percent']),
  productScenario('al_abstract_face_opacity_percent', 1, 20,
    { al_abstract_face_count: 3, al_abstract_face_size_percent: 4 },
    ['al_abstract_face_count']),
  productSwitch('al_random_graphic_enabled', 1,
    { al_random_graphic_count: 8, al_random_graphic_opacity_percent: 50, al_random_graphic_size_px: 32 },
    ['al_random_graphic_opacity_percent']),
  productScenario('al_random_graphic_count', 1, 16,
    { al_random_graphic_enabled: 1, al_random_graphic_opacity_percent: 50, al_random_graphic_size_px: 32 },
    ['al_random_graphic_enabled', 'al_random_graphic_opacity_percent']),
  productScenario('al_random_graphic_opacity_percent', 1, 50,
    { al_random_graphic_enabled: 1, al_random_graphic_count: 8, al_random_graphic_size_px: 32 },
    ['al_random_graphic_enabled']),
  productScenario('al_random_graphic_size_px', 8, 48,
    { al_random_graphic_enabled: 1, al_random_graphic_count: 8, al_random_graphic_opacity_percent: 50 },
    ['al_random_graphic_enabled', 'al_random_graphic_opacity_percent']),
  productResponse('al_overlay_offset_px', 0.25, 10,
    { al_random_graphic_enabled: 1, al_random_graphic_count: 8, al_random_graphic_opacity_percent: 50, al_random_graphic_size_px: 32 },
    ['al_random_graphic_enabled', 'al_random_graphic_opacity_percent']),
  productSwitch('al_pip_enabled', 1,
    { al_pip_scale_percent: 35, al_pip_opacity_percent: 80, al_pip_rotation_degrees: 5 },
    ['al_pip_opacity_percent']),
  productScenario('al_pip_scale_percent', 10, 45,
    { al_pip_enabled: 1, al_pip_opacity_percent: 80 }, ['al_pip_enabled', 'al_pip_opacity_percent']),
  productScenario('al_pip_opacity_percent', 90, 50,
    { al_pip_enabled: 1, al_pip_scale_percent: 35 }, ['al_pip_enabled']),
  productScenario('al_pip_rotation_degrees', 0.25, 15,
    { al_pip_enabled: 1, al_pip_scale_percent: 35, al_pip_opacity_percent: 80 },
    ['al_pip_enabled', 'al_pip_opacity_percent']),
  productSwitch('al_local_blur_enabled', 1,
    { al_local_blur_region_percent: 30, al_local_blur_radius_px: 8, al_runtime_local_blur_active: 1 },
    ['al_local_blur_region_percent', 'al_local_blur_radius_px', 'al_runtime_local_blur_active']),
  productScenario('al_local_blur_region_percent', 5, 40,
    { al_local_blur_enabled: 1, al_local_blur_radius_px: 8, al_runtime_local_blur_active: 1 },
    ['al_local_blur_enabled', 'al_runtime_local_blur_active']),
  productScenario('al_local_blur_radius_px', 3, 8,
    { al_local_blur_enabled: 1, al_local_blur_region_percent: 30, al_runtime_local_blur_active: 1 },
    ['al_local_blur_enabled', 'al_runtime_local_blur_active']),
  productSwitch('al_edge_fill_enabled', 1,
    { al_edge_feather_percent: 100 }, ['al_edge_feather_percent']),
  productScenario('al_edge_feather_percent', 1, 100,
    { al_edge_fill_enabled: 1 }, ['al_edge_fill_enabled']),
  productScenario('al_channel_offset_percent', 0.1, 6),
  productResponse('al_target_frequency_hz', 65, 1000),
  productResponse('al_core_frequency_hz', 500, 65,
    { al_target_frequency_hz: 1000 }, ['al_target_frequency_hz']),
  productScenario('al_wave_intensity', 0.01, 0.6),
  productScenario('al_wave_level', 0.01, 0.25),
  productResponse('al_wave_grain_count', 21, 60,
    { al_wave_intensity: 0.6 }, ['al_wave_intensity']),
  productResponse('al_dynamic_eq_threshold', 2, 1,
    { al_target_frequency_hz: 1000, al_band_65: 1.5 }, ['al_target_frequency_hz', 'al_band_65']),
  productResponse('al_space_dimension', 1, 3,
    { al_target_frequency_hz: 1000 }, ['al_target_frequency_hz']),
  productScenario('al_frequency_space_x_px', 0.1, 6,
    { al_wave_intensity: 0.6 }, ['al_wave_intensity']),
  productScenario('al_frequency_space_y_px', -0.1, -6,
    { al_wave_intensity: 0.6 }, ['al_wave_intensity']),
  ...BANDS.map((frequency) => productScenario(`al_band_${frequency}`, 1.01, 1.5)),
]);

const PRODUCT_SCENARIO_SPECS = Object.freeze([...BASELINE_SCENARIO_SPECS, ...ADDITIONAL_SCENARIO_SPECS]);
export const BASELINE_PARAMETER_NAMES = Object.freeze(BASELINE_SCENARIO_SPECS.map(({ field }) => field));
export const ADDITIONAL_PARAMETER_NAMES = Object.freeze(ADDITIONAL_SCENARIO_SPECS.map(({ field }) => field));
export const PRODUCT_PARAMETER_NAMES = Object.freeze(PRODUCT_SCENARIO_SPECS.map(({ field }) => field));

function assertCompleteSnapshot(defaultOptions) {
  if (!defaultOptions || typeof defaultOptions !== 'object' || Array.isArray(defaultOptions)) {
    throw new TypeError('默认 shader 选项必须是对象');
  }
  const keys = Object.keys(defaultOptions);
  if (keys.length !== FULL_SNAPSHOT_OPTION_COUNT
    || keys.some((name) => !EXPECTED_PARAMETER_NAME_SET.has(name) || !Number.isFinite(defaultOptions[name]))
    || EXPECTED_PARAMETER_NAMES.some((name) => !Object.hasOwn(defaultOptions, name))) {
    throw new Error(`完整默认快照必须精确包含固定的 ${FULL_SNAPSHOT_OPTION_COUNT} 个有限 al_ 选项`);
  }
}

export function buildFrameDifferenceScenarios(defaultOptions) {
  assertCompleteSnapshot(defaultOptions);

  return PRODUCT_SCENARIO_SPECS.map(({
    field, lowValue, activeValue, context, gateFields, mode,
  }) => {
    const baselineOptions = { ...defaultOptions, ...context, [field]: defaultOptions[field] };
    const activeOptions = { ...baselineOptions, [field]: activeValue };
    if (mode === 'product_switch') {
      return {
        field,
        mode,
        gateFields: [...gateFields],
        baselineOptions,
        activeOptions,
        states: [
          { role: 'baseline', options: baselineOptions },
          { role: 'active', options: activeOptions },
        ],
      };
    }
    const lowOptions = { ...baselineOptions, [field]: lowValue };
    return {
      field,
      mode,
      gateFields: [...gateFields],
      baselineOptions,
      lowOptions,
      activeOptions,
      states: [
        { role: 'baseline', options: baselineOptions },
        { role: 'low', options: lowOptions },
        { role: 'active', options: activeOptions },
      ],
    };
  });
}

export function buildCombinedActiveOptions(defaultOptions) {
  assertCompleteSnapshot(defaultOptions);
  return Object.freeze({
    ...defaultOptions,
    ...Object.fromEntries(PRODUCT_SCENARIO_SPECS.map(({ field, activeValue }) => [field, activeValue])),
    ...RUNTIME_ACTIVE_OPTIONS,
  });
}

export function buildRuntimeSchedulerScenario(
  defaultOptions,
  combinedActiveOptions = buildCombinedActiveOptions(defaultOptions),
) {
  assertCompleteSnapshot(defaultOptions);
  assertCompleteSnapshot(combinedActiveOptions);
  const neutralOptions = { ...defaultOptions };
  return {
    field: 'runtime_scheduler_gates',
    mode: 'pair_difference',
    gateFields: [],
    runtimeFields: [...RUNTIME_OPTION_NAMES],
    neutralOptions,
    activeOptions: combinedActiveOptions,
    states: [
      { role: 'neutral', options: neutralOptions },
      { role: 'active', options: combinedActiveOptions },
    ],
  };
}

const RUNTIME_SCENARIO_SPECS = Object.freeze([
  pairScenario('al_runtime_frame_inner_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_frame_inner_active),
  pairScenario('al_runtime_frame_inter_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_frame_inter_active),
  pairScenario('al_runtime_frame_probability_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_frame_probability_active),
  pairScenario('al_runtime_slice_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_slice_active, {
    al_runtime_transform_easing: 1,
  }, ['al_runtime_transform_easing']),
  pairScenario('al_runtime_local_blur_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_local_blur_active, {
    al_local_blur_enabled: 1,
    al_local_blur_region_percent: 30,
    al_local_blur_radius_px: 8,
  }, ['al_local_blur_enabled', 'al_local_blur_region_percent', 'al_local_blur_radius_px']),
  pairScenario('al_runtime_highlight_active', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_highlight_active),
  pairScenario('al_runtime_pip_jitter_x_px', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_pip_jitter_x_px, {
    al_pip_enabled: 1,
    al_pip_scale_percent: 35,
    al_pip_opacity_percent: 80,
  }, ['al_pip_enabled', 'al_pip_opacity_percent']),
  pairScenario('al_runtime_pip_jitter_y_px', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_pip_jitter_y_px, {
    al_pip_enabled: 1,
    al_pip_scale_percent: 35,
    al_pip_opacity_percent: 80,
  }, ['al_pip_enabled', 'al_pip_opacity_percent']),
  pairScenario('al_runtime_random_graphic_seed', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_random_graphic_seed, {
    al_random_graphic_enabled: 1,
    al_random_graphic_count: 8,
    al_random_graphic_opacity_percent: 50,
    al_random_graphic_size_px: 32,
  }, ['al_random_graphic_enabled', 'al_random_graphic_opacity_percent']),
  pairScenario('al_runtime_async_rotation_degrees', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_async_rotation_degrees, {
    al_runtime_slice_active: 1,
    al_runtime_transform_easing: 1,
  }, ['al_runtime_slice_active', 'al_runtime_transform_easing']),
  pairScenario('al_runtime_transform_easing', 0, RUNTIME_ACTIVE_OPTIONS.al_runtime_transform_easing, {
    al_runtime_slice_active: 1,
  }, ['al_runtime_slice_active']),
]);

export function buildRuntimeSchedulerScenarios(defaultOptions) {
  assertCompleteSnapshot(defaultOptions);
  return RUNTIME_SCENARIO_SPECS.map(({ field, neutralValue, activeValue, context, gateFields }) => {
    const neutralOptions = { ...defaultOptions, ...context, [field]: neutralValue };
    const activeOptions = { ...defaultOptions, ...context, [field]: activeValue };
    return {
      field,
      mode: 'pair_difference',
      gateFields: [...gateFields],
      neutralOptions,
      activeOptions,
      states: [
        { role: 'neutral', options: neutralOptions },
        { role: 'active', options: activeOptions },
      ],
    };
  });
}

export function buildRandomSeedPipIsolationScenario(defaultOptions) {
  assertCompleteSnapshot(defaultOptions);
  const context = {
    ...defaultOptions,
    al_pip_enabled: 1,
    al_pip_scale_percent: 35,
    al_pip_opacity_percent: 80,
    al_random_graphic_enabled: 0,
  };
  const seedZeroOptions = { ...context, al_runtime_random_graphic_seed: 0 };
  const seedChangedOptions = {
    ...context,
    al_runtime_random_graphic_seed: RUNTIME_ACTIVE_OPTIONS.al_runtime_random_graphic_seed,
  };
  return {
    field: 'runtime_random_seed_pip_isolation',
    mode: 'expected_same',
    gateFields: ['al_pip_enabled'],
    states: [
      { role: 'seed_zero', options: seedZeroOptions },
      { role: 'seed_changed', options: seedChangedOptions },
    ],
  };
}

function assertExactFieldSet(label, actualFields, expectedFields) {
  if (actualFields.length !== expectedFields.length
    || new Set(actualFields).size !== actualFields.length
    || expectedFields.some((field) => !actualFields.includes(field))) {
    throw new Error(`${label}字段覆盖必须精确且唯一: ${expectedFields.length}`);
  }
}

function assertBoundedSnapshot(label, options, parameterByName) {
  assertCompleteSnapshot(options);
  for (const [name, value] of Object.entries(options)) {
    const parameter = parameterByName.get(name);
    if (value < parameter.minimum || value > parameter.maximum) {
      throw new Error(`${label} 参数越界: ${name}=${value}`);
    }
  }
}

export function assertScenarioOnlyChangesTarget(scenario, targetField = scenario?.field) {
  if (!scenario || !Array.isArray(scenario.states) || scenario.states.length < 2
    || typeof targetField !== 'string' || targetField.length === 0) {
    throw new TypeError('单变量场景必须提供至少两个状态和目标字段');
  }
  const [baseline, ...comparisons] = scenario.states.map(({ options }) => options);
  if (!baseline || !Object.hasOwn(baseline, targetField)) {
    throw new Error(`单变量场景缺少目标字段: ${scenario.field}/${targetField}`);
  }
  const baselineKeys = Object.keys(baseline);
  for (const options of comparisons) {
    const keys = Object.keys(options ?? {});
    const changedKeys = baselineKeys.filter((name) => baseline[name] !== options?.[name]);
    if (keys.length !== baselineKeys.length
      || keys.some((name) => !Object.hasOwn(baseline, name))
      || changedKeys.length !== 1
      || changedKeys[0] !== targetField) {
      throw new Error(`单变量场景除目标字段外上下文漂移: ${scenario.field}`);
    }
  }
  if (new Set(scenario.states.map(({ options }) => options[targetField])).size !== scenario.states.length) {
    throw new Error(`单变量场景目标档位必须互不相同: ${scenario.field}`);
  }
}

function assertScenarioContract({
  defaults,
  parameterByName,
  productScenarios,
  runtimeScenarios,
  runtimeScenario,
  isolationScenarios,
  combinedActiveOptions,
}) {
  assertExactFieldSet('产品场景', productScenarios.map(({ field }) => field), PRODUCT_PARAMETER_NAMES);
  assertExactFieldSet('runtime 独立场景', runtimeScenarios.map(({ field }) => field), RUNTIME_OPTION_NAMES);
  const logicalScenarios = [...productScenarios, ...runtimeScenarios, runtimeScenario];
  if (logicalScenarios.length !== LOGICAL_SCENARIO_COUNT
    || new Set(logicalScenarios.map(({ field }) => field)).size !== LOGICAL_SCENARIO_COUNT) {
    throw new Error(`逻辑场景必须精确且唯一: ${LOGICAL_SCENARIO_COUNT}`);
  }
  if (isolationScenarios.length !== ISOLATION_SCENARIO_COUNT
    || isolationScenarios[0]?.field !== 'runtime_random_seed_pip_isolation'
    || logicalScenarios.some(({ field }) => field === isolationScenarios[0].field)) {
    throw new Error(`隔离场景必须精确且唯一: ${ISOLATION_SCENARIO_COUNT}`);
  }

  for (const scenario of productScenarios) {
    const expectedRoles = scenario.mode === 'product_switch'
      ? 'baseline,active'
      : 'baseline,low,active';
    if (!['product_progression', 'product_response', 'product_switch'].includes(scenario.mode)
      || scenario.states.map(({ role }) => role).join(',') !== expectedRoles) {
      throw new Error(`产品场景档位无效: ${scenario.field}`);
    }
    for (const { role, options } of scenario.states) {
      assertBoundedSnapshot(`${scenario.field}/${role}`, options, parameterByName);
    }
    assertScenarioOnlyChangesTarget(scenario);
    if (scenario.baselineOptions[scenario.field] !== defaults[scenario.field]) {
      throw new Error(`产品场景 baseline 目标字段必须保持默认值: ${scenario.field}`);
    }
    if (scenario.activeOptions[scenario.field] === defaults[scenario.field]
      || (scenario.mode !== 'product_switch'
        && scenario.lowOptions[scenario.field] === defaults[scenario.field])) {
      throw new Error(`产品场景必须启用目标字段: ${scenario.field}`);
    }
    for (const gate of scenario.gateFields) {
      if (scenario.states.some(({ options }) => options[gate] === defaults[gate])) {
        throw new Error(`产品场景必须打开依赖门: ${scenario.field}/${gate}`);
      }
    }
  }

  for (const scenario of [...runtimeScenarios, runtimeScenario]) {
    if (scenario.mode !== 'pair_difference'
      || scenario.states.length !== 2
      || scenario.states.map(({ role }) => role).join(',') !== 'neutral,active') {
      throw new Error(`runtime 场景必须是 neutral/active 两档: ${scenario.field}`);
    }
    for (const { role, options } of scenario.states) {
      assertBoundedSnapshot(`${scenario.field}/${role}`, options, parameterByName);
    }
    if (scenario !== runtimeScenario) assertScenarioOnlyChangesTarget(scenario);
  }

  const isolation = isolationScenarios[0];
  if (isolation.mode !== 'expected_same'
    || isolation.states.length !== 2
    || isolation.states.map(({ role }) => role).join(',') !== 'seed_zero,seed_changed') {
    throw new Error('seed/PIP 隔离场景必须是两档 expected_same');
  }
  for (const { role, options } of isolation.states) {
    assertBoundedSnapshot(`${isolation.field}/${role}`, options, parameterByName);
    if (options.al_pip_enabled !== 1 || options.al_random_graphic_enabled !== 0) {
      throw new Error('seed/PIP 隔离场景必须开启 PIP 并关闭随机图形');
    }
  }
  assertScenarioOnlyChangesTarget(isolation, 'al_runtime_random_graphic_seed');

  assertBoundedSnapshot('combinedActiveOptions', combinedActiveOptions, parameterByName);
  for (const name of [...PRODUCT_PARAMETER_NAMES, ...RUNTIME_OPTION_NAMES]) {
    if (combinedActiveOptions[name] === defaults[name]) {
      throw new Error(`combinedActiveOptions 未启用固定效果: ${name}`);
    }
  }
  if (runtimeScenario.activeOptions !== combinedActiveOptions) {
    throw new Error('全激活组合场景必须复用唯一 combinedActiveOptions');
  }
}

export function buildPhase3aCandidateContract(entries) {
  if (!Array.isArray(entries)) throw new TypeError('候选内容必须以数组传入');
  const sources = new Map();
  for (const entry of entries) {
    if (!entry || typeof entry !== 'object') throw new TypeError('候选条目必须是对象');
    const path = canonicalCandidatePath(entry.path);
    if (sources.has(path)) throw new Error(`重复候选文件: ${path}`);
    if (typeof entry.source !== 'string') throw new TypeError(`${path} 内容必须是字符串`);
    sources.set(path, entry.source);
  }
  const missing = CANDIDATE_PATHS.filter((path) => !sources.has(path));
  if (missing.length > 0) throw new Error(`缺少候选文件: ${missing.join(', ')}`);
  if (sources.size !== CANDIDATE_PATHS.length) throw new Error('候选文件数量不匹配');

  const seen = new Set();
  const files = CANDIDATE_PATHS.map((path) => {
    const source = sources.get(path);
    const parameters = parseCandidateParameters(source, path);
    const filename = path.slice(path.lastIndexOf('/') + 1);
    const parameterNames = parameters.map(({ name }) => name);
    for (const { name } of parameters) {
      if (seen.has(name)) throw new Error(`重复 al_ 参数: ${name}`);
      seen.add(name);
    }
    const expectedParameterNames = EXPECTED_PARAMETERS_BY_FILENAME[filename];
    if (parameterNames.length !== expectedParameterNames.length
      || parameterNames.some((name, index) => name !== expectedParameterNames[index])) {
      throw new Error(`${path} 的 al_ 参数清单或顺序与固定契约不一致`);
    }
    return {
      path,
      filename,
      sha256: hash(source),
      parameterNames,
      parameters,
      source,
      productionSource: productionShaderSource(filename, source),
    };
  });

  const baselineCount = files[0].parameters.length;
  const snapshotCount = files.reduce((sum, file) => sum + file.parameters.length, 0);
  const runtimeCount = files.reduce(
    (sum, file) => sum + file.parameters.filter(({ name }) => RUNTIME_OPTION_NAME_SET.has(name)).length,
    0,
  );
  const candidateCount = snapshotCount - runtimeCount;
  const additionalCount = candidateCount - baselineCount;
  if (baselineCount !== BASELINE_PARAMETER_COUNT
    || additionalCount !== ADDITIONAL_PARAMETER_COUNT
    || candidateCount !== TOTAL_PARAMETER_COUNT
    || runtimeCount !== RUNTIME_OPTION_COUNT
    || snapshotCount !== FULL_SNAPSHOT_OPTION_COUNT) {
    throw new Error(`候选选项数量不匹配: baseline=${baselineCount}, additional=${additionalCount}, runtime=${runtimeCount}, snapshot=${snapshotCount}`);
  }

  const parameters = files.flatMap(({ path, parameters: fileParameters }) =>
    fileParameters.map((parameter) => ({ ...parameter, path })));
  const defaults = Object.fromEntries(parameters.map(({ name, defaultValue }) => [name, defaultValue]));
  for (const name of RUNTIME_OPTION_NAMES) {
    if (defaults[name] !== RUNTIME_NEUTRAL_OPTIONS[name]) {
      throw new Error(`runtime 选项默认值必须为中性 0: ${name}`);
    }
  }
  const parameterByName = new Map(parameters.map((parameter) => [parameter.name, parameter]));
  for (const [name, value] of Object.entries(RUNTIME_ACTIVE_OPTIONS)) {
    const parameter = parameterByName.get(name);
    if (!Number.isFinite(value) || value < parameter.minimum || value > parameter.maximum) {
      throw new Error(`runtime 明显场景固定值越界: ${name}=${value}`);
    }
  }
  const shaderText = files.map(({ path, source }) =>
    `// BEGIN ${path}\n${source.trimEnd()}\n// END ${path}`).join('\n\n') + '\n';
  const productionShaderText = files.map(({ path, productionSource }) =>
    `// BEGIN ${path}\n${productionSource.trimEnd()}\n// END ${path}`).join('\n\n') + '\n';
  const manifest = files.map(({ path, filename, sha256, parameterNames }) => ({
    path,
    filename,
    sha256,
    parameterNames,
  }));

  const productScenarios = buildFrameDifferenceScenarios(defaults);
  const runtimeScenarios = buildRuntimeSchedulerScenarios(defaults);
  const combinedActiveOptions = buildCombinedActiveOptions(defaults);
  const runtimeScenario = buildRuntimeSchedulerScenario(defaults, combinedActiveOptions);
  const isolationScenarios = [buildRandomSeedPipIsolationScenario(defaults)];
  assertScenarioContract({
    defaults,
    parameterByName,
    productScenarios,
    runtimeScenarios,
    runtimeScenario,
    isolationScenarios,
    combinedActiveOptions,
  });
  const logicalScenarios = [...productScenarios, ...runtimeScenarios, runtimeScenario];
  return {
    shaderText,
    sha256: hash(shaderText),
    productionShaderText,
    productionSha256: hash(productionShaderText),
    manifest,
    parameters,
    defaults,
    counts: {
      baseline: baselineCount,
      additional: additionalCount,
      runtime: runtimeCount,
      total: candidateCount,
      snapshot: snapshotCount,
      logicalScenarios: logicalScenarios.length,
      isolationScenarios: isolationScenarios.length,
    },
    productScenarios,
    runtimeScenarios,
    runtimeScenario,
    logicalScenarios,
    isolationScenarios,
    combinedActiveOptions,
    scenarios: [...logicalScenarios, ...isolationScenarios],
  };
}
