import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { compareRawRgbPairs } from './mpv-frame-difference.mjs';

const spatialUrl = new URL(
  './shader-candidates/gpu83-spatial-modulation-candidate.hook',
  import.meta.url,
);
const schedulerUrl = new URL(
  './shader-candidates/gpu83-scheduler-gates-candidate.hook',
  import.meta.url,
);

function shaderCode(source) {
  return source.replace(/^\/\/!.*$/gm, '').replace(/\/\/.*$/gm, '');
}

function floatConstant(source, name) {
  const match = source.match(new RegExp(`const\\s+float\\s+${name}\\s*=\\s*([0-9.]+)\\s*;`));
  assert.ok(match, `${name} 必须显式声明有限常量`);
  return Number(match[1]);
}

function codeDelta(source, name, minimum = 2, maximum = 4) {
  const value = floatConstant(source, name);
  assert.ok(value >= minimum && value <= maximum,
    `${name} 必须在 ${minimum}–${maximum} 个码值内`);
  return value;
}

function synthesizeFrameInnerArea(maskEdge, codeValueDelta) {
  const sourceWidth = 1920;
  const sourceHeight = 1080;
  const analysisWidth = 320;
  const analysisHeight = 180;
  const scaleX = sourceWidth / analysisWidth;
  const scaleY = sourceHeight / analysisHeight;
  assert.equal(scaleX, 6);
  assert.equal(scaleY, 6);
  const baseline = Buffer.alloc(analysisWidth * analysisHeight * 3, 128);
  const active = Buffer.alloc(baseline.length);

  for (let outputY = 0; outputY < analysisHeight; outputY += 1) {
    for (let outputX = 0; outputX < analysisWidth; outputX += 1) {
      let sourceDelta = 0;
      for (let offsetY = 0; offsetY < scaleY; offsetY += 1) {
        for (let offsetX = 0; offsetX < scaleX; offsetX += 1) {
          const sourceX = outputX * scaleX + offsetX;
          const sourceY = outputY * scaleY + offsetY;
          if (sourceX % 8 >= maskEdge || sourceY % 8 >= maskEdge) continue;
          const sign = (Math.floor(sourceX / 8) + Math.floor(sourceY / 8)) % 2 === 0 ? -1 : 1;
          sourceDelta += sign * codeValueDelta;
        }
      }
      const value = Math.round(128 + sourceDelta / (scaleX * scaleY));
      active.fill(value, (outputY * analysisWidth + outputX) * 3,
        (outputY * analysisWidth + outputX + 1) * 3);
    }
  }
  return Buffer.concat([baseline, active]);
}

test('wave level 保留覆盖概率语义并产生有界可测响应', async () => {
  const source = await readFile(spatialUrl, 'utf8');
  const code = shaderCode(source);

  assert.match(source, /\/\/!PARAM al_wave_level\s+\/\/!TYPE float\s+\/\/!MINIMUM 0\.0\s+\/\/!MAXIMUM 1\.0\s+0\.0/s);
  assert.ok(codeDelta(code, 'AL_WAVE_LEVEL_CODE_DELTA') >= 2);
  assert.match(code, /al_same_frame_grain\s*\([^)]*\)\s*<\s*al_wave_level/);
  assert.match(code, /luminance_delta\s*\+=\s*AL_WAVE_LEVEL_CODE_DELTA\s*;/);
  assert.match(code, /return\s+vec4\s*\(\s*modulated_rgb\s*,\s*source_color\.a\s*\)\s*;/);
  assert.equal([...code.matchAll(/\bHOOKED_tex\s*\(/g)].length, 1);
  assert.doesNotMatch(code, /\b(?:for|while)\s*\(|\b(?:PTS|TIME|clock|history|audio)\b/i);
});

test('frame inner 保留稀疏同帧掩码并产生有界可测响应', async () => {
  const source = await readFile(schedulerUrl, 'utf8');
  const code = shaderCode(source);

  assert.match(source, /\/\/!PARAM al_runtime_frame_inner_active\s+\/\/!TYPE float\s+\/\/!MINIMUM 0\.0\s+\/\/!MAXIMUM 1\.0\s+0\.0/s);
  assert.equal(codeDelta(code, 'AL_FRAME_INNER_CODE_DELTA', 16, 16), 16);
  assert.equal(floatConstant(code, 'AL_FRAME_INNER_MASK_EDGE'), 2);
  assert.match(code, /inner_mask\s*=\s*mod\s*\(\s*pixel\.x\s*,\s*8\.0\s*\)\s*<\s*AL_FRAME_INNER_MASK_EDGE\s*&&\s*mod\s*\(\s*pixel\.y\s*,\s*8\.0\s*\)\s*<\s*AL_FRAME_INNER_MASK_EDGE/);
  assert.match(code, /inner_delta\s*=\s*inner_gate\s*\*\s*inner_mask\s*\*\s*signed_pattern\s*\*\s*\(\s*AL_FRAME_INNER_CODE_DELTA\s*\/\s*255\.0\s*\)\s*;/);
  assert.match(code, /if\s*\(\s*!has_luma_effect\s*&&\s*inter_mix\s*<=\s*0\.0\s*&&\s*!has_transform\s*\)\s*\{\s*return\s+source_color\s*;/s);
  assert.match(code, /return\s+vec4\s*\(\s*result\s*,\s*source_color\.a\s*\)\s*;/);
  assert.equal([...code.matchAll(/\bHOOKED_tex\s*\(/g)].length, 3);
  assert.doesNotMatch(code, /\b(?:for|while)\s*\(|\b(?:PTS|TIME|clock|history|audio)\b/i);
});

test('frame inner 经过 1920×1080 到 320×180 area 下采样仍跨过两码值门槛', async () => {
  const code = shaderCode(await readFile(schedulerUrl, 'utf8'));
  const codeValueDelta = codeDelta(code, 'AL_FRAME_INNER_CODE_DELTA', 16, 16);
  const maskEdge = floatConstant(code, 'AL_FRAME_INNER_MASK_EDGE');
  const [oldSparse] = compareRawRgbPairs(
    synthesizeFrameInnerArea(1, codeValueDelta), ['frame_inner_old_sparse'],
  );
  const [current] = compareRawRgbPairs(
    synthesizeFrameInnerArea(maskEdge, codeValueDelta), ['frame_inner'],
  );

  assert.equal(oldSparse.status, 'failed');
  assert.equal(oldSparse.changedPixels, 0);
  assert.equal(current.status, 'passed');
  assert.equal(current.changedPixelRatio, 0.5625);
  assert.equal(current.maximumChannelDelta, 2);
});
