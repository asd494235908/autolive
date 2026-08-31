import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const candidateUrl = new URL(
  './shader-candidates/gpu83-image-repair-candidate.hook',
  import.meta.url,
);

test('图像修复候选保持中性、固定五采样和完整强度的边缘感知混合', async () => {
  const source = await readFile(candidateUrl, 'utf8');
  const code = source.replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');

  assert.match(source, /\/\/!PARAM al_image_repair_enabled[\s\S]*?\n0\.0\s*$/m);
  assert.match(source, /\/\/!PARAM al_image_repair_strength_percent[\s\S]*?\n0\.0\s*$/m);
  assert.match(source, /^\/\/!WHEN al_image_repair_enabled al_image_repair_strength_percent \*\s*$/m);
  assert.match(code, /if\s*\(\s*repair_mix\s*<=\s*0\.0\s*\)\s*\{\s*return source_color\s*;/s);
  assert.equal([...code.matchAll(/\bHOOKED_tex\s*\(/g)].length, 5,
    '每像素必须固定为中心点加十字邻域共五次采样');
  assert.doesNotMatch(code, /\b(?:for|while)\s*\(/);
  assert.doesNotMatch(code, /\b(?:fract|random|noise|hash)\b/i);

  assert.match(code,
    /repair_mix\s*=\s*clamp\(al_image_repair_enabled,\s*0\.0,\s*1\.0\)\s*\*\s*clamp\(al_image_repair_strength_percent\s*\/\s*100\.0,\s*0\.0,\s*1\.0\)\s*;/s,
    'strength=100 必须达到完整混合，不能再乘额外衰减系数');
  assert.equal([...code.matchAll(/1\.0\s*\/\s*\(1\.0\s*\+\s*64\.0\s*\*\s*dot\(/g)].length, 4,
    '四个邻点必须继续按与中心像素的颜色距离进行边缘感知降权');
  assert.match(code,
    /repaired_color\s*=\s*\([\s\S]*source_color\.rgb[\s\S]*left\s*\*\s*left_weight[\s\S]*right\s*\*\s*right_weight[\s\S]*up\s*\*\s*up_weight[\s\S]*down\s*\*\s*down_weight[\s\S]*\)\s*\/\s*weight_sum\s*;/);
  assert.match(code,
    /return\s+vec4\(mix\(source_color\.rgb,\s*repaired_color,\s*repair_mix\),\s*source_color\.a\)\s*;/,
    '输出只能混合原色与边缘感知修复结果，并原样保留 alpha');

  const center = 0.5;
  const neighbor = 0.515;
  const weight = 1 / (1 + 64 * 3 * ((neighbor - center) ** 2));
  const repaired = (center + 4 * neighbor * weight) / (1 + 4 * weight);
  const codeValue = (value) => Math.round(value * 255);
  assert.ok(codeValue(repaired) - codeValue(center) >= 2,
    '完整强度对轻微局部缺陷应产生至少两档 8-bit 响应');
  assert.ok(codeValue(center + (repaired - center) * 0.35) - codeValue(center) <= 1,
    '旧 0.35 衰减会复现 v3 的单 code-value 失败');
});
