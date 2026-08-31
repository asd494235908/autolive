import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Duplex } from 'node:stream';
import test from 'node:test';

import {
  SAMPLE_SPECS,
  UPDATE_CONTINUITY_IPC_COMMAND_COUNT,
  JsonIpcClient,
  buildDropFrameEvidence,
  buildDropFrameTimelineEvidence,
  buildFrameBudgetEvidence,
  buildUpdatePlaybackTimelineEvidence,
  advanceFrameBudgetObservation,
  observeFrameBudgetSample,
  frameObservationSettleMs,
  sendCpu4CommandWithRetry,
  buildMpvArguments,
  buildShaderOptions,
  classifyAttemptError,
  compileCpu4Update,
  encodeIpcRequest,
  finalReportStatus,
  generateSamples,
  managedPipeName,
  maximumValidatedResolutionClaim,
  parseCliArgs,
  parseIpcResponseLine,
  parseShaderParameters,
  percentile,
  renderTargetMatchesSample,
  requireStableSessionPid,
  shaderCompilationEvidence,
  summarizeLatencies,
  validateUpdateOptionSnapshots,
  validateMpvIdentity,
  verifyShaderContract,
} from './verify-mpv-phase1.mjs';

function fakeSocket(onRequest) {
  return new Duplex({
    read() {},
    write(chunk, _encoding, callback) {
      onRequest?.call(this, chunk);
      callback();
    },
  });
}

const IMAGE_REPAIR_CANDIDATE_PARAMETERS = Object.freeze({
  al_image_repair_enabled: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_image_repair_strength_percent: Object.freeze({ minimum: 0, maximum: 100, defaultValue: 0 }),
});
const ABSTRACT_FACE_CANDIDATE_PARAMETERS = Object.freeze({
  al_abstract_face_count: Object.freeze({ minimum: 0, maximum: 10, defaultValue: 0 }),
  al_abstract_face_size_percent: Object.freeze({ minimum: 1, maximum: 10, defaultValue: 2 }),
  al_abstract_face_opacity_percent: Object.freeze({ minimum: 0, maximum: 30, defaultValue: 0 }),
});
const LOCAL_BLUR_CANDIDATE_PARAMETERS = Object.freeze({
  al_local_blur_enabled: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_runtime_local_blur_active: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_local_blur_region_percent: Object.freeze({ minimum: 5, maximum: 50, defaultValue: 20 }),
  al_local_blur_radius_px: Object.freeze({ minimum: 0.1, maximum: 16, defaultValue: 2 }),
});
const EDGE_FILL_CANDIDATE_PARAMETERS = Object.freeze({
  al_edge_fill_enabled: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_edge_feather_percent: Object.freeze({ minimum: 0, maximum: 100, defaultValue: 0 }),
});
const CHANNEL_OFFSET_CANDIDATE_PARAMETERS = Object.freeze({
  al_channel_offset_percent: Object.freeze({ minimum: -10, maximum: 10, defaultValue: 0 }),
});
const VISUAL_BAND_FREQUENCIES_HZ = Object.freeze([
  65, 92, 131, 188, 267, 381, 544, 777, 1110, 1585, 2263, 20000,
]);
const SPATIAL_MODULATION_CANDIDATE_PARAMETERS = Object.freeze({
  al_target_frequency_hz: Object.freeze({ minimum: 0, maximum: 20000, defaultValue: 0 }),
  al_core_frequency_hz: Object.freeze({ minimum: 0, maximum: 20000, defaultValue: 0 }),
  al_wave_intensity: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_wave_level: Object.freeze({ minimum: 0, maximum: 1, defaultValue: 0 }),
  al_wave_grain_count: Object.freeze({ minimum: 1, maximum: 100, defaultValue: 20 }),
  al_dynamic_eq_threshold: Object.freeze({ minimum: 0, maximum: 20, defaultValue: 10 }),
  al_space_dimension: Object.freeze({ minimum: 1, maximum: 3, defaultValue: 2 }),
  al_frequency_space_x_px: Object.freeze({ minimum: -10, maximum: 10, defaultValue: 0 }),
  al_frequency_space_y_px: Object.freeze({ minimum: -10, maximum: 10, defaultValue: 0 }),
  ...Object.fromEntries(VISUAL_BAND_FREQUENCIES_HZ.map((frequency) => [
    `al_band_${frequency}`,
    Object.freeze({ minimum: 0.5, maximum: 1.5, defaultValue: 1 }),
  ])),
});
const FAIL_CLOSED_PRODUCTION_PARAMETERS = Object.freeze([
  'al_color_space_enabled',
  'al_color_space_strength_percent',
  'al_pip_jitter_px',
]);

function shaderParameterDefault(source, name) {
  const lines = String(source).split(/\r?\n/);
  const declarations = lines.flatMap((line, index) =>
    line.trim() === `//!PARAM ${name}` ? [index] : [],
  );
  assert.equal(declarations.length, 1, `${name} 必须且只能声明一次`);
  const start = declarations[0] + 1;
  const end = lines.findIndex((line, index) => index >= start && /^\/\/!(?:PARAM|HOOK)\b/.test(line.trim()));
  const block = lines.slice(start, end === -1 ? lines.length : end);
  const defaultLine = block.find((line) => line.trim() !== '' && !line.trim().startsWith('//!'));
  assert.ok(defaultLine, `${name} 缺少默认值`);
  const value = Number(defaultLine.trim());
  assert.ok(Number.isFinite(value), `${name} 默认值必须是有限数`);
  return value;
}

function bracedBody(source, openingBraceIndex) {
  assert.equal(source[openingBraceIndex], '{', '缺少 GLSL 块起始花括号');
  let depth = 1;
  for (let index = openingBraceIndex + 1; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) return source.slice(openingBraceIndex + 1, index);
  }
  assert.fail('GLSL 块缺少结束花括号');
}

function shaderFunctionBody(source, name) {
  const signature = new RegExp(`\\b(?:bool|float|void|vec[234]|mat[234])\\s+${name}\\s*\\([^)]*\\)\\s*\\{`).exec(source);
  assert.ok(signature, `缺少 GLSL 函数 ${name}`);
  return bracedBody(source, signature.index + signature[0].lastIndexOf('{'));
}

function shaderConditionalBody(source, conditionName) {
  const condition = new RegExp(`\\bif\\s*\\(\\s*${conditionName}\\s*\\)\\s*\\{`).exec(source);
  assert.ok(condition, `缺少 ${conditionName} 条件块`);
  return bracedBody(source, condition.index + condition[0].lastIndexOf('{'));
}

function shaderAssignments(source) {
  return [...String(source).matchAll(/\b(?:bool|int|float|vec[234])\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^;]+);/g)]
    .map((match) => ({ name: match[1], expression: match[2], index: match.index }));
}

function shaderFunctionCalls(source, name) {
  const calls = [];
  const startPattern = new RegExp(`\\b${name}\\s*\\(`, 'g');
  for (let match = startPattern.exec(source); match; match = startPattern.exec(source)) {
    const argumentStart = startPattern.lastIndex;
    let depth = 1;
    let cursor = argumentStart;
    for (; cursor < source.length && depth > 0; cursor += 1) {
      if (source[cursor] === '(') depth += 1;
      if (source[cursor] === ')') depth -= 1;
    }
    assert.equal(depth, 0, `${name} 调用缺少结束括号`);
    const argumentSource = source.slice(argumentStart, cursor - 1);
    const argumentsList = [];
    let argumentDepth = 0;
    let argumentOffset = 0;
    for (let index = 0; index <= argumentSource.length; index += 1) {
      const character = argumentSource[index];
      if (character === '(') argumentDepth += 1;
      if (character === ')') argumentDepth -= 1;
      if ((character === ',' && argumentDepth === 0) || index === argumentSource.length) {
        argumentsList.push(argumentSource.slice(argumentOffset, index).trim());
        argumentOffset = index + 1;
      }
    }
    calls.push(argumentsList);
    startPattern.lastIndex = cursor;
  }
  return calls;
}

function assertNeutralProductGate(source, enabledName, strengthName) {
  const code = String(source).replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
  const assignments = [...code.matchAll(/\bfloat\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^;]+);/g)];
  const gate = assignments.find(([, , expression]) => {
    const factors = expression.split('*');
    return factors.some((factor) => factor.includes(enabledName))
      && factors.some((factor) => factor.includes(strengthName));
  });
  assert.ok(gate, `${enabledName} 与 ${strengthName} 必须通过乘法合成为门控权重`);
  const [, gateName, expression] = gate;
  const normalizedStrength = new RegExp(
    `\\b${strengthName}\\b\\s*\\/\\s*100(?:\\.0+)?\\b|`
      + `\\b${strengthName}\\b\\s*\\*\\s*0\\.01\\b|`
      + `\\b0\\.01\\b\\s*\\*\\s*${strengthName}\\b`,
  );
  assert.match(expression, normalizedStrength, `${strengthName} 必须按百分比归一化`);
  return gateName;
}

function assertShaderParameterContract(source, expected) {
  const parameters = parseShaderParameters(source);
  assert.deepEqual(
    parameters.map(({ name }) => name).sort(),
    Object.keys(expected).sort(),
  );
  const byName = new Map(parameters.map((parameter) => [parameter.name, parameter]));
  for (const [name, contract] of Object.entries(expected)) {
    assert.deepEqual(byName.get(name), {
      name,
      minimum: contract.minimum,
      maximum: contract.maximum,
    });
    assert.equal(shaderParameterDefault(source, name), contract.defaultValue);
    const parameterBlock = new RegExp(
      `^//!PARAM\\s+${name}\\s*$([\\s\\S]*?)(?=^//!PARAM\\b|^//!HOOK\\b)`,
      'm',
    ).exec(source)?.[1] ?? '';
    assert.equal([...parameterBlock.matchAll(/^\/\/!TYPE\s+float\s*$/gm)].length, 1,
      `${name} 必须且只能声明一次 //!TYPE float`);
  }
}

