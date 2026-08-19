import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function loadTypeScriptModule(fileName, exports) {
  const source = await readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  const module = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
  return Object.fromEntries(exports.map((name) => [name, module[name]]));
}

const { buildFinalEffectWindowResizeKey } = await loadTypeScriptModule('final-effect-window-size.ts', [
  'buildFinalEffectWindowResizeKey',
]);

test('缺少有效视频尺寸时不产生调整 key', () => {
  assert.equal(buildFinalEffectWindowResizeKey({ width: null, height: 720, sourcePath: '/tmp/a.mp4' }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ width: 1280, height: 0, sourcePath: '/tmp/a.mp4' }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ width: 1280.5, height: 720, sourcePath: '/tmp/a.mp4' }), null);
});

test('相同分辨率产生稳定 key，快照轮询不会重复调整', () => {
  const input = { width: 1280, height: 720, sourcePath: '/tmp/a.mp4', videoReference: '/tmp/a.mp4' };
  assert.equal(buildFinalEffectWindowResizeKey(input), buildFinalEffectWindowResizeKey(input));
});

test('processed 路径变化不产生新 key，分辨率变化才产生', () => {
  const base = { width: 1280, height: 720, sourcePath: '/tmp/a.mp4', videoReference: '/tmp/a.mp4' };
  assert.equal(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, videoReference: '/tmp/b.mp4' }),
  );
  assert.notEqual(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, width: 1920 }),
  );
});
