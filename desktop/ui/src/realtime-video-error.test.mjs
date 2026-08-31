import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadErrorPolicy() {
  const source = await readFile(new URL('./realtime-video-error.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('实时视频计划和后端 epoch 过期都属于可恢复竞态', async () => {
  const policy = await loadErrorPolicy();
  const recoverable = [
    { code: 'stale_realtime_video_plan', message: '实时画面计划绑定的播放代次已过期' },
    { code: 'realtime_video_prepare_stale', message: '实时画面准备操作已过期' },
    { code: 'stale_realtime_video_backend_epoch', message: '实时画面准备请求的后端 epoch 已过期' },
  ];

  for (const cause of recoverable) {
    assert.equal(policy.isRecoverableRealtimeVideoError(cause), true, cause.code);
    assert.equal(policy.isPermanentRealtimeVideoError(cause), false, cause.code);
  }
});
