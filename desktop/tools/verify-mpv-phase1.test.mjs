import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Duplex } from 'node:stream';
import test from 'node:test';

import {
  SAMPLE_SPECS,
  JsonIpcClient,
  buildDropFrameEvidence,
  buildDropFrameTimelineEvidence,
  buildFrameBudgetEvidence,
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
  parseCliArgs,
  parseIpcResponseLine,
  parseShaderParameters,
  percentile,
  shaderCompilationEvidence,
  summarizeLatencies,
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
const FAIL_CLOSED_PRODUCTION_PARAMETERS = Object.freeze([
  'al_color_space_enabled',
  'al_color_space_strength_percent',
  ...Object.keys(IMAGE_REPAIR_CANDIDATE_PARAMETERS),
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
  return [...String(source).matchAll(/\b(?:bool|float|vec[234])\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^;]+);/g)]
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

test('门禁只接受当前已审计 gpu83 shader 的精确哈希和 20 个动态参数', async () => {
  const shaderPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const source = await readFile(shaderPath, 'utf8');
  const parameters = parseShaderParameters(source);
  assert.equal(verifyShaderContract(source, parameters).dynamicParameterCount, 20);
  assert.throws(() => verifyShaderContract(`${source}\n// tampered`, parameters), /已审计契约不匹配/);
});

test('Phase 3A baseline 参数集合严格等于生产 20 项并保留修正后的像素语义', async () => {
  const productionPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const baselinePath = new URL('./shader-candidates/gpu83-baseline-candidate.hook', import.meta.url);
  const [productionSource, baselineSource] = await Promise.all([
    readFile(productionPath, 'utf8'),
    readFile(baselinePath, 'utf8'),
  ]);
  const productionNames = parseShaderParameters(productionSource).map(({ name }) => name).sort();
  const baselineNames = parseShaderParameters(baselineSource).map(({ name }) => name).sort();
  assert.deepEqual(baselineNames, productionNames);

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

test('Phase 3A 候选不进入生产 shader 或 Tauri bundle', async () => {
  const productionPath = new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url);
  const tauriConfigPath = new URL('../src-tauri/tauri.conf.json', import.meta.url);
  const [productionSource, tauriConfigSource] = await Promise.all([
    readFile(productionPath, 'utf8'),
    readFile(tauriConfigPath, 'utf8'),
  ]);

  assert.equal(parseShaderParameters(productionSource).length, 20);
  for (const name of FAIL_CLOSED_PRODUCTION_PARAMETERS) {
    assert.doesNotMatch(productionSource, new RegExp(`\\b${name}\\b`));
  }

  const resources = JSON.parse(tauriConfigSource).bundle.resources
    .map((resource) => resource.replace(/\\/g, '/'));
  const shaderResources = resources
    .filter((resource) => resource.startsWith('resources/shaders/'));
  assert.deepEqual(shaderResources, ['resources/shaders/gpu83.hook']);
  for (const filename of ['gpu83-baseline-candidate.hook', 'gpu83-image-repair-candidate.hook']) {
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

test('逐帧帧预算快照允许 pass 列表变化并拒绝重复或畸形 pass', () => {
  const first = observeFrameBudgetSample({
    fresh: [
      { desc: 'upload', last: 1_000_000, count: 12, samples: [900_000, 1_000_000] },
      { desc: 'gpu83', last: 2_000_000, count: 12, samples: [1_900_000, 2_000_000] },
    ],
  });
  const next = observeFrameBudgetSample({
    fresh: [{ desc: 'gpu83', last: 2_100_000, count: 13, samples: [2_000_000, 2_100_000] }],
  });
  assert.equal(first.totalNs, 3_000_000);
  assert.equal(next.totalNs, 2_100_000);
  assert.equal(observeFrameBudgetSample({ fresh: [
    { desc: 'same', last: 1, count: 1, samples: [1] },
    { desc: 'same', last: 2, count: 1, samples: [2] },
  ] }), null);
  assert.equal(observeFrameBudgetSample({ fresh: [{ desc: 'bad', last: -1, count: 1, samples: [-1] }] }), null);
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

test('丢帧时间线允许短素材循环重置但拒绝周期内计数增长', () => {
  const resetOnly = buildDropFrameTimelineEvidence([
    { frameDropCount: 1, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
  ]);
  assert.equal(resetOnly.status, 'passed');
  assert.equal(resetOnly.resets.frameDropCount, 1);
  const dropped = buildDropFrameTimelineEvidence([
    { frameDropCount: 0, decoderFrameDropCount: 0 },
    { frameDropCount: 1, decoderFrameDropCount: 0 },
    { frameDropCount: 0, decoderFrameDropCount: 0 },
  ]);
  assert.equal(dropped.status, 'failed');
  assert.equal(dropped.increments.frameDropCount, 1);
  assert.equal(buildDropFrameTimelineEvidence([{ frameDropCount: 0 }]).status, 'unverified');
});

test('报告状态不把不可用或失败伪造成通过', () => {
  assert.equal(finalReportStatus([{ status: 'passed' }]), 'passed');
  assert.equal(finalReportStatus([{ status: 'passed' }, { status: 'unavailable' }]), 'unavailable');
  assert.equal(finalReportStatus([{ status: 'unavailable' }, { status: 'failed' }]), 'failed');
  assert.equal(finalReportStatus([{ status: 'passed' }, { status: 'unverified' }]), 'unverified');
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
