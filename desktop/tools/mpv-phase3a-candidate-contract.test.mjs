import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import {
  ADDITIONAL_PARAMETER_COUNT,
  ADDITIONAL_PARAMETER_NAMES,
  BASELINE_PARAMETER_COUNT,
  BASELINE_PARAMETER_NAMES,
  CANDIDATE_PATHS,
  FULL_SNAPSHOT_OPTION_COUNT,
  ISOLATION_SCENARIO_COUNT,
  LOGICAL_SCENARIO_COUNT,
  PRODUCT_PARAMETER_NAMES,
  RUNTIME_ACTIVE_OPTIONS,
  RUNTIME_NEUTRAL_OPTIONS,
  RUNTIME_OPTION_COUNT,
  RUNTIME_OPTION_NAMES,
  TOTAL_PARAMETER_COUNT,
  assertScenarioOnlyChangesTarget,
  buildFrameDifferenceScenarios,
  buildPhase3aCandidateContract,
  parseCandidateParameters,
  parseRustGpu83Mappings,
} from './mpv-phase3a-candidate-contract.mjs';
import { capturePausedScenarioFrames } from './mpv-frame-difference.mjs';

const SCHEDULER_FILE_OPTIONS = Object.freeze([
  'al_runtime_frame_inner_active',
  'al_runtime_frame_inter_active',
  'al_runtime_frame_probability_active',
  'al_runtime_slice_active',
  'al_runtime_highlight_active',
  'al_runtime_async_rotation_degrees',
  'al_runtime_transform_easing',
]);

async function candidateEntries() {
  return Promise.all(CANDIDATE_PATHS.map(async (path) => ({
    path,
    source: await readFile(new URL(`./shader-candidates/${path.split('/').at(-1)}`, import.meta.url), 'utf8'),
  })));
}

