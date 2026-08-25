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

const {
  buildFinalEffectMediaIdentity,
  buildFinalEffectWindowResizeKey,
  isFinalEffectMediaIdentityCurrent,
} = await loadTypeScriptModule('final-effect-window-size.ts', [
  'buildFinalEffectMediaIdentity',
  'buildFinalEffectWindowResizeKey',
  'isFinalEffectMediaIdentityCurrent',
]);

test('缺少有效视频尺寸时不产生调整 key', () => {
  const identity = { playbackGeneration: 1, sourcePath: '/tmp/a.mp4' };
  assert.equal(buildFinalEffectWindowResizeKey({ ...identity, width: null, height: 720 }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ ...identity, width: 1280, height: 0 }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ ...identity, width: 1280.5, height: 720 }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ ...identity, playbackGeneration: null, width: 1280, height: 720 }), null);
  assert.equal(buildFinalEffectWindowResizeKey({ ...identity, sourcePath: '', width: 1280, height: 720 }), null);
});

test('相同分辨率产生稳定 key，快照轮询不会重复调整', () => {
  const input = {
    playbackGeneration: 1,
    width: 1280,
    height: 720,
    sourcePath: '/tmp/a.mp4',
    videoReference: '/tmp/a.mp4',
  };
  assert.equal(buildFinalEffectWindowResizeKey(input), buildFinalEffectWindowResizeKey(input));
});

test('processed 路径变化不产生新 key，播放代次、源文件或分辨率变化才产生', () => {
  const base = {
    playbackGeneration: 1,
    width: 1280,
    height: 720,
    sourcePath: '/tmp/a.mp4',
    videoReference: '/tmp/a.mp4',
  };
  assert.equal(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, videoReference: '/tmp/b.mp4' }),
  );
  assert.notEqual(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, playbackGeneration: 2 }),
  );
  assert.notEqual(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, sourcePath: '/tmp/b.mp4' }),
  );
  assert.notEqual(
    buildFinalEffectWindowResizeKey(base),
    buildFinalEffectWindowResizeKey({ ...base, width: 1920 }),
  );
});

test('544x960 切换到 720x1280 时旧 metadata 与旧 resize 身份均失效', () => {
  const first = {
    playbackGeneration: 7,
    sourceIdentity: 'asset://localhost/first.mp4',
  };
  const second = {
    playbackGeneration: 8,
    sourceIdentity: 'asset://localhost/second.mp4',
  };
  const firstIdentity = buildFinalEffectMediaIdentity(first);
  const secondIdentity = buildFinalEffectMediaIdentity(second);

  assert.notEqual(firstIdentity, secondIdentity);
  assert.equal(isFinalEffectMediaIdentityCurrent(firstIdentity, second), false);
  assert.equal(isFinalEffectMediaIdentityCurrent(secondIdentity, second), true);
  assert.notEqual(
    buildFinalEffectWindowResizeKey({
      playbackGeneration: first.playbackGeneration,
      sourcePath: 'D:/video/first.mp4',
      width: 544,
      height: 960,
    }),
    buildFinalEffectWindowResizeKey({
      playbackGeneration: second.playbackGeneration,
      sourcePath: 'D:/video/second.mp4',
      width: 720,
      height: 1280,
    }),
  );
});

test('相同尺寸换源也会产生新 resize key，迟到旧操作不能提交', () => {
  const first = {
    playbackGeneration: 21,
    sourceIdentity: 'asset://localhost/first.mp4',
  };
  const second = {
    playbackGeneration: 22,
    sourceIdentity: 'asset://localhost/second.mp4',
  };
  const firstIdentity = buildFinalEffectMediaIdentity(first);

  assert.equal(isFinalEffectMediaIdentityCurrent(firstIdentity, second), false);
  assert.notEqual(
    buildFinalEffectWindowResizeKey({
      playbackGeneration: first.playbackGeneration,
      sourcePath: 'D:/video/first.mp4',
      width: 720,
      height: 1280,
    }),
    buildFinalEffectWindowResizeKey({
      playbackGeneration: second.playbackGeneration,
      sourcePath: 'D:/video/second.mp4',
      width: 720,
      height: 1280,
    }),
  );
});
