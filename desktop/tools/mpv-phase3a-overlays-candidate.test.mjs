import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { parseShaderParameters } from './verify-mpv-phase1.mjs';

const CANDIDATE_URL = new URL(
  './shader-candidates/gpu83-same-frame-overlays-candidate.hook',
  import.meta.url,
);

const EXPECTED_PARAMETERS = Object.freeze([
  { name: 'al_random_graphic_enabled', minimum: 0, maximum: 1, defaultValue: 0 },
  { name: 'al_random_graphic_count', minimum: 1, maximum: 32, defaultValue: 4 },
  { name: 'al_random_graphic_opacity_percent', minimum: 0, maximum: 50, defaultValue: 0 },
  { name: 'al_random_graphic_size_px', minimum: 1, maximum: 64, defaultValue: 4 },
  { name: 'al_overlay_offset_px', minimum: -10, maximum: 10, defaultValue: 0 },
  { name: 'al_pip_enabled', minimum: 0, maximum: 1, defaultValue: 0 },
  { name: 'al_pip_scale_percent', minimum: 10, maximum: 50, defaultValue: 24 },
  { name: 'al_pip_opacity_percent', minimum: 0, maximum: 100, defaultValue: 100 },
  { name: 'al_pip_rotation_degrees', minimum: -15, maximum: 15, defaultValue: 0 },
  { name: 'al_runtime_pip_jitter_x_px', minimum: -4, maximum: 4, defaultValue: 0 },
  { name: 'al_runtime_pip_jitter_y_px', minimum: -4, maximum: 4, defaultValue: 0 },
  { name: 'al_runtime_random_graphic_seed', minimum: 0, maximum: 16777215, defaultValue: 0 },
]);

function parametersWithDefaults(source) {
  const parsed = parseShaderParameters(source);
  const declarations = [...source.matchAll(/^\/\/!PARAM\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/gm)];
  assert.deepEqual(declarations.map((match) => match[1]), parsed.map(({ name }) => name),
    '不得声明契约外参数');
  return parsed.map((parameter, index) => {
    const start = declarations[index].index + declarations[index][0].length;
    const end = declarations[index + 1]?.index ?? source.indexOf('//!HOOK', start);
    const block = source.slice(start, end);
    assert.match(block, /^\/\/!TYPE\s+float\s*$/m, `${parameter.name} 必须是 float 参数`);
    const values = block.split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith('//!'));
    assert.equal(values.length, 1, `${parameter.name} 必须只有一个默认值`);
    const defaultValue = Number(values[0]);
    assert.ok(Number.isFinite(defaultValue), `${parameter.name} 默认值必须是有限数`);
    return { ...parameter, defaultValue };
  });
}

function functionBody(source, name) {
  const declaration = new RegExp(`\\bvec4\\s+${name}\\s*\\(\\s*\\)\\s*\\{`).exec(source);
  assert.ok(declaration, `缺少 vec4 ${name}()`);
  const open = source.indexOf('{', declaration.index);
  let depth = 1;
  for (let index = open + 1; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, index);
  }
  assert.fail(`${name}() 花括号不完整`);
}

function matchingBrace(source, open) {
  let depth = 1;
  for (let index = open + 1; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) return index;
  }
  return -1;
}

test('同帧叠加候选固定为 9 项语义参数与 3 项本地 runtime 选项', async () => {
  const source = await readFile(CANDIDATE_URL, 'utf8');
  assert.deepEqual(parametersWithDefaults(source), EXPECTED_PARAMETERS);
  assert.doesNotMatch(source, /^\/\/!PARAM\s+al_pip_(?:jitter_px|timeline_locked)\s*$/m);
});