function parseRustGpu83FieldPaths(source) {
  const body = /pub const GPU83_FIELD_PATHS:[\s\S]*?=\s*\[([\s\S]*?)\];/.exec(source)?.[1];
  assert.ok(body, 'Rust GPU83_FIELD_PATHS 常量缺失');
  return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

test('固定九文件按确定顺序聚合为 61 项候选语义与 11 项 runtime 选项', async () => {
  const entries = await candidateEntries();
  const contract = buildPhase3aCandidateContract([...entries].reverse());

  assert.equal(contract.manifest.length, 9);
  assert.deepEqual(contract.manifest.map(({ path }) => path), CANDIDATE_PATHS);
  assert.deepEqual(contract.counts, {
    baseline: BASELINE_PARAMETER_COUNT,
    additional: ADDITIONAL_PARAMETER_COUNT,
    runtime: RUNTIME_OPTION_COUNT,
    total: TOTAL_PARAMETER_COUNT,
    snapshot: FULL_SNAPSHOT_OPTION_COUNT,
    logicalScenarios: LOGICAL_SCENARIO_COUNT,
    isolationScenarios: ISOLATION_SCENARIO_COUNT,
  });
  assert.equal(contract.parameters.length, 72);
  assert.equal(Object.keys(contract.defaults).length, 72);
  assert.deepEqual(
    contract.manifest.slice(1)
      .flatMap(({ parameterNames }) => parameterNames)
      .filter((name) => !RUNTIME_OPTION_NAMES.includes(name)),
    ADDITIONAL_PARAMETER_NAMES,
  );
  const runtimeByFile = Object.fromEntries(contract.manifest.map(({ filename, parameterNames }) => [
    filename,
    parameterNames.filter((name) => RUNTIME_OPTION_NAMES.includes(name)),
  ]));
  assert.deepEqual(runtimeByFile['gpu83-local-blur-candidate.hook'], ['al_runtime_local_blur_active']);
  assert.deepEqual(runtimeByFile['gpu83-same-frame-overlays-candidate.hook'], [
    'al_runtime_pip_jitter_x_px',
    'al_runtime_pip_jitter_y_px',
    'al_runtime_random_graphic_seed',
  ]);
  assert.deepEqual(runtimeByFile['gpu83-scheduler-gates-candidate.hook'], SCHEDULER_FILE_OPTIONS);
  assert.deepEqual(
    contract.manifest.flatMap(({ parameterNames }) => parameterNames)
      .filter((name) => RUNTIME_OPTION_NAMES.includes(name)).sort(),
    [...RUNTIME_OPTION_NAMES].sort(),
  );
  assert.match(contract.sha256, /^[a-f0-9]{64}$/);
  assert.ok(contract.manifest.every(({ sha256 }) => /^[a-f0-9]{64}$/.test(sha256)));

  let previous = -1;
  for (const path of CANDIDATE_PATHS) {
    const position = contract.shaderText.indexOf(`// BEGIN ${path}`);
    assert.ok(position > previous, `${path} 聚合顺序错误`);
    previous = position;
  }
  assert.equal(buildPhase3aCandidateContract(entries).sha256, contract.sha256);
});

test('v6 通过的 61 项产品参数按九文件顺序原子晋级为唯一生产 shader', async () => {
  const [entries, rustSource, productionShader] = await Promise.all([
    candidateEntries(),
    readFile(new URL('../src-tauri/src/media_video_gpu_effects.rs', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url), 'utf8'),
  ]);
  const contract = buildPhase3aCandidateContract(entries);
  const mappings = parseRustGpu83Mappings(rustSource);
  const fieldPaths = parseRustGpu83FieldPaths(rustSource);
  const mappingByOption = new Map(mappings.map((mapping) => [mapping.shaderOption, mapping]));

  assert.equal(mappings.length, 83);
  assert.equal(new Set(mappings.map(({ fieldPath }) => fieldPath)).size, 83);
  assert.equal(new Set(mappings.map(({ shaderOption }) => shaderOption)).size, 83);
  assert.deepEqual(
    [...mappings.map(({ fieldPath }) => fieldPath)].sort(),
    [...fieldPaths].sort(),
    'Rust mapping field_path 必须精确覆盖 GPU83_FIELD_PATHS',
  );
  assert.deepEqual(
    Object.fromEntries(['AVAILABLE', 'UNVERIFIED', 'SCHEDULER', 'HISTORY'].map((capability) => [
      capability,
      mappings.filter((mapping) => mapping.capability === capability).length,
    ])),
    { AVAILABLE: 61, UNVERIFIED: 2, SCHEDULER: 18, HISTORY: 2 },
  );
  assert.deepEqual(
    mappings.filter(({ capability }) => capability === 'UNVERIFIED').map(({ fieldPath }) => fieldPath),
    [
      'video.color_space_conversion_strength_percent',
      'video.color_space_conversion_enabled',
    ],
  );
  assert.deepEqual(
    mappings.filter(({ capability }) => capability === 'HISTORY').map(({ fieldPath }) => fieldPath),
    ['advanced.slice_min_length_ms', 'advanced.picture_in_picture_timeline_locked'],
  );

  const candidateMappings = PRODUCT_PARAMETER_NAMES.map((shaderOption) => {
    const mapping = mappingByOption.get(shaderOption);
    assert.ok(mapping, `候选参数缺少 Rust GPU83 映射: ${shaderOption}`);
    return mapping;
  });
  assert.equal(new Set(candidateMappings.map(({ fieldPath }) => fieldPath)).size, TOTAL_PARAMETER_COUNT);
  assert.deepEqual(
    Object.fromEntries(['AVAILABLE', 'UNVERIFIED', 'SCHEDULER', 'HISTORY'].map((capability) => [
      capability,
      candidateMappings.filter((mapping) => mapping.capability === capability).length,
    ])),
    { AVAILABLE: 61, UNVERIFIED: 0, SCHEDULER: 0, HISTORY: 0 },
  );

  const productionOptions = parseCandidateParameters(
    productionShader,
    'desktop/src-tauri/resources/shaders/gpu83.hook',
  ).map(({ name }) => name);
  for (const blockedOption of [
    'al_color_space_strength_percent',
    'al_color_space_enabled',
    'al_slice_min_length_ms',
    'al_pip_timeline_locked',
  ]) {
    assert.ok(!productionOptions.includes(blockedOption),
      `未准入字段不得进入生产 shader 快照: ${blockedOption}`);
  }
  assert.equal(contract.sha256, '64049f6ae835899e6bc9ec4fcac4b6c80536f0d7cb073ebb48f09e5111e10207');
  assert.equal(contract.productionSha256, '3e08e25507a063d405807184f4eaf51e5da77e03df72acba322e4a44e01b6f9f');
  assert.match(contract.shaderText, /^\/\/!DESC .*candidate$/m);
  assert.equal(productionShader, contract.productionShaderText);
  assert.doesNotMatch(productionShader, /^\/\/!DESC .*(?:candidate|Phase 3A).*$/m);
  assert.deepEqual(productionOptions, contract.parameters.map(({ name }) => name));
  assert.deepEqual(
    mappings.filter(({ capability }) => capability === 'AVAILABLE').map(({ shaderOption }) => shaderOption).sort(),
    productionOptions.filter((name) => !RUNTIME_OPTION_NAMES.includes(name)).sort(),
  );
  assert.deepEqual(
    contract.parameters
      .map(({ name }) => name)
      .filter((name) => !RUNTIME_OPTION_NAMES.includes(name)),
    PRODUCT_PARAMETER_NAMES,
  );
});

test('scheduler-gates 候选只声明 7 项本地 runtime 参数并保持有界同帧语义', async () => {
  const source = await readFile(
    new URL('./shader-candidates/gpu83-scheduler-gates-candidate.hook', import.meta.url),
    'utf8',
  );
  assert.deepEqual(parseCandidateParameters(source), [
    { name: 'al_runtime_frame_inner_active', minimum: 0, maximum: 1, defaultValue: 0 },
    { name: 'al_runtime_frame_inter_active', minimum: 0, maximum: 1, defaultValue: 0 },
    { name: 'al_runtime_frame_probability_active', minimum: 0, maximum: 1, defaultValue: 0 },
    { name: 'al_runtime_slice_active', minimum: 0, maximum: 1, defaultValue: 0 },
    { name: 'al_runtime_highlight_active', minimum: 0, maximum: 1, defaultValue: 0 },
    { name: 'al_runtime_async_rotation_degrees', minimum: -15, maximum: 15, defaultValue: 0 },
    { name: 'al_runtime_transform_easing', minimum: 0, maximum: 1, defaultValue: 0 },
  ]);
  assert.doesNotMatch(source, /^\/\/!DEFAULT\b/m);
  assert.match(source, /^\/\/!HOOK MAIN\s*$/m);
  assert.match(source, /^\/\/!BIND HOOKED\s*$/m);
  assert.match(source, /^\/\/!WHEN 1\s*$/m);
  assert.doesNotMatch(source, /^\/\/!(?:SAVE|COMPUTE)\b/m);

  const code = source.replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
  assert.doesNotMatch(code, /\b(?:PTS|TIME|clock|history|audio)\b/i);
  assert.doesNotMatch(code, /\b(?:for|while)\s*\(/);
  assert.doesNotMatch(code, /\b(?:fract|al_scheduler_hash)\b/,
    '周期门控不得在 fragment shader 内自行生成随机数');
  assert.equal([...code.matchAll(/\bHOOKED_tex\s*\(/g)].length, 3,
    '成本必须固定为原像素、同帧变换和同帧邻点最多三次采样');
  assert.match(code, /\binter_mix\s*=\s*inter_gate\s*\*\s*0\.2\s*;/);
  assert.match(code, /\bprobability_strength\s*=\s*probability_gate\s*;/);
  assert.match(code, /\binner_delta\s*=\s*inner_gate\s*\*/);
  assert.match(code, /\btransform_weight\s*=\s*slice_gate\s*\*\s*easing\s*;/);
  assert.match(code, /\bslice_shift_uv\s*=\s*2\.0\s*\*\s*HOOKED_pt\.x\s*\*\s*transform_weight\s*;/);
  assert.match(code, /\bangle\s*=\s*radians\s*\(\s*angle_degrees\s*\)\s*\*\s*transform_weight\s*;/);
  assert.match(code, /\bhighlight_delta\s*=\s*highlight_gate\s*\/\s*255\.0\s*;/);
  assert.match(code, /\bif\s*\(\s*!has_luma_effect\s*&&\s*inter_mix\s*<=\s*0\.0\s*&&\s*!has_transform\s*\)\s*\{\s*return\s+source_color\s*;/s,
    '7 项 runtime 全中性时必须在额外采样前返回原像素');
  assert.match(code, /\breturn\s+vec4\s*\(\s*result\s*,\s*source_color\.a\s*\)\s*;/);

  const allowedAlNames = new Set([...SCHEDULER_FILE_OPTIONS, 'al_scheduler_rotate']);
  const referencedAlNames = new Set(code.match(/\bal_[A-Za-z0-9_]+\b/g) ?? []);
  assert.deepEqual([...referencedAlNames].filter((name) => !allowedAlNames.has(name)), [],
    'scheduler 候选不得跨文件引用未声明产品参数');
});

test('61 个产品字段场景均提供唯一、有限且合法的完整快照', async () => {
  const contract = buildPhase3aCandidateContract(await candidateEntries());
  const scenarios = buildFrameDifferenceScenarios(contract.defaults);
  assert.equal(scenarios.length, 61);
  assert.equal(new Set(BASELINE_PARAMETER_NAMES).size, 20);
  assert.equal(new Set(ADDITIONAL_PARAMETER_NAMES).size, 41);
  assert.equal(new Set(PRODUCT_PARAMETER_NAMES).size, 61);
  assert.deepEqual(
    [...scenarios.map(({ field }) => field)].sort(),
    [...PRODUCT_PARAMETER_NAMES].sort(),
  );

  const expectedKeys = Object.keys(contract.defaults).sort();
  for (const scenario of scenarios) {
    const { field, gateFields, baselineOptions, lowOptions, activeOptions } = scenario;
    assert.ok(['product_progression', 'product_response', 'product_switch'].includes(scenario.mode));
    assert.deepEqual(
      scenario.states.map(({ role }) => role),
      scenario.mode === 'product_switch' ? ['baseline', 'active'] : ['baseline', 'low', 'active'],
    );
    assert.deepEqual(Object.keys(baselineOptions).sort(), expectedKeys, `${field} baseline 不是完整快照`);
    assert.deepEqual(Object.keys(activeOptions).sort(), expectedKeys, `${field} active 不是完整快照`);
    assert.equal(baselineOptions[field], contract.defaults[field], `${field} baseline 目标字段不是默认值`);
    assert.notEqual(activeOptions[field], contract.defaults[field], `${field} active 未启用目标字段`);
    assert.ok(Object.values(activeOptions).every(Number.isFinite));
    assert.doesNotThrow(() => assertScenarioOnlyChangesTarget(scenario));
    for (const name of expectedKeys.filter((name) => name !== field)) {
      assert.equal(activeOptions[name], baselineOptions[name], `${field} active 上下文漂移: ${name}`);
    }
    if (scenario.mode !== 'product_switch') {
      assert.deepEqual(Object.keys(lowOptions).sort(), expectedKeys, `${field} low 不是完整快照`);
      assert.notEqual(lowOptions[field], contract.defaults[field], `${field} low 未启用目标字段`);
      assert.notEqual(lowOptions[field], activeOptions[field], `${field} low/active 目标档位相同`);
      assert.ok(Object.values(lowOptions).every(Number.isFinite));
      for (const name of expectedKeys.filter((name) => name !== field)) {
        assert.equal(lowOptions[name], baselineOptions[name], `${field} low 上下文漂移: ${name}`);
      }
    }
    for (const gate of gateFields) {
      for (const { role, options } of scenario.states) {
        assert.notEqual(options[gate], contract.defaults[gate], `${field}/${role} 未打开依赖门 ${gate}`);
      }
    }
  }

  const byField = new Map(scenarios.map((item) => [item.field, item]));
  for (const field of ['al_red_lock_enabled', 'al_image_repair_enabled', 'al_random_graphic_enabled',
    'al_pip_enabled', 'al_local_blur_enabled', 'al_edge_fill_enabled']) {
    assert.equal(byField.get(field).mode, 'product_switch', `${field} 必须使用二态开关门禁`);
    assert.equal(byField.get(field).activeOptions[field], 1, `${field} active 必须是真实布尔启用值`);
  }
  for (const field of ['al_abstract_face_count', 'al_random_graphic_count', 'al_wave_grain_count',
    'al_space_dimension']) {
    assert.ok(Number.isInteger(byField.get(field).lowOptions[field]), `${field} low 必须是离散整数`);
  }
  assert.equal(byField.get('al_image_repair_strength_percent').activeOptions.al_image_repair_enabled, 1);
  assert.equal(byField.get('al_image_repair_enabled').activeOptions.al_image_repair_strength_percent, 100);
  assert.equal(byField.get('al_image_repair_strength_percent').activeOptions.al_image_repair_strength_percent, 100);
  assert.ok(byField.get('al_abstract_face_size_percent').activeOptions.al_abstract_face_count > 0);
  assert.ok(byField.get('al_abstract_face_size_percent').activeOptions.al_abstract_face_opacity_percent > 0);
  assert.equal(byField.get('al_random_graphic_count').activeOptions.al_random_graphic_enabled, 1);
  assert.equal(byField.get('al_random_graphic_count').activeOptions.al_random_graphic_opacity_percent, 50);
  assert.equal(byField.get('al_pip_scale_percent').activeOptions.al_pip_enabled, 1);
  assert.equal(byField.get('al_pip_rotation_degrees').activeOptions.al_pip_opacity_percent, 80);
  assert.equal(byField.get('al_local_blur_radius_px').activeOptions.al_local_blur_enabled, 1);
  for (const field of ['al_local_blur_enabled', 'al_local_blur_region_percent', 'al_local_blur_radius_px']) {
    assert.equal(byField.get(field).activeOptions.al_runtime_local_blur_active, 1);
    assert.ok(byField.get(field).gateFields.includes('al_runtime_local_blur_active'));
  }
  assert.equal(byField.get('al_edge_feather_percent').activeOptions.al_edge_fill_enabled, 1);
  assert.ok(byField.get('al_core_frequency_hz').activeOptions.al_target_frequency_hz >= 65);
  assert.ok(byField.get('al_wave_grain_count').activeOptions.al_wave_intensity > 0);
  assert.notEqual(byField.get('al_dynamic_eq_threshold').activeOptions.al_band_65, 1);
  assert.ok(byField.get('al_frequency_space_x_px').activeOptions.al_wave_intensity > 0);

  for (const field of ['al_target_frequency_hz', 'al_core_frequency_hz', 'al_wave_grain_count',
    'al_dynamic_eq_threshold', 'al_space_dimension']) {
    assert.equal(byField.get(field).mode, 'product_response', `${field} 不得按强度单调性判定`);
  }
  assert.equal(byField.get('al_pip_opacity_percent').lowOptions.al_pip_opacity_percent, 90);
  assert.equal(byField.get('al_pip_opacity_percent').activeOptions.al_pip_opacity_percent, 50);
  assert.equal(byField.get('al_local_blur_radius_px').lowOptions.al_local_blur_radius_px, 3);
  assert.equal(byField.get('al_core_frequency_hz').lowOptions.al_core_frequency_hz, 500);
  assert.equal(byField.get('al_core_frequency_hz').activeOptions.al_core_frequency_hz, 65);
  assert.equal(byField.get('al_dynamic_eq_threshold').lowOptions.al_dynamic_eq_threshold, 2);
  assert.equal(byField.get('al_abstract_face_count').lowOptions.al_abstract_face_opacity_percent, 20);
  assert.equal(byField.get('al_abstract_face_size_percent').lowOptions.al_abstract_face_opacity_percent, 20);
  assert.equal(byField.get('al_abstract_face_opacity_percent').lowOptions.al_abstract_face_size_percent, 4);
  assert.equal(byField.get('al_random_graphic_count').lowOptions.al_random_graphic_opacity_percent, 50);
  assert.equal(byField.get('al_random_graphic_size_px').lowOptions.al_random_graphic_size_px, 8);
  assert.equal(byField.get('al_overlay_offset_px').lowOptions.al_random_graphic_size_px, 32);
  assert.equal(byField.get('al_overlay_offset_px').mode, 'product_response',
    '二维位移改变差异位置，不得按差异幅度单调性判定');
  assert.equal(byField.get('al_space_dimension').lowOptions.al_space_dimension, 1);
  assert.equal(byField.get('al_space_dimension').activeOptions.al_space_dimension, 3);

  const invalidDefaults = { ...contract.defaults };
  delete invalidDefaults.al_brightness_percent;
  invalidDefaults.al_unexpected_baseline_parameter = 0;
  assert.throws(
    () => buildFrameDifferenceScenarios(invalidDefaults),
    /精确包含固定的 72 个有限 al_ 选项/,
  );
});

test('产品场景任何非目标字段漂移都 fail-closed', async () => {
  const contract = buildPhase3aCandidateContract(await candidateEntries());
  const scenario = structuredClone(contract.productScenarios.find(({ field }) =>
    field === 'al_pip_scale_percent'));
  scenario.states[2].options.al_pip_opacity_percent = 79;
  assert.throws(
    () => assertScenarioOnlyChangesTarget(scenario),
    /单变量场景除目标字段外上下文漂移: al_pip_scale_percent/,
  );

  const duplicateTargetLevel = structuredClone(contract.productScenarios.find(({ field }) =>
    field === 'al_space_dimension'));
  duplicateTargetLevel.states[1].options.al_space_dimension =
    duplicateTargetLevel.states[2].options.al_space_dimension;
  assert.throws(
    () => assertScenarioOnlyChangesTarget(duplicateTargetLevel),
    /单变量场景目标档位必须互不相同: al_space_dimension/,
  );
});

test('runtime 默认全中性且明显场景使用固定有限值', async () => {
  const contract = buildPhase3aCandidateContract(await candidateEntries());
  const defaultsBeforeScenarioBuild = { ...contract.defaults };
  assert.equal(new Set(RUNTIME_OPTION_NAMES).size, RUNTIME_OPTION_COUNT);
  assert.deepEqual(
    Object.fromEntries(RUNTIME_OPTION_NAMES.map((name) => [name, contract.defaults[name]])),
    RUNTIME_NEUTRAL_OPTIONS,
  );
  assert.deepEqual(
    Object.fromEntries(RUNTIME_OPTION_NAMES.map((name) => [name, contract.runtimeScenario.activeOptions[name]])),
    RUNTIME_ACTIVE_OPTIONS,
  );
  assert.equal(Object.keys(contract.runtimeScenario.neutralOptions).length, FULL_SNAPSHOT_OPTION_COUNT);
  assert.equal(Object.keys(contract.runtimeScenario.activeOptions).length, FULL_SNAPSHOT_OPTION_COUNT);
  assert.ok(Object.values(contract.runtimeScenario.neutralOptions).every(Number.isFinite));
  assert.ok(Object.values(contract.runtimeScenario.activeOptions).every(Number.isFinite));
  assert.equal(contract.runtimeScenario.neutralOptions.al_local_blur_enabled, contract.defaults.al_local_blur_enabled);
  assert.equal(contract.runtimeScenario.neutralOptions.al_runtime_local_blur_active, 0);
  assert.equal(contract.runtimeScenario.activeOptions.al_runtime_local_blur_active, 1);
  assert.equal(contract.runtimeScenario.activeOptions, contract.combinedActiveOptions);
  assert.deepEqual(contract.defaults, defaultsBeforeScenarioBuild);
  assert.deepEqual(
    contract.productScenarios.map(({ field }) => field),
    PRODUCT_PARAMETER_NAMES,
  );
  assert.equal(contract.logicalScenarios.at(-1), contract.runtimeScenario);
  assert.equal(contract.runtimeScenarios.length, RUNTIME_OPTION_COUNT);
  assert.deepEqual(contract.runtimeScenarios.map(({ field }) => field), RUNTIME_OPTION_NAMES);
  for (const scenario of contract.runtimeScenarios) {
    assert.equal(scenario.neutralOptions[scenario.field], 0);
    assert.equal(scenario.activeOptions[scenario.field], RUNTIME_ACTIVE_OPTIONS[scenario.field]);
    assert.deepEqual(Object.keys(scenario.neutralOptions).sort(), Object.keys(contract.defaults).sort());
    assert.deepEqual(Object.keys(scenario.activeOptions).sort(), Object.keys(contract.defaults).sort());
    for (const gate of scenario.gateFields) {
      assert.notEqual(scenario.activeOptions[gate], contract.defaults[gate],
        `${scenario.field} 未固定打开依赖 ${gate}`);
    }
  }
  assert.equal(
    contract.logicalScenarios.length,
    TOTAL_PARAMETER_COUNT + RUNTIME_OPTION_COUNT + 1,
  );
  assert.equal(contract.logicalScenarios.length, LOGICAL_SCENARIO_COUNT);
  assert.equal(contract.scenarios.length, LOGICAL_SCENARIO_COUNT + ISOLATION_SCENARIO_COUNT);
  for (const name of [...PRODUCT_PARAMETER_NAMES, ...RUNTIME_OPTION_NAMES]) {
    assert.notEqual(contract.combinedActiveOptions[name], contract.defaults[name], `${name} 未进入组合负载`);
    assert.equal(contract.runtimeScenario.activeOptions[name], contract.combinedActiveOptions[name]);
  }
  const easing = contract.runtimeScenarios.find(({ field }) => field === 'al_runtime_transform_easing');
  assert.equal(easing.activeOptions.al_runtime_slice_active, 1);
  const slice = contract.runtimeScenarios.find(({ field }) => field === 'al_runtime_slice_active');
  assert.equal(slice.activeOptions.al_runtime_transform_easing, 1);
  const rotation = contract.runtimeScenarios.find(({ field }) =>
    field === 'al_runtime_async_rotation_degrees');
  assert.equal(rotation.activeOptions.al_runtime_slice_active, 1);
  assert.equal(rotation.activeOptions.al_runtime_transform_easing, 1);
  const localBlur = contract.runtimeScenarios.find(({ field }) =>
    field === 'al_runtime_local_blur_active');
  assert.equal(localBlur.activeOptions.al_local_blur_enabled, 1);
  assert.equal(localBlur.activeOptions.al_local_blur_region_percent, 30);
  assert.equal(localBlur.activeOptions.al_local_blur_radius_px, 8);
  for (const field of ['al_runtime_pip_jitter_x_px', 'al_runtime_pip_jitter_y_px']) {
    const scenario = contract.runtimeScenarios.find((item) => item.field === field);
    assert.equal(scenario.activeOptions.al_pip_enabled, 1);
    assert.equal(scenario.activeOptions.al_pip_opacity_percent, 80);
  }
  const randomSeed = contract.runtimeScenarios.find(({ field }) =>
    field === 'al_runtime_random_graphic_seed');
  assert.equal(randomSeed.neutralOptions.al_runtime_random_graphic_seed, 0);
  assert.equal(randomSeed.activeOptions.al_runtime_random_graphic_seed, 1193046);
  assert.ok(Number.isFinite(randomSeed.activeOptions.al_runtime_random_graphic_seed));
  assert.ok(Number.isInteger(randomSeed.activeOptions.al_runtime_random_graphic_seed));
  assert.equal(randomSeed.activeOptions.al_random_graphic_enabled, 1);
  assert.equal(randomSeed.activeOptions.al_random_graphic_opacity_percent, 50);

  const [isolation] = contract.isolationScenarios;
  assert.equal(isolation.mode, 'expected_same');
  assert.deepEqual(isolation.states.map(({ role }) => role), ['seed_zero', 'seed_changed']);
  assert.ok(isolation.states.every(({ options }) => options.al_pip_enabled === 1));
  assert.ok(isolation.states.every(({ options }) => options.al_random_graphic_enabled === 0));
  const changed = Object.keys(isolation.states[0].options)
    .filter((name) => isolation.states[0].options[name] !== isolation.states[1].options[name]);
  assert.deepEqual(changed, ['al_runtime_random_graphic_seed']);
});

test('假 mpv 按 55 个三档和 6 个两档产品场景、12 个两档 runtime 场景及隔离场景捕获证据', async () => {
  const contract = buildPhase3aCandidateContract(await candidateEntries());
  const commands = [];
  const ipc = {
    async send(command) {
      commands.push(command);
      if (command[0] !== 'get_property') return { data: null };
      return { data: command[1] === 'pause' ? false : 27.25 };
    },
  };

  const captures = await capturePausedScenarioFrames({
    ipc,
    directory: 'E:/fake-phase3b-captures',
    scenarios: contract.scenarios,
  });

  const snapshots = commands.filter((command) =>
    command[0] === 'set_property' && command[1] === 'glsl-shader-opts');
  const expectedCaptureCount = 55 * 3 + 6 * 2
    + (RUNTIME_OPTION_COUNT + 1 + ISOLATION_SCENARIO_COUNT) * 2;
  assert.equal(expectedCaptureCount, 203);
  assert.equal(captures.length, expectedCaptureCount);
  assert.equal(snapshots.length, expectedCaptureCount);
  for (const [, , serialized] of snapshots) {
    const entries = serialized.split(',');
    assert.equal(entries.length, FULL_SNAPSHOT_OPTION_COUNT);
    assert.equal(new Set(entries.map((entry) => entry.split('=', 1)[0])).size, FULL_SNAPSHOT_OPTION_COUNT);
    assert.ok(entries.every((entry) => Number.isFinite(Number(entry.slice(entry.indexOf('=') + 1)))));
  }
  assert.equal(
    commands.filter((command) => command[0] === 'screenshot-to-file').length,
    expectedCaptureCount,
  );
  assert.equal(
    captures.filter(({ mode }) => ['product_progression', 'product_response'].includes(mode)).length,
    55 * 3,
  );
  assert.equal(captures.filter(({ mode }) => mode === 'product_switch').length, 6 * 2);
  assert.equal(captures.filter(({ mode }) => mode === 'pair_difference').length, 12 * 2);
  assert.equal(captures.filter(({ mode }) => mode === 'expected_same').length, 2);
  assert.ok(captures.every(({ mediaPtsSeconds }) => mediaPtsSeconds === 27.25));
});

test('缺失、额外、重复和越界路径全部 fail-closed', async () => {
  const entries = await candidateEntries();
  assert.throws(() => buildPhase3aCandidateContract(entries.slice(1)), /缺少候选文件/);
  assert.throws(() => buildPhase3aCandidateContract([
    ...entries,
    { path: 'desktop/tools/shader-candidates/extra.hook', source: 'x' },
  ]), /不允许的候选路径/);
  assert.throws(() => buildPhase3aCandidateContract([
    ...entries.slice(0, -1),
    { path: 'desktop/tools/shader-candidates/../gpu83-spatial-modulation-candidate.hook', source: entries.at(-1).source },
  ]), /不允许的候选路径/);
  assert.throws(() => buildPhase3aCandidateContract([
    { ...entries[0], path: entries[0].path.split('/').at(-1) },
    ...entries.slice(1),
  ]), /不允许的候选路径/);
  assert.throws(() => buildPhase3aCandidateContract([
    { ...entries[0], path: `E:/aotlve/${entries[0].path}` },
    ...entries.slice(1),
  ]), /不允许的候选路径/);
  assert.throws(() => buildPhase3aCandidateContract([...entries, entries[0]]), /重复候选文件/);
});

test('跨文件重复 al_ 参数 fail-closed', async () => {
  const entries = await candidateEntries();
  const duplicate = entries.map((entry, index) => index !== 1 ? entry : {
    ...entry,
    source: `//!PARAM al_brightness_percent\n//!TYPE float\n//!MINIMUM 0\n//!MAXIMUM 1\n0\n\n${entry.source}`,
  });
  assert.throws(() => buildPhase3aCandidateContract(duplicate), /重复 al_ 参数: al_brightness_percent/);
});

test('参数总数不变但固定参数被换名仍 fail-closed', async () => {
  const entries = await candidateEntries();
  const renamed = entries.map((entry, index) => index !== 0 ? entry : {
    ...entry,
    source: entry.source.replace('//!PARAM al_brightness_percent', '//!PARAM al_unexpected_baseline_parameter'),
  });
  assert.throws(
    () => buildPhase3aCandidateContract(renamed),
    /al_ 参数清单或顺序与固定契约不一致/,
  );
});

test('候选元数据收窄后，构建阶段会拒绝越界场景而不是只依赖测试', async () => {
  const entries = await candidateEntries();
  const narrowed = entries.map((entry, index) => index !== 0 ? entry : {
    ...entry,
    source: entry.source.replace(
      '//!PARAM al_brightness_percent\n//!TYPE float\n//!MINIMUM -100.0\n//!MAXIMUM 100.0',
      '//!PARAM al_brightness_percent\n//!TYPE float\n//!MINIMUM -100.0\n//!MAXIMUM 0.5',
    ),
  });
  assert.throws(
    () => buildPhase3aCandidateContract(narrowed),
    /al_brightness_percent\/low 参数越界/,
  );
});
