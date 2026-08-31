import assert from 'node:assert/strict';
import test from 'node:test';
import ts from 'typescript';
import fs from 'node:fs';
import vm from 'node:vm';

const source = fs.readFileSync(new URL('../src/realtime-video-error.ts', import.meta.url), 'utf8');
const javascript = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const module = { exports: {} };
vm.runInNewContext(javascript, { module, exports: module.exports, Set, Error });
const { classifyRealtimeVideoSyncError, isPermanentRealtimeVideoError } = module.exports;

test('确定性实时视频错误不会进入自动重试', () => {
  assert.equal(isPermanentRealtimeVideoError({ code: 'gpu83_parameters_unavailable', message: 'x' }), true);
  assert.equal(isPermanentRealtimeVideoError({ code: 'realtime_video_sync_invalid', message: 'x' }), true);
  assert.equal(isPermanentRealtimeVideoError({ code: 'original_video_playback_inactive', message: 'x' }), true);
  assert.equal(isPermanentRealtimeVideoError({ code: 'realtime_video_commit_failed', message: '实时画面同步的待提交计划已过期' }), true);
});

test('同步错误按 stale、busy、result unknown、transport 和 permanent 分类', () => {
  assert.equal(classifyRealtimeVideoSyncError({ code: 'stale_realtime_video_sync' }), 'stale');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_prepare_stale' }), 'stale');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_sync_busy' }), 'busy');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_sync_result_unknown' }), 'result_unknown');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_sync_transport_failed' }), 'transport');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_sync_invalid' }), 'permanent');
  assert.equal(classifyRealtimeVideoSyncError({ code: 'realtime_video_sync_superseded' }), 'superseded');
});

test('进程或 IPC 瞬时错误仍允许退避重试', () => {
  assert.equal(isPermanentRealtimeVideoError({ code: 'realtime_video_prepare_failed', message: '命名管道暂不可用' }), false);
  assert.equal(isPermanentRealtimeVideoError({ code: 'original_video_renderer_failed', message: '命名管道暂不可用' }), false);
  assert.equal(isPermanentRealtimeVideoError(new Error('mpv 进程异常退出')), false);
});

test('prepare 后端 epoch 竞态即使带已过期文案也允许恢复', () => {
  assert.equal(isPermanentRealtimeVideoError({
    code: 'stale_realtime_video_backend_epoch',
    message: '实时画面同步的准备后端 epoch 已过期',
  }), false);
});