function assertSingleFrameFragmentCandidate(source) {
  assert.equal([...source.matchAll(/\bvec4\s+hook\s*\(\s*\)\s*\{/g)].length, 1,
    '候选必须且只能提供一个普通 vec4 hook() fragment 入口');
  assert.doesNotMatch(source, /^\/\/!COMPUTE\b/m);
  assert.doesNotMatch(source, /\b(?:gl_GlobalInvocationID|imageStore|memoryBarrier|barrier)\b/);
  assert.doesNotMatch(source, /^\/\/!SAVE\b/m, '候选不得保存历史纹理');
  const requiredDirectives = [
    /^\/\/!HOOK\s+MAIN\s*$/gm,
    /^\/\/!BIND\s+HOOKED\s*$/gm,
    /^\/\/!DESC\s+\S.*$/gm,
    /^\/\/!WIDTH\s+HOOKED\.w\s*$/gm,
    /^\/\/!HEIGHT\s+HOOKED\.h\s*$/gm,
    /^\/\/!WHEN\s+\S.*$/gm,
  ];
  const directivePositions = requiredDirectives.map((pattern) => {
    const matches = [...source.matchAll(pattern)];
    assert.equal(matches.length, 1, `候选元数据 ${pattern.source} 必须且只能出现一次`);
    return matches[0].index;
  });
  assert.deepEqual([...directivePositions].sort((left, right) => left - right), directivePositions,
    'HOOK/BIND/DESC/WIDTH/HEIGHT/WHEN 元数据顺序必须稳定');
  const bindings = [...source.matchAll(/^\/\/!BIND\s+(\S+)\s*$/gm)].map((match) => match[1]);
  assert.deepEqual([...new Set(bindings)], ['HOOKED'], '候选只能读取当前 HOOKED 帧');
  const parameters = parseShaderParameters(source).map(({ name }) => name);
  assert.ok(parameters.every((name) => !/(?:^|_)(?:pts|time|clock|history)(?:_|$)/i.test(name)),
    '候选参数不得引入历史或时间依赖');
  const code = String(source).replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
  assert.doesNotMatch(code, /\b(?:PTS|TIME|frame_index|history_tex)\b/,
    '候选 GLSL 不得读取 PTS、时间、帧号或历史纹理');
}

function assertEarlyNeutralReturn(hookBody, gateName, sourceName) {
  const pattern = new RegExp(
    `\\bif\\s*\\([^)]*\\b${gateName}\\b[^)]*(?:<=|==)\\s*0(?:\\.0+)?[^)]*\\)\\s*\\{[^{}]*\\breturn\\s+${sourceName}\\s*;`,
    's',
  );
  assert.match(hookBody, pattern, `${gateName}=0 必须在采样或绘制前直接返回 ${sourceName}`);
}

test('当前画面范围只验收 1080p 的 24/25/30/50/60fps', () => {
  assert.deepEqual(SAMPLE_SPECS.filter(({ width }) => width === 1920).map(({ fps }) => fps), [24, 25, 30, 50, 60]);
  assert.ok(SAMPLE_SPECS.every(({ width, height }) => width === 1920 && height === 1080));
  assert.ok(SAMPLE_SPECS.every(({ gateRole }) => gateRole === 'full_resolution_render'));
});

test('CLI 只接受报告路径，不开放任意二进制或命令输入', () => {
  assert.deepEqual(parseCliArgs(['--report', 'phase1.json']), { report: join(process.cwd(), 'phase1.json'), help: false });
  assert.throws(() => parseCliArgs(['--mpv', 'evil.exe']), /不支持的参数/);
  assert.throws(() => parseCliArgs(['--report']), /缺少/);
});

test('受管管道与 IPC 请求强制 request_id 和大小边界', () => {
  const pipe = managedPipeName('safe-123');
  assert.equal(pipe, String.raw`\\.\pipe\autolive-mpv-phase1-safe-123`);
  const request = JSON.parse(encodeIpcRequest(['get_property', 'vo-configured'], 7));
  assert.equal(request.request_id, 7);
  assert.deepEqual(request.command, ['get_property', 'vo-configured']);
  assert.throws(() => encodeIpcRequest(['x'.repeat(40_000)], 8), /大小上限/);
  assert.throws(() => managedPipeName('../bad'), /令牌无效|命名管道/);
});

test('IPC 响应拒绝超限、空值和非对象 JSON', () => {
  assert.equal(parseIpcResponseLine('{"error":"success","request_id":1}').request_id, 1);
  assert.throws(() => parseIpcResponseLine(Buffer.alloc(9), 8), /大小无效/);
  assert.throws(() => parseIpcResponseLine('null'), /必须是对象/);
  assert.throws(() => parseIpcResponseLine('{'), /有效 JSON/);
});

test('假管道按 request_id 匹配响应，并执行每请求 deadline', async () => {
  const responsiveSocket = fakeSocket(function respond(chunk) {
    const request = JSON.parse(chunk.toString('utf8'));
    queueMicrotask(() => this.push(`${JSON.stringify({ error: 'success', data: true, request_id: request.request_id })}\n`));
  });
  const responsiveClient = new JsonIpcClient(responsiveSocket, { maxResponseBytes: 1024 });
  assert.equal((await responsiveClient.send(['get_property', 'vo-configured'], 100)).data, true);
  responsiveClient.close();

  const silentClient = new JsonIpcClient(fakeSocket(), { maxResponseBytes: 1024 });
  await assert.rejects(silentClient.send(['get_property', 'vo-configured'], 5), /超时/);
  silentClient.close();
});

test('IPC 忽略已超时请求的迟到响应但仍拒绝真正未知 ID', async () => {
  let requestCount = 0;
  const lateSocket = fakeSocket(function respond(chunk) {
    const request = JSON.parse(chunk.toString('utf8'));
    requestCount += 1;
    if (requestCount === 1) {
      setTimeout(() => this.push(`${JSON.stringify({ error: 'success', data: 'late', request_id: request.request_id })}\n`), 15);
    } else {
      queueMicrotask(() => this.push(`${JSON.stringify({ error: 'success', data: 'current', request_id: request.request_id })}\n`));
    }
  });
  const client = new JsonIpcClient(lateSocket, { maxResponseBytes: 1024 });
  await assert.rejects(client.send(['get_property', 'vo-configured'], 5), /超时/);
  await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  assert.equal((await client.send(['get_property', 'vo-configured'], 100)).data, 'current');
  lateSocket.push('{"error":"success","request_id":999999}\n');
  await assert.rejects(client.send(['get_property', 'vo-configured'], 100), /未知或重复 request_id/);
  client.close();
});

test('shader 参数只从 hook 声明构造单条原子快照', () => {
  const source = `//!PARAM al_brightness_percent\n//!TYPE float\n//!MINIMUM -100\n//!MAXIMUM 100\n+0.0\n\n//!PARAM al_enabled\n//!TYPE float\n//!MINIMUM 0\n//!MAXIMUM 1\n+0.0\n\n//!HOOK MAIN\n`;
  const parameters = parseShaderParameters(source);
  assert.deepEqual(parameters, [
    { name: 'al_brightness_percent', minimum: -100, maximum: 100 },
    { name: 'al_enabled', minimum: 0, maximum: 1 },
  ]);
  assert.match(buildShaderOptions(parameters, 3), /^al_brightness_percent=-62\.5,al_enabled=0\.375$/);
  assert.throws(() => buildShaderOptions([{ name: 'x;quit', minimum: 0, maximum: 1 }], 0), /不安全/);
});

test('门禁只接受当前晋级后的精确 gpu83 shader 哈希和 80 个动态选项', async () => {
  const shaderPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const source = await readFile(shaderPath, 'utf8');
  const parameters = parseShaderParameters(source);
  assert.equal(verifyShaderContract(source, parameters).dynamicParameterCount, 80);
  assert.throws(() => verifyShaderContract(`${source}\n// tampered`, parameters), /已审计契约不匹配/);
});

test('Phase 3A baseline 保持生产首段并保留修正后的像素语义', async () => {
  const productionPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const baselinePath = new URL('./shader-candidates/gpu83-baseline-candidate.hook', import.meta.url);
  const [productionSource, baselineSource] = await Promise.all([
    readFile(productionPath, 'utf8'),
    readFile(baselinePath, 'utf8'),
  ]);
  const baselineNames = parseShaderParameters(baselineSource).map(({ name }) => name).sort();
  assert.equal(baselineNames.length, 20);
  assert.match(productionSource, /^\/\/ BEGIN desktop\/tools\/shader-candidates\/gpu83-baseline-candidate\.hook\r?\n/);
  assert.ok(baselineNames.every((name) => productionSource.includes(`//!PARAM ${name}`)));

  const hueBody = shaderFunctionBody(baselineSource, 'al_hue_rotate');
  const hueProjections = shaderAssignments(hueBody)
    .filter(({ expression }) => /\bdot\s*\(\s*color\s*,/.test(expression));
  assert.equal(hueProjections.length, 3, 'hue 必须显式计算三个 color dot 投影');
  for (const { name, index } of hueProjections) {
    const declarationEnd = hueBody.indexOf(';', index);
    assert.match(hueBody.slice(declarationEnd + 1), new RegExp(`\\b${name}\\b`), `${name} dot 结果必须进入后续 hue 计算`);
  }

  const hookBody = shaderFunctionBody(baselineSource, 'hook');
  const assignments = shaderAssignments(hookBody);
  const brightnessIndex = hookBody.indexOf('al_brightness_percent');
  const contrastIndex = hookBody.indexOf('al_contrast_percent');
  const luma = assignments.find(({ expression, index }) =>
    index > contrastIndex && /\bdot\s*\(\s*color\.rgb\s*,/.test(expression));
  const saturationIndex = hookBody.indexOf('al_saturation_percent');
  assert.ok(brightnessIndex >= 0 && contrastIndex > brightnessIndex);
  assert.ok(luma && luma.index > contrastIndex && saturationIndex > luma.index,
    '亮度和对比度之后、饱和度之前必须重新计算 luma');
  const saturationStatement = hookBody.slice(
    hookBody.lastIndexOf(';', saturationIndex) + 1,
    hookBody.indexOf(';', saturationIndex),
  );
  assert.match(saturationStatement, new RegExp(`\\b${luma.name}\\b`));

  const blurMix = assignments.find(({ name }) => name === 'blur_mix');
  assert.ok(blurMix);
  assert.match(blurMix.expression, /\bal_blur_radius_px\b/);
  assert.match(blurMix.expression, /\bal_edge_softness_percent\b\s*\/\s*100(?:\.0+)?\b/);
  const geometryIndex = hookBody.indexOf('if (has_geometry_transform)');
  assert.ok(geometryIndex > blurMix.index, '全帧 blur_mix 必须位于几何裁剪条件之外');
  assert.ok(shaderFunctionCalls(hookBody.slice(0, geometryIndex), 'mix')
    .some((argumentsList) => argumentsList[2] === 'blur_mix'));

  const cropBody = shaderConditionalBody(hookBody, 'has_geometry_transform');
  assert.match(cropBody, /\bal_crop_edge_smoothing\b/);
  assert.match(cropBody, /\bsmoothstep\s*\(/);
  assert.ok(shaderFunctionCalls(cropBody, 'mix').some((argumentsList) => argumentsList[2] === 'crop_weight'));
  assert.doesNotMatch(cropBody, /\bal_edge_softness_percent\b/,
    'edge_softness 已在全帧柔化中生效，crop 羽化不得重复叠加');
});

test('Phase 3A image-repair 候选只声明两字段并实现四邻域边缘感知修复', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-image-repair-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  const parameters = parseShaderParameters(source);
  assert.deepEqual(
    parameters.map(({ name }) => name).sort(),
    Object.keys(IMAGE_REPAIR_CANDIDATE_PARAMETERS).sort(),
  );
  const byName = new Map(parameters.map((parameter) => [parameter.name, parameter]));
  for (const [name, expected] of Object.entries(IMAGE_REPAIR_CANDIDATE_PARAMETERS)) {
    assert.deepEqual(byName.get(name), { name, minimum: expected.minimum, maximum: expected.maximum });
    assert.equal(shaderParameterDefault(source, name), expected.defaultValue);
  }
  assert.match(source, /^\/\/!WHEN\s+al_image_repair_enabled\s+al_image_repair_strength_percent\s+\*\s*$/m,
    '图像修复中性参数必须跳过五采样 pass');

  const hookBody = shaderFunctionBody(source, 'hook');
  const assignments = new Map(shaderAssignments(hookBody).map((assignment) => [assignment.name, assignment]));
  assert.match(assignments.get('source_color')?.expression ?? '', /\bHOOKED_tex\s*\(\s*HOOKED_pos\s*\)/);
  assert.match(assignments.get('pixel_size')?.expression ?? '', /^\s*HOOKED_pt\s*$/);
  const neighbors = {
    left: /HOOKED_pos\s*-\s*vec2\s*\(\s*pixel_size\.x\s*,\s*0(?:\.0+)?\s*\)/,
    right: /HOOKED_pos\s*\+\s*vec2\s*\(\s*pixel_size\.x\s*,\s*0(?:\.0+)?\s*\)/,
    up: /HOOKED_pos\s*-\s*vec2\s*\(\s*0(?:\.0+)?\s*,\s*pixel_size\.y\s*\)/,
    down: /HOOKED_pos\s*\+\s*vec2\s*\(\s*0(?:\.0+)?\s*,\s*pixel_size\.y\s*\)/,
  };
  assert.doesNotMatch(hookBody, /\bexp\s*\(/, '图像修复权重不得使用昂贵指数');
  for (const [name, offsetPattern] of Object.entries(neighbors)) {
    const sample = assignments.get(name)?.expression ?? '';
    assert.match(sample, /\bHOOKED_tex\s*\(/);
    assert.match(sample, offsetPattern, `${name} 必须采样 HOOKED 的一像素邻域`);
    const weight = assignments.get(`${name}_weight`)?.expression ?? '';
    assert.match(
      weight,
      /^\s*1(?:\.0+)?\s*\/\s*\(\s*1(?:\.0+)?\s*\+\s*64(?:\.0+)?\s*\*\s*dot\s*\(/,
      `${name}_weight 必须使用 1/(1+64*dot(diff,diff)) 有界权重`,
    );
    const dotCalls = shaderFunctionCalls(weight, 'dot');
    assert.equal(dotCalls.length, 1);
    assert.equal(dotCalls[0].length, 2);
    const normalizedDiffs = dotCalls[0].map((argument) => argument.replace(/\s+/g, ''));
    assert.equal(normalizedDiffs[0], normalizedDiffs[1], 'dot 的两个输入必须是同一个颜色差');
    const diffName = dotCalls[0][0].trim();
    const diffExpression = /^[A-Za-z_][A-Za-z0-9_]*$/.test(diffName)
      ? assignments.get(diffName)?.expression ?? ''
      : dotCalls[0][0];
    assert.match(diffExpression, new RegExp(`\\b${name}\\b\\s*-\\s*source_color\\.rgb`));
  }
  const weightSum = assignments.get('weight_sum')?.expression ?? '';
  const repaired = assignments.get('repaired_color')?.expression ?? '';
  assert.match(repaired, /\bsource_color\.rgb\b/);
  assert.match(repaired, /\/\s*weight_sum\b/);
  for (const name of Object.keys(neighbors)) {
    assert.match(weightSum, new RegExp(`\\b${name}_weight\\b`));
    assert.match(repaired, new RegExp(`\\b${name}\\b\\s*\\*\\s*${name}_weight\\b`));
  }

  const gateName = assertNeutralProductGate(
    source,
    'al_image_repair_enabled',
    'al_image_repair_strength_percent',
  );
  const finalMix = shaderFunctionCalls(hookBody, 'mix').find((argumentsList) =>
    argumentsList[0] === 'source_color.rgb'
      && argumentsList[1] === 'repaired_color'
      && argumentsList[2] === gateName);
  assert.ok(finalMix, '最终 mix 必须以原中心色、repaired_color 和归一门控为三个输入');
  assert.match(hookBody, /\breturn\s+vec4\s*\(\s*mix\s*\(/);
  assert.match(hookBody, /\bsource_color\.a\b/);
});

test('Phase 3A abstract-face 候选以 UV 直定位十槽完成有界透明合成', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-abstract-face-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  assertShaderParameterContract(source, ABSTRACT_FACE_CANDIDATE_PARAMETERS);
  assertSingleFrameFragmentCandidate(source);

  const hookBody = shaderFunctionBody(source, 'hook');
  const assignments = shaderAssignments(hookBody);
  const sourceColor = assignments.find(({ expression }) =>
    /^\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*$/.test(expression));
  assert.ok(sourceColor, '抽象脸必须先读取当前帧中心样本作为原色');
  const count = assignments.find(({ expression }) =>
    /\bal_abstract_face_count\b/.test(expression)
      && /\bfloor\s*\(/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && /\b0(?:\.0+)?\b/.test(expression)
      && /\b10(?:\.0+)?\b/.test(expression));
  const opacity = assignments.find(({ expression }) =>
    /\bal_abstract_face_opacity_percent\b/.test(expression)
      && /\/\s*100(?:\.0+)?\b/.test(expression)
      && /\bclamp\s*\(/.test(expression));
  assert.ok(count, 'count 必须先夹紧到参数范围');
  assert.ok(opacity, 'opacity 必须先按百分比归一化并夹紧');
  assertEarlyNeutralReturn(hookBody, count.name, sourceColor.name);
  assertEarlyNeutralReturn(hookBody, opacity.name, sourceColor.name);
  assert.match(source, /^\/\/!WHEN\s+al_abstract_face_count\s+al_abstract_face_opacity_percent\s+\*\s*$/m,
    '抽象脸必须在 count 或 opacity 为零时由 mpv 跳过整个 pass');

  assert.doesNotMatch(hookBody, /\bfor\s*\(/, '抽象脸不得对每个像素遍历全部十槽');
  assert.doesNotMatch(hookBody, /\b(?:random|rand|hash|noise|fract)\s*\(/i,
    '抽象脸位置和轮廓必须是确定性几何');
  const column = assignments.find(({ name, expression }) =>
    /column/i.test(name) && /HOOKED_pos\.x\s*\*\s*5(?:\.0+)?/.test(expression) && /\bfloor\s*\(/.test(expression));
  const row = assignments.find(({ name, expression }) =>
    /row/i.test(name) && /HOOKED_pos\.y\s*\*\s*2(?:\.0+)?/.test(expression) && /\bfloor\s*\(/.test(expression));
  const slotIndex = assignments.find(({ name, expression }) =>
    /slot.*index|index.*slot/i.test(name)
      && column && row
      && new RegExp(`\\b${row.name}\\b\\s*\\*\\s*5`).test(expression)
      && new RegExp(`\\b${column.name}\\b`).test(expression));
  assert.ok(column && row && slotIndex, '当前像素必须直接映射到固定 5x2 槽位');
  assert.match(
    hookBody,
    new RegExp(`\\bif\\s*\\(\\s*${slotIndex.name}\\s*>=\\s*${count.name}\\s*\\)\\s*\\{[^{}]*\\breturn\\s+${sourceColor.name}\\s*;`, 's'),
    '未启用槽位必须直接返回原帧',
  );

  const faceWidth = assignments.find(({ name, expression }) =>
    /width|size/i.test(name) && /\bal_abstract_face_size_percent\b/.test(expression) && /\/\s*100(?:\.0+)?\b/.test(expression));
  const faceScale = assignments.find(({ name, expression }) =>
    /scale/i.test(name) && faceWidth && new RegExp(`\\b${faceWidth.name}\\b`).test(expression));
  assert.ok(faceWidth, '脸部宽度必须由 size_percent/100 得到');
  assert.ok(faceScale, '脸部局部坐标比例必须由参数化宽度得到');

  const centerBody = shaderFunctionBody(source, 'al_face_center');
  assert.match(centerBody, /\breturn\s+vec2\s*\(/, '槽位中心必须由索引确定性返回');
  assert.doesNotMatch(centerBody, /\b(?:random|rand|hash|noise|fract)\s*\(/i);

  const facePoint = assignments.find(({ name, expression }) =>
    /face.*point|point.*face/i.test(name)
      && /\bHOOKED_pos\b/.test(expression)
      && new RegExp(`\\bal_face_center\\s*\\(\\s*${slotIndex.name}\\s*\\)`).test(expression)
      && new RegExp(`\\b${faceScale.name}\\b`).test(expression));
  assert.ok(facePoint, '当前像素必须经过确定性中心和参数化尺寸转换为脸部局部坐标');
  assert.match(hookBody, new RegExp(`\\bgreaterThan\\s*\\(\\s*abs\\s*\\(\\s*${facePoint.name}\\s*\\)`),
    '进入 SDF 前必须用局部包围盒旁路无关像素');

  const maskBody = shaderFunctionBody(source, 'al_face_mask');
  const maskAssignments = shaderAssignments(maskBody);
  const primitive = (pattern, label) => {
    const assignment = maskAssignments.find(({ name, expression }) => pattern.test(name) && /\bpoint\b/.test(expression));
    assert.ok(assignment, `${label} 必须由同一脸部局部坐标计算`);
    return assignment;
  };
  const outline = primitive(/outline/i, '脸部轮廓');
  const leftEye = primitive(/left.*eye|eye.*left/i, '左眼');
  const rightEye = primitive(/right.*eye|eye.*right/i, '右眼');
  const mouth = primitive(/mouth/i, '嘴部');
  const maskReturn = /\breturn\s+([^;]+);/.exec(maskBody)?.[1] ?? '';
  for (const { name } of [outline, leftEye, rightEye, mouth]) {
    assert.match(maskReturn, new RegExp(`\\b${name}\\b`), `${name} 必须进入最终 face mask`);
  }
  assert.equal(shaderFunctionCalls(hookBody, 'al_face_mask').length, 1,
    '每个像素最多只能计算一次脸部 SDF');
  assert.match(hookBody, new RegExp(`\\bfloat\\s+mask\\s*=\\s*al_face_mask\\s*\\(\\s*${facePoint.name}\\s*\\)\\s*;`),
    '活动槽必须只计算其自身 face mask');

  const result = assignments.find(({ name, expression }) =>
    /result|composite/i.test(name)
      && new RegExp(`\\b${sourceColor.name}\\.rgb\\b`).test(expression)
      && new RegExp(`\\b${opacity.name}\\b`).test(expression)
      && /\bmask\b/.test(expression)
      && /\bmix\s*\(/.test(expression));
  assert.ok(result, '最终 mix 必须由原色、确定性几何 mask 和归一化 opacity 驱动');
  assert.match(hookBody, new RegExp(`\\breturn\\s+vec4\\s*\\(\\s*${result.name}\\s*,\\s*${sourceColor.name}\\.a\\s*\\)\\s*;`),
    '抽象脸合成必须保留原始 alpha');
});

test('local-blur 候选由语义开关与 runtime 门共同控制有界九点二维 tent 采样', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-local-blur-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  assertShaderParameterContract(source, LOCAL_BLUR_CANDIDATE_PARAMETERS);
  assertSingleFrameFragmentCandidate(source);
  assert.doesNotMatch(source, /\b(?:al_)?local_blur_interval(?:_ms)?\b/i,
    '局部模糊单帧候选不得声明或读取 interval');

  const hookBody = shaderFunctionBody(source, 'hook');
  const assignments = shaderAssignments(hookBody);
  const sourceColor = assignments.find(({ expression }) =>
    /^\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*$/.test(expression));
  assert.ok(sourceColor, '局部模糊必须先读取当前中心像素');
  assertEarlyNeutralReturn(hookBody, 'al_local_blur_enabled', sourceColor.name);
  assertEarlyNeutralReturn(hookBody, 'al_runtime_local_blur_active', sourceColor.name);
  assert.match(source, /^\/\/!WHEN\s+al_local_blur_enabled\s+al_runtime_local_blur_active\s+\*\s*$/m,
    '局部模糊语义开关或 runtime 门关闭时必须由 mpv 跳过整个 pass');

  const halfRegion = assignments.find(({ name, expression }) =>
    /half.*region|region.*half/i.test(name)
      && /\bal_local_blur_region_percent\b/.test(expression)
      && /\/\s*200(?:\.0+)?\b/.test(expression)
      && /\bvec2\s*\(/.test(expression));
  const centerDelta = assignments.find(({ name, expression }) =>
    /center.*(?:delta|distance)|(?:delta|distance).*center/i.test(name)
      && /\babs\s*\(\s*HOOKED_pos\s*-\s*vec2\s*\(\s*0\.5\s*\)\s*\)/.test(expression)
      && !/HOOKED_pt/.test(expression));
  assert.ok(halfRegion, '中心区域二维半宽必须分别由 region_percent/200 得到');
  assert.ok(centerDelta, '区域门控必须基于归一化坐标到画面中心的二维距离');
  assert.match(
    hookBody,
    new RegExp(`\\bif\\s*\\(\\s*${centerDelta.name}\\.x\\s*>\\s*${halfRegion.name}\\.x\\s*\\|\\|\\s*${centerDelta.name}\\.y\\s*>\\s*${halfRegion.name}\\.y\\s*\\)\\s*\\{[^{}]*\\breturn\\s+${sourceColor.name}\\s*;`, 's'),
    '区域外必须直接返回原帧',
  );

  const sampleOffset = assignments.find(({ name, expression }) =>
    /offset|radius/i.test(name)
      && /\bal_local_blur_radius_px\b/.test(expression)
      && /\bHOOKED_pt\b/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && /\*/.test(expression));
  assert.ok(sampleOffset, '采样偏移必须由夹紧后的 radius_px * HOOKED_pt 计算');
  const samples = shaderFunctionCalls(hookBody, 'HOOKED_tex');
  assert.equal(samples.length, 9,
    '局部模糊成本必须恒定为中心加八邻域共九次采样');
  assert.equal(samples.filter(([coordinate]) => coordinate === 'HOOKED_pos').length, 1,
    '九点采样必须且只能包含一个中心像素');
  const neighborCoordinates = samples.filter(([coordinate]) => coordinate !== 'HOOKED_pos').map(([coordinate]) => coordinate);
  const neighborPatterns = [
    new RegExp(`HOOKED_pos\\s*-\\s*vec2\\s*\\(\\s*${sampleOffset.name}\\.x\\s*,\\s*0(?:\\.0+)?\\s*\\)`),
    new RegExp(`HOOKED_pos\\s*\\+\\s*vec2\\s*\\(\\s*${sampleOffset.name}\\.x\\s*,\\s*0(?:\\.0+)?\\s*\\)`),
    new RegExp(`HOOKED_pos\\s*-\\s*vec2\\s*\\(\\s*0(?:\\.0+)?\\s*,\\s*${sampleOffset.name}\\.y\\s*\\)`),
    new RegExp(`HOOKED_pos\\s*\\+\\s*vec2\\s*\\(\\s*0(?:\\.0+)?\\s*,\\s*${sampleOffset.name}\\.y\\s*\\)`),
    new RegExp(`HOOKED_pos\\s*\\+\\s*${sampleOffset.name}\\b`),
    new RegExp(`HOOKED_pos\\s*-\\s*${sampleOffset.name}\\b`),
    new RegExp(`HOOKED_pos\\s*\\+\\s*vec2\\s*\\(\\s*${sampleOffset.name}\\.x\\s*,\\s*-${sampleOffset.name}\\.y\\s*\\)`),
    new RegExp(`HOOKED_pos\\s*\\+\\s*vec2\\s*\\(\\s*-${sampleOffset.name}\\.x\\s*,\\s*${sampleOffset.name}\\.y\\s*\\)`),
  ];
  for (const coordinatePattern of neighborPatterns) {
    const coordinate = neighborCoordinates.find((candidate) => coordinatePattern.test(candidate));
    assert.ok(coordinate, `缺少二维八邻域采样：${coordinatePattern}`);
    assert.match(coordinate, /^\s*clamp\s*\(/, '八邻域采样坐标必须 clamp 到局部区域');
    assert.match(coordinate, /\bsample_min\b\s*,\s*\bsample_max\b/,
      'clamp 边界必须限制在局部区域内');
  }
  const blurred = assignments.find(({ name, expression }) =>
    /blur/i.test(name)
      && new RegExp(`\\b${sourceColor.name}\\.rgb\\s*\\*\\s*4`).test(expression)
      && shaderFunctionCalls(expression, 'HOOKED_tex').length === 8
      && /\/\s*16(?:\.0+)?\b/.test(expression));
  assert.ok(blurred, '模糊色必须由 4/2/1 权重的九点二维 tent 核得到');
  const regionWeight = assignments.find(({ name, expression }) =>
    /region.*weight|weight.*region/i.test(name) && /\bsmoothstep\s*\(/.test(expression));
  const mixWeight = assignments.find(({ name, expression }) =>
    /mix.*weight|weight.*mix/i.test(name)
      && /\bal_local_blur_enabled\b/.test(expression)
      && regionWeight && new RegExp(`\\b${regionWeight.name}\\b`).test(expression));
  assert.ok(regionWeight && mixWeight, '局部区域边缘必须羽化后再与 enabled 合成最终权重');
  const finalMix = shaderFunctionCalls(hookBody, 'mix').find((argumentsList) =>
    argumentsList[0] === `${sourceColor.name}.rgb`
      && argumentsList[1] === blurred.name
      && argumentsList[2] === mixWeight.name);
  assert.ok(finalMix, '区域内必须由羽化权重门控原帧与九点模糊色的最终 mix');
  assert.match(hookBody, new RegExp(`\\breturn\\s+vec4\\s*\\(\\s*mix\\s*\\([^;]+\\)\\s*,\\s*${sourceColor.name}\\.a\\s*\\)\\s*;`),
    '局部模糊必须只处理 RGB 并保留原始 alpha');
});

test('Phase 3A edge-fill 候选以固定上限同帧采样复现边缘填充语义', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-edge-fill-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  assertShaderParameterContract(source, EDGE_FILL_CANDIDATE_PARAMETERS);
  assertSingleFrameFragmentCandidate(source);
  assert.match(source, /^\/\/!WHEN\s+al_edge_fill_enabled\s*$/m,
    '边缘填充关闭时必须由 mpv 跳过整个 pass');

  const hookBody = shaderFunctionBody(source, 'hook');
  const assignments = shaderAssignments(hookBody);
  const sourceColor = assignments.find(({ expression }) =>
    /^\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*$/.test(expression));
  assert.ok(sourceColor, '边缘填充必须先读取当前像素');
  assertEarlyNeutralReturn(hookBody, 'al_edge_fill_enabled', sourceColor.name);

  const feather = assignments.find(({ name, expression }) =>
    /feather/i.test(name)
      && /\bal_edge_feather_percent\b/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && /\b0(?:\.0+)?\b/.test(expression)
      && /\b100(?:\.0+)?\b/.test(expression));
  const borderPixels = assignments.find(({ name, expression }) =>
    /border.*(?:px|pixel)|(?:px|pixel).*border/i.test(name)
      && feather && new RegExp(`\\b${feather.name}\\b\\s*\\*\\s*0\\.14`).test(expression)
      && /\bfloor\s*\(/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && /\b2(?:\.0+)?\b/.test(expression)
      && /\b16(?:\.0+)?\b/.test(expression));
  assert.ok(feather, 'edge_feather_percent 必须先夹紧到 0–100%');
  assert.ok(borderPixels, '边缘宽度必须按旧契约映射并固定夹紧到 2–16px');

  const borderUv = assignments.find(({ name, expression }) =>
    /border.*(?:uv|offset)|(?:uv|offset).*border/i.test(name)
      && borderPixels && /\bHOOKED_pt\b/.test(expression)
      && new RegExp(`\\b${borderPixels.name}\\b`).test(expression));
  const filledPosition = assignments.find(({ name, expression }) =>
    /fill.*(?:position|coord)|(?:position|coord).*fill/i.test(name)
      && borderUv && /\bHOOKED_pos\b/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && new RegExp(`\\b${borderUv.name}\\b`).test(expression));
  assert.ok(borderUv && filledPosition, '边缘像素必须夹紧到 2–16px 内侧同帧坐标');
  assert.match(
    hookBody,
    new RegExp(`\\bif\\s*\\(\\s*all\\s*\\(\\s*equal\\s*\\(\\s*${filledPosition.name}\\s*,\\s*HOOKED_pos\\s*\\)\\s*\\)\\s*\\)\\s*\\{[^{}]*\\breturn\\s+${sourceColor.name}\\s*;`, 's'),
    '非边缘像素必须在第二次采样前直接返回原帧',
  );

  const samples = shaderFunctionCalls(hookBody, 'HOOKED_tex');
  assert.equal(samples.length, 2, '边缘填充每像素最多只能读取原像素与一个内侧像素');
  assert.ok(samples.some(([coordinate]) => coordinate === filledPosition.name),
    '第二次采样必须读取夹紧后的同帧内侧坐标');

  const fillWeight = assignments.find(({ name, expression }) =>
    /fill.*weight|weight.*fill/i.test(name)
      && feather && new RegExp(`\\b${feather.name}\\b\\s*<=\\s*0(?:\\.0+)?`).test(expression)
      && /\?\s*1(?:\.0+)?\s*:/.test(expression)
      && new RegExp(`\\b${feather.name}\\b\\s*\\/\\s*100(?:\\.0+)?`).test(expression));
  assert.ok(fillWeight, '零羽化必须保持 2px 硬填充，非零羽化按百分比混合');
  const finalMix = shaderFunctionCalls(hookBody, 'mix').find((argumentsList) =>
    argumentsList[0] === `${sourceColor.name}.rgb`
      && argumentsList[2] === fillWeight.name);
  assert.ok(finalMix, '边缘颜色必须由原 RGB、单个内侧样本和 fill weight 合成');
  assert.match(hookBody, new RegExp(`\\breturn\\s+vec4\\s*\\(\\s*mix\\s*\\([^;]+\\)\\s*,\\s*${sourceColor.name}\\.a\\s*\\)\\s*;`),
    '边缘填充必须保留原始 alpha');
});

test('Phase 3A channel-offset 候选直接在浮点 RGB 上执行对向通道偏移', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-channel-offset-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  assertShaderParameterContract(source, CHANNEL_OFFSET_CANDIDATE_PARAMETERS);
  assertSingleFrameFragmentCandidate(source);
  assert.match(source, /^\/\/!WHEN\s+al_channel_offset_percent\s*$/m,
    '通道偏移为零时必须由 mpv 跳过整个 pass');

  const hookBody = shaderFunctionBody(source, 'hook');
  const assignments = shaderAssignments(hookBody);
  const sourceColor = assignments.find(({ expression }) =>
    /^\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*$/.test(expression));
  assert.ok(sourceColor, '通道偏移必须且只能从当前像素开始');
  assert.equal(shaderFunctionCalls(hookBody, 'HOOKED_tex').length, 1,
    '通道偏移不得增加邻域或历史采样');

  const channelOffset = assignments.find(({ name, expression }) =>
    /channel.*offset|offset.*channel/i.test(name)
      && /\bal_channel_offset_percent\b/.test(expression)
      && /\bclamp\s*\(/.test(expression)
      && /-10(?:\.0+)?/.test(expression)
      && /\b10(?:\.0+)?\b/.test(expression)
      && /\/\s*100(?:\.0+)?\b/.test(expression));
  assert.ok(channelOffset, '通道偏移必须从 -10–10% 夹紧后直接归一化到浮点 RGB');
  assertEarlyNeutralReturn(hookBody, channelOffset.name, sourceColor.name);
  assert.doesNotMatch(hookBody, /\b(?:mod|floor|ceil)\s*\(/,
    'GPU 浮点通道偏移不得复制旧 8-bit code-value 抖动');

  const shifted = assignments.find(({ name, expression }) =>
    /shift|result/i.test(name)
      && new RegExp(`\\b${sourceColor.name}\\.r\\s*\\+\\s*${channelOffset.name}\\b`).test(expression)
      && new RegExp(`\\b${sourceColor.name}\\.g\\b`).test(expression)
      && new RegExp(`\\b${sourceColor.name}\\.b\\s*-\\s*${channelOffset.name}\\b`).test(expression)
      && /\bclamp\s*\(/.test(expression));
  assert.ok(shifted, '正值必须增加红通道、保持绿通道并减少蓝通道，负值方向相反');
  assert.match(hookBody, new RegExp(`\\breturn\\s+vec4\\s*\\(\\s*${shifted.name}\\s*,\\s*${sourceColor.name}\\.a\\s*\\)\\s*;`),
    '通道偏移必须保留原始 alpha');
});

test('Phase 3A spatial-modulation 候选以一个共享 pass 保持 12 段独立空间语义', async () => {
  const candidatePath = new URL('./shader-candidates/gpu83-spatial-modulation-candidate.hook', import.meta.url);
  const source = await readFile(candidatePath, 'utf8');
  assertShaderParameterContract(source, SPATIAL_MODULATION_CANDIDATE_PARAMETERS);
  assertSingleFrameFragmentCandidate(source);
  assert.match(source, /^\/\/!WHEN\s+1\s*$/m,
    '空间调制使用固定 pass 集合，避免超长 libplacebo WHEN 表达式解析失败');

  const hookBody = shaderFunctionBody(source, 'hook');
  const shaderCode = source.replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
  const assignments = shaderAssignments(hookBody);
  const sourceColor = assignments.find(({ expression }) =>
    /^\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*$/.test(expression));
  assert.ok(sourceColor, '空间调制必须先读取当前像素');
  assert.equal(shaderFunctionCalls(hookBody, 'HOOKED_tex').length, 1,
    '共享空间调制不得增加邻域、历史或音频纹理采样');
  assert.doesNotMatch(shaderCode, /\b(?:PTS|TIME|frame|audio|spectrum|fft|history)\b/i,
    'Phase 3A 空间调制不得读取 PTS、帧号、历史或音频频谱');
  assert.doesNotMatch(hookBody, /\bfor\s*\(/,
    '12 个固定频段必须显式有界展开，不能留下运行时循环');

  assert.match(source, /\blog2?\s*\(/,
    '65–20000Hz 必须通过对数位置映射为空间周期');
  assert.match(source, /\b65(?:\.0+)?\b/);
  assert.match(source, /\b20000(?:\.0+)?\b/);
  assert.match(source, /\b32(?:\.0+)?\b/);
  for (const frequency of VISUAL_BAND_FREQUENCIES_HZ) {
    const parameter = `al_band_${frequency}`;
    assert.match(hookBody, new RegExp(`\\b${parameter}\\b\\s*-\\s*1(?:\\.0+)?\\b`),
      `${frequency}Hz 必须以自身 weight-1 的独立 delta 进入共享 pass`);
  }
  assert.equal(shaderFunctionCalls(hookBody, 'al_spatial_cycles').length, 1,
    '12 个固定频段必须预计算周期常量，每像素最多只为可选 target 做一次对数映射');

  assert.match(hookBody, /\bal_target_frequency_hz\b\s*>=\s*AL_MIN_FREQUENCY_HZ\b/,
    'target=0 必须作为 None 哨兵，其余值映射目标空间频率');
  assert.match(hookBody, /\bal_core_frequency_hz\b\s*>=\s*AL_MIN_FREQUENCY_HZ\b/,
    'core=0 必须跟随目标频率，不能单独激活效果');
  assert.match(hookBody, /\bal_frequency_space_x_px\b/);
  assert.match(hookBody, /\bal_frequency_space_y_px\b/);
  assert.match(hookBody, /\bHOOKED_pt\b/,
    '频率空间 XY 偏移必须按像素通过 HOOKED_pt 转为 UV');
  const dimension = assignments.find(({ name, expression }) =>
    /dimension/i.test(name) && /\bal_space_dimension\b/.test(expression) && /\bclamp\s*\(/.test(expression));
  assert.ok(dimension, '空间维度必须夹紧并离散到 1–3');
  assert.match(hookBody, new RegExp(`\\b${dimension.name}\\b\\s*>=\\s*2(?:\\.0+)?\\b`));
  assert.match(hookBody, new RegExp(`\\b${dimension.name}\\b\\s*>=\\s*3(?:\\.0+)?\\b`),
    '三维模式必须有独立同帧空间分量');
  assert.match(hookBody, /\blength\s*\(/,
    '第三空间维度必须使用同帧径向分量，不得偷用时间轴');
  assert.match(hookBody, /\bal_dynamic_eq_threshold\b/,
    '动态均衡阈值必须限制频段组合幅度');
  assert.match(hookBody, /\bal_wave_grain_count\b/);
  assert.match(hookBody, /\bal_wave_level\b/);
  assert.match(hookBody, /\bal_wave_intensity\b/);

  const firstNeutralReturn = new RegExp(`\\breturn\\s+${sourceColor.name}\\s*;`).exec(hookBody);
  assert.ok(firstNeutralReturn, '固定 pass 的全中性组合必须在调制计算前返回原帧');
  assert.ok(firstNeutralReturn.index < hookBody.search(/\bsin\s*\(/),
    '中性旁路必须早于正弦载波计算');
  assert.match(hookBody, new RegExp(`\\breturn\\s+vec4\\s*\\([^;]*${sourceColor.name}\\.a[^;]*\\)\\s*;`),
    '空间调制必须保留原始 alpha');
});

test('Phase 3A 独立 fragment 候选不进入 Tauri bundle', async () => {
  const tauriConfigPath = new URL('../src-tauri/tauri.conf.json', import.meta.url);
  const resources = JSON.parse(await readFile(tauriConfigPath, 'utf8')).bundle.resources
    .map((resource) => resource.replace(/\\/g, '/'));
  for (const filename of [
    'gpu83-abstract-face-candidate.hook',
    'gpu83-local-blur-candidate.hook',
    'gpu83-edge-fill-candidate.hook',
    'gpu83-channel-offset-candidate.hook',
    'gpu83-spatial-modulation-candidate.hook',
    'gpu83-same-frame-overlays-candidate.hook',
    'gpu83-scheduler-gates-candidate.hook',
  ]) {
    assert.ok(resources.every((resource) => !resource.endsWith(filename)), `${filename} 不得进入 Tauri bundle`);
  }
});

test('Phase 3A 候选源码只晋级到唯一生产 shader，候选文件本身不进入 Tauri bundle', async () => {
  const productionPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const tauriConfigPath = new URL('../src-tauri/tauri.conf.json', import.meta.url);
  const [productionSource, tauriConfigSource] = await Promise.all([
    readFile(productionPath, 'utf8'),
    readFile(tauriConfigPath, 'utf8'),
  ]);

  assert.equal(parseShaderParameters(productionSource).length, 80);
  for (const name of FAIL_CLOSED_PRODUCTION_PARAMETERS) {
    assert.doesNotMatch(productionSource, new RegExp(`^//\\!PARAM ${name}$`, 'm'));
  }

  const resources = JSON.parse(tauriConfigSource).bundle.resources
    .map((resource) => resource.replace(/\\/g, '/'));
  const shaderResources = resources
    .filter((resource) => resource.startsWith('resources/shaders/'));
  assert.deepEqual(shaderResources, ['resources/shaders/gpu83.hook']);
  for (const filename of [
    'gpu83-baseline-candidate.hook',
    'gpu83-image-repair-candidate.hook',
    'gpu83-same-frame-overlays-candidate.hook',
  ]) {
    assert.ok(resources.every((resource) => !resource.endsWith(filename)), `${filename} 不得进入 Tauri bundle`);
  }
});

test('CPU4 只编译固定四字段、固定滤镜标签和 vf-command', () => {
  const commands = compileCpu4Update({
    brightness_percent: 10,
    contrast_percent: 110,
    saturation_percent: 90,
    hue_rotation_degrees: -12,
  });
  assert.deepEqual(commands, [
    ['vf-command', 'autolive_cpu4', 'brightness', '0.1', 'eq@autolive_cpu4_eq'],
    ['vf-command', 'autolive_cpu4', 'contrast', '1.1', 'eq@autolive_cpu4_eq'],
    ['vf-command', 'autolive_cpu4', 'saturation', '0.9', 'eq@autolive_cpu4_eq'],
    ['vf-command', 'autolive_cpu4', 'h', '-12', 'hue@autolive_cpu4_hue'],
  ]);
  assert.throws(() => compileCpu4Update({ brightness_percent: 0 }), /只接受/);
  assert.throws(
    () => compileCpu4Update({ brightness_percent: 0, contrast_percent: 100, saturation_percent: 100, hue_rotation_degrees: 0, command: 'quit' }),
    /只接受/,
  );
  assert.throws(
    () => compileCpu4Update({ brightness_percent: Number.NaN, contrast_percent: 100, saturation_percent: 100, hue_rotation_degrees: 0 }),
    /有限数/,
  );
});

test('GPU 启动参数与生产 D3D11/D3D11VA、Vulkan/WinVK 契约一致', () => {
  const pipe = managedPipeName('args');
  const common = {
    mediaPath: 'C:\\media\\sample.mp4', shaderPath: 'C:\\trusted\\gpu83.hook',
    pipeName: pipe, renderSize: { width: 1920, height: 1080 }, cpu4: false,
  };
  const d3d11 = buildMpvArguments({ ...common, backend: 'd3d11' });
  assert.ok(!d3d11.includes('--load-scripts=no'));
  assert.ok(!d3d11.includes('--osc=no'));
  assert.ok(d3d11.includes('--gpu-api=d3d11'));
  assert.ok(d3d11.includes('--gpu-context=d3d11'));
  assert.ok(d3d11.includes('--hwdec=d3d11va'));
  assert.ok(d3d11.includes('--terminal=yes'));
  assert.ok(d3d11.includes('--input-terminal=no'));
  assert.ok(d3d11.includes('--geometry=1920x1080'));
  assert.ok(d3d11.includes('--glsl-shaders=C:\\trusted\\gpu83.hook'));
  const vulkan = buildMpvArguments({ ...common, backend: 'vulkan' });
  assert.ok(vulkan.includes('--gpu-api=vulkan'));
  assert.ok(vulkan.includes('--gpu-context=winvk'));
  assert.ok(vulkan.includes('--hwdec=d3d11va-copy'));

  const hidden = buildMpvArguments({ ...common, backend: 'd3d11', hiddenWindow: true });
  assert.ok(!hidden.includes('--window-minimized=yes'));
  assert.ok(hidden.includes('--focus-on=never'));
  assert.ok(hidden.includes('--show-in-taskbar=no'));
  assert.ok(hidden.includes('--geometry=1920x1080'));
  assert.ok(!hidden.some((argument) => argument.startsWith('--autofit')));
  assert.ok(hidden.includes('--border=no'));
  assert.ok(hidden.includes('--video-unscaled=yes'));
  assert.ok(hidden.includes('--hidpi-window-scale=no'));
  assert.ok(hidden.includes('--auto-window-resize=no'));
  assert.ok(!hidden.includes('--fullscreen=yes'));

  const fullResolutionGate = buildMpvArguments({
    ...common,
    backend: 'd3d11',
    hiddenWindow: true,
    fullscreenWindow: true,
  });
  assert.ok(fullResolutionGate.includes('--fullscreen=yes'));
  assert.ok(fullResolutionGate.includes('--fs-screen=current'));
  assert.ok(!d3d11.includes('--window-minimized=yes'));
  assert.ok(!d3d11.includes('--focus-on=never'));
  assert.ok(!d3d11.includes('--show-in-taskbar=no'));
  assert.ok(!d3d11.some((argument) => argument.startsWith('--autofit')));
  assert.ok(!d3d11.includes('--border=no'));
  assert.ok(!d3d11.includes('--video-unscaled=yes'));
  assert.ok(!d3d11.includes('--hidpi-window-scale=no'));
  assert.ok(!d3d11.includes('--auto-window-resize=no'));

  const multipleShaders = buildMpvArguments({
    ...common,
    backend: 'd3d11',
    shaderPaths: ['C:\\trusted\\baseline.hook', 'C:\\trusted\\repair.hook'],
  });
  assert.ok(multipleShaders.includes('--glsl-shader=C:\\trusted\\baseline.hook'));
  assert.ok(multipleShaders.includes('--glsl-shader=C:\\trusted\\repair.hook'));
  assert.ok(!multipleShaders.some((argument) => argument.startsWith('--glsl-shaders=')));

  assert.throws(
    () => buildMpvArguments({ ...common, backend: 'd3d11', shaderPaths: [] }),
    /allowNoShaderStart/,
  );
  const noShaderStart = buildMpvArguments({
    ...common, backend: 'd3d11', shaderPaths: [], allowNoShaderStart: true,
  });
  assert.ok(!noShaderStart.some((argument) => argument.startsWith('--glsl-shader')));
});

test('CPU4 启动图固定且不加载 shader', () => {
  const argumentsList = buildMpvArguments({
    backend: 'cpu4', mediaPath: 'C:\\media\\sample.mp4', shaderPath: '',
    pipeName: managedPipeName('cpu4'), renderSize: { width: 1920, height: 1080 }, cpu4: true,
  });
  assert.ok(argumentsList.includes('--hwdec=no'));
  assert.ok(argumentsList.some((argument) => argument.includes('@autolive_cpu4:lavfi=[eq@autolive_cpu4_eq=') && argument.includes('hue@autolive_cpu4_hue=')));
  assert.ok(argumentsList.some((argument) => argument.includes('hue@autolive_cpu4_hue=h=0:s=1')));
  assert.ok(!argumentsList.some((argument) => argument.startsWith('--glsl-shaders=')));
});

test('延迟报告保留每次样本并计算 nearest-rank P99', () => {
  const values = Array.from({ length: 200 }, (_, index) => index + 1);
  assert.equal(percentile(values, 99), 198);
  const summary = summarizeLatencies([1, 2, 4]);
  assert.deepEqual(summary.samplesMs, [1, 2, 4]);
  assert.equal(summary.totalMs, 7);
  assert.equal(summary.p99Ms, 4);
});

test('固定性能快照必须非空、有界、同序且保持预序列化内容', () => {
  const snapshots = ['al_a=0,al_b=1', 'al_a=1.25,al_b=-2e-3'];
  const parameters = [
    { name: 'al_b', minimum: -1, maximum: 2 },
    { name: 'al_a', minimum: 0, maximum: 2 },
  ];
  assert.deepEqual(validateUpdateOptionSnapshots(snapshots, parameters), snapshots);
  assert.equal(validateUpdateOptionSnapshots(null), null);
  assert.throws(() => validateUpdateOptionSnapshots([]), /非空/);
  assert.throws(() => validateUpdateOptionSnapshots(['al_a=0'], parameters), /完整参数集合/);
  assert.throws(() => validateUpdateOptionSnapshots(['al_a=0,al_a=1'], parameters), /重复参数/);
  assert.throws(() => validateUpdateOptionSnapshots(['al_a=0,al_b=3'], parameters), /参数越界/);
  assert.throws(() => validateUpdateOptionSnapshots(['al_a=1e999,al_b=1'], parameters), /非有限/);
  assert.throws(
    () => validateUpdateOptionSnapshots(['al_a=0,al_b=1', 'al_b=1,al_a=0'], parameters),
    /相同字段顺序/,
  );
  assert.throws(() => validateUpdateOptionSnapshots(['al_a=0\nquit'], [{ name: 'al_a' }]), /非 ASCII/);
  assert.throws(
    () => validateUpdateOptionSnapshots([`al_a=${'1'.repeat(33 * 1024)}`], [{ name: 'al_a' }]),
    /超长/,
  );
});

test('PID 证据要求启动、IPC 与子进程三者全程非空且一致', () => {
  assert.equal(requireStableSessionPid(42, 42, 42, '测试阶段'), 42);
  assert.throws(() => requireStableSessionPid(42, null, 42, '测试阶段'), /不完整或不一致/);
  assert.throws(() => requireStableSessionPid(42, 43, 42, '测试阶段'), /42\/43\/42/);
  assert.throws(() => requireStableSessionPid(null, 42, 42, '测试阶段'), /不完整或不一致/);
});

test('实际渲染表面必须与样本分辨率完全一致', () => {
  const sample = { width: 1920, height: 1080 };
  assert.equal(renderTargetMatchesSample(sample, { w: 1920, h: 1080 }), true);
  assert.equal(renderTargetMatchesSample(sample, { w: 1280, h: 720 }), false);
  assert.equal(renderTargetMatchesSample(sample, null), false);
});

test('GPU 渲染目标限定仅撤销伪通过，不降级 failed 或 unverified', async () => {
  const { qualifyFrameBudgetForRenderTarget } = await import('./verify-mpv-phase1.mjs');
  const measured = {
    status: 'passed',
    sourceFps: 60,
    frameIntervalBudgetMs: 16.667,
    gpuProcessingP99TargetMs: 12,
    sampleCount: 100,
    minimumSamples: 100,
    timing: { p99Ms: 8.779, samplesMs: [8.779] },
  };
  const failed = { ...measured, status: 'failed', reason: 'P99 超出门禁' };
  const unverified = { ...measured, status: 'unverified', reason: '样本不足' };

  for (const evidence of [measured, failed, unverified]) {
    assert.strictEqual(qualifyFrameBudgetForRenderTarget(evidence, true), evidence);
  }
  assert.deepEqual(qualifyFrameBudgetForRenderTarget(measured, false), {
    ...measured,
    status: 'unverified',
    reason: '实际渲染表面与样本分辨率不一致，原始计时样本仅供诊断，不构成全分辨率帧预算通过证据',
  });
  assert.strictEqual(qualifyFrameBudgetForRenderTarget(failed, false), failed);
  assert.strictEqual(qualifyFrameBudgetForRenderTarget(unverified, false), unverified);
});

test('帧预算使用 mpv vo-passes 样本，shader 证据要求精确源码契约', () => {
  const frameEvidence = buildFrameBudgetEvidence({ fps: 60 });
  assert.equal(frameEvidence.status, 'unverified');
  assert.equal(frameEvidence.frameIntervalBudgetMs, 16.667);
  const measured = buildFrameBudgetEvidence(
    { fps: 60 },
    { fresh: [{ desc: 'hook', samples: [4_000_000, 5_000_000] }, { desc: 'scale', samples: [2_000_000, 3_000_000] }] },
    { fresh: [{ desc: 'scale', samples: [] }, { desc: 'hook', samples: [] }] },
    2,
  );
  assert.equal(measured.status, 'passed');
  assert.deepEqual(measured.timing.samplesMs, [6, 8]);
  const tooFew = buildFrameBudgetEvidence(
    { fps: 60 },
    { fresh: [{ desc: 'hook', samples: [4_000_000] }] },
    { fresh: [{ desc: 'hook', samples: [] }] },
  );
  assert.equal(tooFew.status, 'unverified');
  assert.equal(tooFew.sampleCount, 1);

  const absentTelemetry = shaderCompilationEvidence('startup\n', 'startup\nno compile event\n');
  assert.equal(absentTelemetry.status, 'unverified');
  assert.equal(absentTelemetry.telemetryReliable, false);
  const sourceVerified = shaderCompilationEvidence('startup\n', 'startup\nno compile event\n', true);
  assert.equal(sourceVerified.status, 'verified');
  assert.equal(sourceVerified.sourceContract.matched, true);
  const repeated = shaderCompilationEvidence('startup shader compiled\n', 'startup shader compiled\nshader compiling again\n');
  assert.equal(repeated.status, 'failed');
  assert.equal(repeated.repeatedCompileDetected, true);
  const discontinuous = shaderCompilationEvidence('startup\n', 'rotated log\n', true);
  assert.equal(discontinuous.status, 'unverified');
  assert.equal(discontinuous.logContinuity, false);
});

test('Phase 7A 准入身份可替换旧二进制常量且保留精确源码证据', () => {
  const identity = validateMpvIdentity({
    versionToken: 'v0.41.0-UNKNOWN',
    expectedSha256: 'a'.repeat(64),
    sourceRef: '7b8915bc1d04c7e1b61184e00c7fbfaab1911e75',
    sourceUrl: 'https://github.com/mpv-player/mpv/tree/7b8915bc1d04c7e1b61184e00c7fbfaab1911e75',
  });
  const evidence = shaderCompilationEvidence(
    'startup\n',
    'startup\nno compile event\n',
    true,
    identity,
  );

  assert.equal(evidence.status, 'verified');
  assert.equal(evidence.sourceContract.ref, identity.sourceRef);
  assert.equal(evidence.sourceContract.url, identity.sourceUrl);
  assert.throws(
    () => validateMpvIdentity({ ...identity, expectedSha256: 'not-a-hash' }),
    /mpv 身份 SHA-256 无效/,
  );
});

test('逐帧帧预算快照允许不同 count、pass 数和重复 desc', () => {
  const first = observeFrameBudgetSample({
    fresh: [
      { desc: 'upload', last: 1_000_000, count: 12, samples: [900_000, 1_000_000] },
      { desc: 'upload', last: 2_000_000, count: 7, samples: [1_900_000, 2_000_000] },
    ],
  });
  const next = observeFrameBudgetSample({
    fresh: [{ desc: 'gpu83', last: 2_100_000, count: 13, samples: [2_000_000, 2_100_000] }],
  });
  assert.equal(first.totalNs, 3_000_000);
  assert.deepEqual(JSON.parse(first.telemetryIdentity), [
    [0, 'upload', 12, [900_000, 1_000_000]],
    [1, 'upload', 7, [1_900_000, 2_000_000]],
  ]);
  assert.deepEqual(first.passDescriptions, ['upload', 'upload']);
  assert.equal(next.totalNs, 2_100_000);
  assert.equal(next.telemetryIdentity, observeFrameBudgetSample({
    fresh: [{ desc: 'gpu83', last: 9_999_999, count: 13, samples: [2_000_000, 2_100_000] }],
  }).telemetryIdentity, '仅 last 变化不得制造新 identity');
  assert.ok(observeFrameBudgetSample({ fresh: [
    { desc: 'upload', last: 1, count: 12, samples: [1] },
    { desc: 'gpu83', last: 2, count: 13, samples: [2] },
  ] }));
  assert.equal(observeFrameBudgetSample({ fresh: [{ desc: 'bad', last: -1, count: 1, samples: [-1] }] }), null);
});

test('帧预算只跳过相同 telemetry identity，合法变化与 count 重置均接受', () => {
  const identity = JSON.stringify([[0, 'upload', 100, [1]], [1, 'gpu83', 7, [2]]]);
  const changed = JSON.stringify([[0, 'upload', 0, [1]]]);
  assert.deepEqual(
    advanceFrameBudgetObservation(null, { telemetryIdentity: identity }),
    { accepted: true, telemetryIdentity: identity },
  );
  assert.deepEqual(
    advanceFrameBudgetObservation(identity, { telemetryIdentity: identity }),
    { accepted: false, telemetryIdentity: identity },
  );
  assert.deepEqual(
    advanceFrameBudgetObservation(identity, { telemetryIdentity: changed }),
    { accepted: true, telemetryIdentity: changed },
  );
  assert.throws(() => advanceFrameBudgetObservation(identity, { telemetryIdentity: 'not-json' }), /identity 无效/i);
  assert.throws(() => advanceFrameBudgetObservation('[]', { telemetryIdentity: identity }), /identity 无效/i);
  assert.throws(() => advanceFrameBudgetObservation('[]', null), /identity 无效/i);
});

test('帧预算轮询至少间隔一个源帧周期', () => {
  assert.equal(frameObservationSettleMs({ fps: 60 }), 22);
  assert.equal(frameObservationSettleMs({ fps: 24 }), 47);
  assert.throws(() => frameObservationSettleMs({ fps: 0 }), /sample fps/);
});

test('CPU4 命令只重试短暂滤镜图重建窗口并保留永久失败', async () => {
  let calls = 0;
  const waits = [];
  const retries = await sendCpu4CommandWithRetry({
    async send() {
      calls += 1;
      if (calls < 3) throw new Error('graph rebuilding');
    },
  }, ['vf-command'], 4, async (milliseconds) => waits.push(milliseconds));
  assert.equal(retries, 2);
  assert.deepEqual(waits, [25, 25]);
  await assert.rejects(
    sendCpu4CommandWithRetry({ async send() { throw new Error('permanent'); } }, ['vf-command'], 2, async () => {}),
    /连续 2 次失败.*permanent/,
  );
});

test('逐帧帧预算证据必须包含至少一百个有效帧快照', () => {
  const sample = { fps: 60 };
  const passed = buildFrameBudgetEvidence(sample, null, null, 100, Array.from({ length: 100 }, () => 1_000_000));
  assert.equal(passed.status, 'passed');
  assert.equal(passed.sampleCount, 100);
  const insufficient = buildFrameBudgetEvidence(sample, null, null, 100, Array.from({ length: 99 }, () => 1_000_000));
  assert.equal(insufficient.status, 'unverified');
  assert.match(insufficient.reason, /99.*100/);
});

test('丢帧门禁同时检查 VO 与 decoder 计数增量', () => {
  assert.equal(
    buildDropFrameEvidence(
      { frameDropCount: 2, decoderFrameDropCount: 1 },
      { frameDropCount: 2, decoderFrameDropCount: 1 },
    ).status,
    'passed',
  );
  assert.equal(
    buildDropFrameEvidence(
      { frameDropCount: 2, decoderFrameDropCount: 1 },
      { frameDropCount: 3, decoderFrameDropCount: 1 },
    ).status,
    'failed',
  );
  assert.equal(buildDropFrameEvidence({}, {}).status, 'unverified');
});

test('丢帧时间线对任一计数器 reset 或增长都 fail-closed', () => {
  const resetOnly = buildDropFrameTimelineEvidence([
    { frameDropCount: 1, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
  ]);
  assert.equal(resetOnly.status, 'failed');
  assert.equal(resetOnly.resets.frameDropCount, 1);
  assert.deepEqual(resetOnly.resetEvents, [
    { field: 'frameDropCount', sampleIndex: 1, previous: 1, current: 0 },
  ]);
  const dropped = buildDropFrameTimelineEvidence([
    { frameDropCount: 0, decoderFrameDropCount: 0 },
    { frameDropCount: 1, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
  ]);
  assert.equal(dropped.status, 'failed');
  assert.equal(dropped.increments.frameDropCount, 1);
  assert.equal(buildDropFrameTimelineEvidence([{ frameDropCount: 0 }]).status, 'unverified');
});

test('更新阶段 PTS 必须完整、单调不回退且实际推进', () => {
  assert.equal(UPDATE_CONTINUITY_IPC_COMMAND_COUNT, 5);
  const sample = { fps: 60, duration: 15 };
  const timeline = (positions, intervalMs = 70) => positions.map((mediaPtsSeconds, index) => ({
    mediaPtsSeconds,
    observedAtMs: index * intervalMs,
  }));
  const passed = buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.17, 0.24]), 2, sample);
  assert.equal(passed.status, 'passed');
  assert.equal(passed.monotonicNonDecreasing, true);
  assert.equal(passed.strictlyAdvancing, true);
  assert.equal(passed.loopOrResetDetected, false);
  assert.equal(passed.deltaSeconds, 0.14);
  assert.equal(passed.remainingWindowPassed, true);

  const reset = buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.17, 0.01]), 2, sample);
  assert.equal(reset.status, 'failed');
  assert.equal(reset.loopOrResetDetected, true);
  assert.deepEqual(reset.resetEvents, [
    { sampleIndex: 2, previous: 0.17, current: 0.01 },
  ]);
  assert.match(reset.reason, /跨 loop|重置/);

  const stalled = buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.1, 0.14]), 2, sample);
  assert.equal(stalled.status, 'failed');
  assert.equal(stalled.strictlyAdvancing, false);
  assert.match(stalled.reason, /PTS 停滞/);

  const delayed = buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.14, 0.18], 80), 2, sample);
  assert.equal(delayed.status, 'failed');
  assert.match(delayed.reason, /超过一个视频帧/);

  const exhausted = buildUpdatePlaybackTimelineEvidence(
    timeline([12.9, 13, 13.1]), 2, sample,
  );
  assert.equal(exhausted.status, 'failed');
  assert.match(exhausted.reason, /剩余播放窗口不足两秒/);

  const longStall = timeline([...Array(120).fill(0), 0.1]);
  assert.equal(buildUpdatePlaybackTimelineEvidence(longStall, 120, sample).status, 'failed');
  assert.equal(buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.2]), 2, sample).status, 'failed');
  assert.equal(buildUpdatePlaybackTimelineEvidence(
    timeline([0.1, Number.NaN, 0.2]), 2, sample,
  ).status, 'failed');
  assert.equal(buildUpdatePlaybackTimelineEvidence(timeline([0.1, 0.17, 0.24]), 2).status, 'failed');
});

test('runMpvAttempt 固定 setup/baseline/probe/PID 顺序并把 probe 日志纳入重复编译检测', async () => {
  const source = await readFile(new URL('./verify-mpv-phase1.mjs', import.meta.url), 'utf8');
  const start = source.indexOf('export async function runMpvAttempt');
  const end = source.indexOf('async function runCpu4Attempt', start);
  const body = source.slice(start, end);
  const orderedTokens = [
    'await waitForFirstFrame(ipc)',
    'const ipcPidBeforeSetup = requireStableSessionPid',
    'await sessionSetup(',
    'const ipcPidAfterSetup = requireStableSessionPid',
    'const logsBeforeUpdates =',
    'const ipcPidBeforeProbe = requireStableSessionPid',
    'const dropsBeforeProbe = await dropFrameCounters(ipc)',
    'await sessionProbe(',
    'const ipcPidAfterProbe = requireStableSessionPid',
    'const dropsAfterProbe = await dropFrameCounters(ipc)',
    'const voPassesBeforeUpdates =',
    'const dropFrameTimeline = [dropsBeforeProbe, dropsAfterProbe]',
    'shaderCompilationEvidence(',
  ];
  const positions = orderedTokens.map((token) => {
    const position = body.indexOf(token);
    assert.notEqual(position, -1, `缺少顺序节点：${token}`);
    return position;
  });
  assert.deepEqual([...positions].sort((left, right) => left - right), positions);
  assert.match(body, /fixedOptionSnapshots\[index % fixedOptionSnapshots\.length\]/);
  assert.match(body, /advanceFrameBudgetObservation\(lastTelemetryIdentity, observation\)/);
  assert.match(body, /updatePlaybackPositions\.push\(await captureUpdatePlaybackPosition\(ipc\)\)/);
  assert.match(body, /buildUpdatePlaybackTimelineEvidence\(updatePlaybackPositions, updateCount, sample\)/);
  assert.match(body, /updatePlaybackTimelineEvidence: updatePlaybackTimeline/);
  assert.match(body, /sessionSetupEvidence,/);
  assert.match(body, /sessionProbeEvidence,/);
  assert.match(body, /const frameBudgetEvidence = qualifyFrameBudgetForRenderTarget\(frameBudget, renderTargetMatched\)/);
  assert.match(body, /pidUnchanged: false/);
});

test('CPU4 与 GPU 共用 telemetry 去重和实际渲染表面限定', async () => {
  const source = await readFile(new URL('./verify-mpv-phase1.mjs', import.meta.url), 'utf8');
  const start = source.indexOf('async function runCpu4Attempt');
  const end = source.indexOf('export async function versionEvidence', start);
  const body = source.slice(start, end);
  assert.match(body, /let lastTelemetryIdentity = null/);
  assert.match(body, /advanceFrameBudgetObservation\(lastTelemetryIdentity, observation\)/);
  assert.match(body, /if \(advancement\.accepted\) observedFrameTotalsNs\.push\(observation\.totalNs\)/);
  assert.match(body, /const osdDimensions = await optionalProperty\(ipc, 'osd-dimensions'\)/);
  assert.match(body, /const renderTargetMatched = renderTargetMatchesSample\(sample, osdDimensions\)/);
  assert.match(body, /qualifyFrameBudgetForRenderTarget\(rawFrameBudget, renderTargetMatched\)/);
  assert.match(body, /CPU4 渲染表面分辨率证据不符/);
});

test('GPU 与 CPU4 失败 catch 同时保留 stdout/stderr 有界证据', async () => {
  const source = await readFile(new URL('./verify-mpv-phase1.mjs', import.meta.url), 'utf8');
  const gpuStart = source.indexOf('export async function runMpvAttempt');
  const cpuStart = source.indexOf('async function runCpu4Attempt', gpuStart);
  const versionStart = source.indexOf('export async function versionEvidence', cpuStart);
  for (const [label, body] of [
    ['GPU', source.slice(gpuStart, cpuStart)],
    ['CPU4', source.slice(cpuStart, versionStart)],
  ]) {
    const failureCatch = body.slice(body.lastIndexOf('} catch (error) {'), body.lastIndexOf('} finally {'));
    assert.match(failureCatch, /const stdout = readStdout\(\)/, `${label} 失败 catch 必须读取 stdout`);
    assert.match(failureCatch, /const stderr = readStderr\(\)/, `${label} 失败 catch 必须读取 stderr`);
    for (const field of ['stdoutBytes', 'stdoutOverflow', 'stdoutTail', 'stderrBytes', 'stderrOverflow', 'stderrTail']) {
      assert.match(failureCatch, new RegExp(`\\b${field}\\b`), `${label} 失败 catch 缺少 ${field}`);
    }
  }
});

test('报告状态不把不可用或失败伪造成通过', () => {
  assert.equal(finalReportStatus([{ status: 'passed' }]), 'passed');
  assert.equal(finalReportStatus([{ status: 'passed' }, { status: 'unavailable' }]), 'unavailable');
  assert.equal(finalReportStatus([{ status: 'unavailable' }, { status: 'failed' }]), 'failed');
  assert.equal(finalReportStatus([{ status: 'passed' }, { status: 'unverified' }]), 'unverified');
});

test('GPU 最高分辨率声明要求最终通过且所有 GPU 分辨率证据匹配', async () => {
  const matchedGpu = {
    backend: 'd3d11',
    resolutionEvidence: {
      source: {
        expectedWidth: 1920,
        expectedHeight: 1080,
        actualWidth: 1920,
        actualHeight: 1080,
      },
      windowRenderTarget: { matched: true },
    },
  };
  const cpu4 = { backend: 'cpu4', status: 'passed' };

  assert.equal(maximumValidatedResolutionClaim('failed', [matchedGpu, cpu4]), null);
  assert.equal(maximumValidatedResolutionClaim('unavailable', [matchedGpu, cpu4]), null);
  assert.equal(maximumValidatedResolutionClaim('passed', [
    { backend: 'd3d11', resolutionEvidence: null },
    cpu4,
  ]), null);
  assert.equal(maximumValidatedResolutionClaim('passed', [
    matchedGpu,
    {
      ...matchedGpu,
      backend: 'vulkan',
      resolutionEvidence: {
        ...matchedGpu.resolutionEvidence,
        windowRenderTarget: { matched: false },
      },
    },
    cpu4,
  ]), null);
  assert.equal(maximumValidatedResolutionClaim('passed', [
    {
      ...matchedGpu,
      resolutionEvidence: {
        ...matchedGpu.resolutionEvidence,
        source: { ...matchedGpu.resolutionEvidence.source, actualWidth: 1280 },
      },
    },
    cpu4,
  ]), null);
  assert.equal(maximumValidatedResolutionClaim('passed', [cpu4]), null);
  assert.equal(maximumValidatedResolutionClaim('passed', [
    matchedGpu,
    { ...matchedGpu, backend: 'vulkan' },
    cpu4,
  ]), '1920x1080');

  const source = await readFile(new URL('./verify-mpv-phase1.mjs', import.meta.url), 'utf8');
  const gate = source.slice(source.indexOf('export async function runPhase1Gate'));
  const statusPosition = gate.indexOf('report.status = finalReportStatus(report.attempts)');
  const claimsPosition = gate.indexOf('report.claims = {');
  assert.ok(statusPosition >= 0 && statusPosition < claimsPosition, '最终报告状态必须先于 claims 确定');
  assert.match(gate, /maximumValidatedResolution: maximumValidatedResolutionClaim\(report\.status, report\.attempts\)/);
});

test('性能预算超限属于失败而不是环境不可用', () => {
  assert.equal(classifyAttemptError(new Error('GPU 帧预算未通过：P99 24.331ms')).status, 'failed');
  assert.equal(classifyAttemptError(new Error('vulkan 初始化失败')).status, 'unavailable');
});

test('样本生成器可使用假 ffmpeg 验证固定参数且不依赖 GPU', async () => {
  const root = await mkdtemp(join(tmpdir(), 'autolive-phase1-test-'));
  const calls = [];
  try {
    const samples = await generateSamples('fake-ffmpeg.exe', root, async (executable, argumentsList) => {
      calls.push({ executable, argumentsList });
      await writeFile(argumentsList.at(-1), 'fake-video');
    });
    assert.equal(samples.length, SAMPLE_SPECS.length);
    assert.equal(calls.length, SAMPLE_SPECS.length);
    assert.ok(calls.every(({ executable, argumentsList }) => executable === 'fake-ffmpeg.exe' && argumentsList.includes('libopenh264')));
    assert.ok(calls.every(({ argumentsList }) => !argumentsList.includes('-f lavfi')));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('样本生成器可只生成受限的 Phase 3A 专用样本', async () => {
  const root = await mkdtemp(join(tmpdir(), 'autolive-phase3a-sample-test-'));
  const calls = [];
  const specs = [{
    name: '1080p-60fps-phase3a', width: 1920, height: 1080, fps: 60, duration: 4,
    gateRole: 'candidate_full_resolution_render',
  }];
  try {
    const samples = await generateSamples('fake-ffmpeg.exe', root, async (_executable, argumentsList) => {
      calls.push(argumentsList);
      await writeFile(argumentsList.at(-1), 'fake-video');
    }, specs);
    assert.equal(samples.length, 1);
    assert.equal(samples[0].duration, 4);
    assert.equal(calls.length, 1);
    assert.ok(calls[0].includes('testsrc2=size=1920x1080:rate=60:duration=4'));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('Phase 1 固定 mpv 身份与 Phase 7 发布清单和 Tauri 资源门禁一致', async () => {
  const verifySource = await readFile(new URL('./verify-mpv-phase1.mjs', import.meta.url), 'utf8');
  const prepareSource = await readFile(new URL('./prepare-ffmpeg-resources.mjs', import.meta.url), 'utf8');
  const runtimeReleaseSource = await readFile(new URL('./mpv-runtime-release.mjs', import.meta.url), 'utf8');
  const manifestVerifierSource = await readFile(new URL('./verify-mpv-runtime-manifest-v2.mjs', import.meta.url), 'utf8');
  const buildSource = await readFile(new URL('../src-tauri/build.rs', import.meta.url), 'utf8');
  const buildSupportSource = await readFile(new URL('../src-tauri/build_support.rs', import.meta.url), 'utf8');
  const tauriConfig = JSON.parse(await readFile(
    new URL('../src-tauri/tauri.conf.json', import.meta.url),
    'utf8',
  ));
  const releaseManifest = JSON.parse(await readFile(
    new URL('../third_party/mpv/x86_64-pc-windows-msvc/legal/mpv-runtime-manifest.json', import.meta.url),
    'utf8',
  ));

  const sourceRef = /const VERIFIED_MPV_SOURCE_REF = '([^']+)'/.exec(verifySource)?.[1];
  const versionToken = /const VERIFIED_MPV_VERSION_TOKEN = '([^']+)'/.exec(verifySource)?.[1];
  const binaryHash = /const VERIFIED_MPV_SHA256 = '([0-9a-f]+)'/.exec(verifySource)?.[1];
  assert.equal(sourceRef, releaseManifest.components.mpv.source_ref);
  assert.ok(releaseManifest.components.mpv.version.endsWith(versionToken));
  assert.equal(binaryHash, releaseManifest.files['mpv.exe'].sha256);
  assert.deepEqual(releaseManifest.audit, {
    release_review_status: 'blocked',
    corresponding_source_complete: false,
    third_party_notices_reviewed: false,
  });
  assert.ok(prepareSource.includes('loadValidatedMpvRuntimeRelease'));
  assert.ok(runtimeReleaseSource.includes('validateMpvRuntimeManifestV2'));
  for (const source of [manifestVerifierSource, buildSupportSource]) {
    assert.ok(source.includes('release_review_status'));
    assert.ok(source.includes('corresponding_source_complete'));
    assert.ok(source.includes('third_party_notices_reviewed'));
    assert.ok(source.includes('approved'));
  }
  assert.ok(runtimeReleaseSource.includes('runReproducibleBuildReportGate'));
  assert.ok(runtimeReleaseSource.includes('expectedIdentity'));
  assert.ok(runtimeReleaseSource.includes('assertDescriptor'));
  for (const legalFile of [
    'Copyright.txt',
    'GPL-2.0.txt',
    'LGPL-2.1.txt',
    'SOURCE.md',
    'THIRD-PARTY-NOTICES.md',
  ]) {
    assert.ok(runtimeReleaseSource.includes(legalFile), `运行资源准入缺少法律材料门禁：${legalFile}`);
  }
  assert.ok(tauriConfig.bundle.resources.includes('runtime-resources.json'));
  assert.ok(tauriConfig.bundle.resources.includes('embedded-runtime-resources'));
  assert.match(buildSource, /validate_release_runtime_resource_tree/);
});