test('Phase 3A 同帧叠加候选是单一、无时序依赖的普通 fragment pass', async () => {
  const source = await readFile(CANDIDATE_URL, 'utf8');
  assert.deepEqual([...source.matchAll(/^\/\/!HOOK\s+(\S+)\s*$/gm)].map((match) => match[1]), ['MAIN']);
  assert.deepEqual([...source.matchAll(/^\/\/!BIND\s+(\S+)\s*$/gm)].map((match) => match[1]), ['HOOKED']);
  assert.deepEqual([...source.matchAll(/^\/\/!WIDTH\s+(.+)$/gm)].map((match) => match[1]), ['HOOKED.w']);
  assert.deepEqual([...source.matchAll(/^\/\/!HEIGHT\s+(.+)$/gm)].map((match) => match[1]), ['HOOKED.h']);
  assert.equal([...source.matchAll(/\bvec4\s+hook\s*\(\s*\)\s*\{/g)].length, 1);
  assert.doesNotMatch(source, /\b(?:SAVE|COMPUTE)\b/);

  const when = /^\/\/!WHEN\s+(.+)$/m.exec(source)?.[1];
  assert.ok(when, '候选必须声明动态 WHEN');
  assert.match(when, /\bal_random_graphic_enabled\s+al_random_graphic_opacity_percent\s+\*(?!\S)/);
  assert.match(when, /\bal_pip_enabled\s+al_pip_opacity_percent\s+\*(?!\S)/);

  const shaderCode = source.replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
  assert.doesNotMatch(shaderCode, /\b(?:PTS|TIME|frame|history|audio)\b/i);
  assert.doesNotMatch(shaderCode, /\bfor\s*\(/);
});

test('随机图形使用 8x4 直接槽位与拆分后的计划级确定性 seed', async () => {
  const source = await readFile(CANDIDATE_URL, 'utf8');
  const body = functionBody(source, 'hook');
  const grid = /\bvec2\s+([A-Za-z_]\w*)\s*=\s*vec2\s*\(\s*8(?:\.0+)?\s*,\s*4(?:\.0+)?\s*\)\s*;/.exec(body);
  assert.ok(grid, '随机图形必须固定划分为 8x4 槽位');
  const cell = new RegExp(`\\bvec2\\s+([A-Za-z_]\\w*)\\s*=\\s*floor\\s*\\([^;]*\\b${grid[1]}\\b[^;]*\\)\\s*;`).exec(body);
  assert.ok(cell, '当前像素必须直接映射到一个槽位');
  assert.match(
    body,
    new RegExp(`\\bint\\s+[A-Za-z_]\\w*\\s*=\\s*int\\s*\\(\\s*${cell[1]}\\.y\\s*\\)\\s*\\*\\s*8\\s*\\+\\s*int\\s*\\(\\s*${cell[1]}\\.x\\s*\\)\\s*;`),
  );
  assert.match(body, /\bclamp\s*\(\s*floor\s*\(\s*al_random_graphic_count\b[^)]*\)\s*,\s*1(?:\.0+)?\s*,\s*32(?:\.0+)?\s*\)/);
  assert.match(body, /\bal_random_graphic_size_px\b/);
  assert.match(body, /\bal_overlay_offset_px\b[^;]*\bHOOKED_pt\b/);
  const graphicBranch = body.slice(body.indexOf('float graphic_weight'));
  assert.match(graphicBranch,
    /\bfloat\s+seed\s*=\s*floor\s*\(\s*clamp\s*\(\s*al_runtime_random_graphic_seed\s*,\s*0\.0\s*,\s*16777215\.0\s*\)\s*\)\s*;/);
  assert.doesNotMatch(graphicBranch, /\bal_runtime_random_graphic_seed\s*\+\s*0\.5\b/,
    '24 位整数 seed 不得先加 0.5，以免 f32 高位发生相邻整数舍入');
  assert.match(graphicBranch,
    /\bvec2\s+seed_parts\s*=\s*vec2\s*\(\s*mod\s*\(\s*seed\s*,\s*4096\.0\s*\)\s*,\s*floor\s*\(\s*seed\s*\/\s*4096\.0\s*\)\s*\)\s*;/,
    '24 位 seed 必须先拆成低/高 12 位有界分量');
  assert.match(source,
    /\blow_component\s*=\s*al_overlay_hash\s*\(\s*base_input\s*\+\s*seed_parts\.x\s*\)\s*;/);
  assert.match(source,
    /\bhigh_component\s*=\s*al_overlay_hash\s*\(\s*base_input\s*\+\s*seed_parts\.y\s*\+\s*4096\.0\s*\)\s*;/);
  assert.match(source,
    /\bif\s*\(\s*seed_parts\.x\s*<=\s*0\.0\s*&&\s*seed_parts\.y\s*<=\s*0\.0\s*\)\s*\{\s*return\s+al_overlay_hash\s*\(\s*base_input\s*\)\s*;/s,
    '中性 seed=0 必须保留原槽位 hash');
  assert.doesNotMatch(graphicBranch, /\bal_runtime_random_graphic_seed\b\s*\*/,
    '不得把 24 位 seed 直接乘大常数');
  assert.deepEqual(
    [...graphicBranch.matchAll(/\bal_overlay_seeded_hash\s*\(\s*slot\s*,\s*(\d+\.0)\s*,\s*seed_parts\s*\)/g)]
      .map((match) => Number(match[1])),
    [1, 37, 73, 101, 149, 197],
    '位置、形状和三通道颜色必须使用 seed 参与的固定槽位 hash',
  );
  assert.doesNotMatch(body, /\bfor\s*\(/);
});

test('PIP 仅在区域内追加一次 HOOKED 采样并保留原 alpha', async () => {
  const body = functionBody(await readFile(CANDIDATE_URL, 'utf8'), 'hook');
  const samplers = [...body.matchAll(/\b([A-Za-z_]\w*)_tex\s*\(/g)];
  assert.deepEqual([...new Set(samplers.map((match) => match[1]))], ['HOOKED']);
  assert.equal(samplers.length, 2, '只能采样源像素和 PIP 区域像素各一次');
  assert.match(body, /\bHOOKED_tex\s*\(\s*HOOKED_pos\s*\)/);

  for (const parameter of [
    'al_pip_scale_percent',
    'al_pip_opacity_percent',
    'al_pip_rotation_degrees',
    'al_overlay_offset_px',
    'al_runtime_pip_jitter_x_px',
    'al_runtime_pip_jitter_y_px',
  ]) assert.match(body, new RegExp(`\\b${parameter}\\b`), `${parameter} 必须参与 PIP`);
  assert.match(body, /\bal_runtime_pip_jitter_x_px\b[^;]*\bal_runtime_pip_jitter_y_px\b[^;]*\bHOOKED_pt\b/s,
    'PIP runtime X/Y 必须在同一完整快照内转换为像素偏移');
  const graphicStart = body.indexOf('float graphic_weight');
  assert.notEqual(graphicStart, -1, '缺少随机图形分支');
  assert.doesNotMatch(body.slice(0, graphicStart), /\bal_runtime_random_graphic_seed\b|\bseed_parts\b/,
    '随机图形 seed 不得进入 PIP 路径');
  const graphicBranch = body.slice(graphicStart);
  assert.doesNotMatch(graphicBranch, /\bal_runtime_pip_jitter_[xy]_px\b/,
    'PIP runtime jitter 不得平移随机图形');

  const secondSample = samplers[1].index;
  const samplePosition = /^HOOKED_tex\s*\(\s*([A-Za-z_]\w*)\s*\)/.exec(body.slice(secondSample))?.[1];
  assert.ok(samplePosition, 'PIP 必须使用一个受控区域坐标采样');
  const regionGuardStart = body.search(new RegExp(
    `\\bif\\s*\\(\\s*all\\s*\\(\\s*greaterThanEqual\\s*\\(\\s*${samplePosition}\\b`,
  ));
  assert.notEqual(regionGuardStart, -1, '必须先确认 PIP 采样坐标位于有效区域');
  const regionOpen = body.indexOf('{', regionGuardStart);
  assert.ok(secondSample > regionOpen && secondSample < matchingBrace(body, regionOpen),
    '第二次采样必须位于 PIP 区域判断内部');

  const sourceColor = /\bvec4\s+([A-Za-z_]\w*)\s*=\s*HOOKED_tex\s*\(\s*HOOKED_pos\s*\)\s*;/.exec(body)?.[1];
  assert.ok(sourceColor, '必须保存源像素');
  assert.match(body, new RegExp(`\\breturn\\s+vec4\\s*\\([^;]*,\\s*${sourceColor}\\.a\\s*\\)\\s*;`));
});
