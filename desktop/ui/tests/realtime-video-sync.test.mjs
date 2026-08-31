import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import * as ts from 'typescript';

const source = fs.readFileSync(new URL('../src/realtime-video-sync.ts', import.meta.url), 'utf8');
const javascript = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2021,
  },
}).outputText;
const sync = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(javascript)}`);

const clock = {
  playback_generation: 12,
  clock_epoch: 4,
  loop_index: 3,
  position_ms: 8_500,
};

test('视频状态代次落后时不构造 sync IPC', () => {
  assert.equal(sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 11,
    backend_epoch: 9,
  }, false), null);
});

test('同代次 sync 使用同一个后端状态的 epoch', () => {
  assert.deepEqual(sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, true), {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
    position_ms: 8_500,
    paused: true,
  });
});

test('拖拽 seek 使用严格递增 epoch，并把末尾目标限制在有效 mpv PTS 内', () => {
  assert.equal(sync.nextRealtimeVideoSeekClockEpoch(4, 6, 5), 7);
  assert.equal(sync.clampRealtimeVideoSeekPositionMs(12, 12_000), 11_999);
  assert.equal(sync.clampRealtimeVideoSeekPositionMs(3.456, 12_000), 3_456);
  assert.equal(sync.clampRealtimeVideoSeekPositionMs(-1, 12_000), 0);
});

test('sync 应用确认必须验证 mpv 物理播放态、EOF 已清除且 PTS 有效', () => {
  const request = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false);
  const status = {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
    activation: 'active',
    process_id: 42,
    physical_paused: false,
    eof: null,
    presented_pts_ms: 8_500,
  };

  assert.equal(sync.isRealtimeVideoSyncApplied(status, request), true);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, physical_paused: true }, request), false);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, eof: {} }, request), false);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, presented_pts_ms: null }, request), false);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, process_id: null }, request), false);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, process_id: 0 }, request), false);
});

test('seek sync 只有在同一身份的 mpv 已呈现 PTS 接近目标时才算应用', () => {
  const request = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false);
  const status = {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
    activation: 'active',
    process_id: 42,
    physical_paused: false,
    eof: null,
    presented_pts_ms: 8_500,
  };

  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, presented_pts_ms: 8_750 }, request), true);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, presented_pts_ms: 7_000 }, request), false);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, clock_epoch: 3 }, request), false);
  assert.equal(sync.realtimeVideoSyncPositionToleranceMs(60), 350);
  assert.equal(sync.realtimeVideoSyncPositionToleranceMs(22), 387);
});

test('暂停 sync 只有在 mpv 真实暂停后才算应用', () => {
  const request = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, true);
  const status = {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
    activation: 'active',
    process_id: 42,
    physical_paused: true,
    eof: null,
    presented_pts_ms: 8_500,
  };

  assert.equal(sync.isRealtimeVideoSyncApplied(status, request), true);
  assert.equal(sync.isRealtimeVideoSyncApplied({ ...status, physical_paused: false }, request), false);
});

test('WebView 不支持源封装时采用同身份 mpv 已呈现 PTS', () => {
  const backend = {
    playback_generation: 12,
    clock_epoch: 4,
    loop_index: 3,
    activation: 'available',
    process_id: 42,
    presented_pts_ms: 8_750,
  };
  assert.equal(sync.managedVideoPresentedPosition(backend, clock, 10_000), 8_750);
  assert.equal(sync.managedVideoPresentedPosition({ ...backend, playback_generation: 11 }, clock, 10_000), null);
  assert.equal(sync.managedVideoPresentedPosition({ ...backend, process_id: null }, clock, 10_000), null);
  assert.equal(sync.managedVideoPresentedPosition({ ...backend, presented_pts_ms: 12_000 }, clock, 10_000), 10_000);
});

test('stale WebView 时钟不参与 commit，使用同身份 mpv 物理 PTS 生成绝对位置', () => {
  const expected = {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
  };
  const status = {
    ...expected,
    activation: 'active',
    process_id: 42,
    physical_paused: false,
    eof: null,
    presented_pts_ms: 8_500,
  };

  assert.deepEqual(sync.buildRealtimeVideoPhysicalCommitPosition(status, expected, 10_000), {
    position_ms: 8_500,
    absolute_position_ms: 38_500,
  });
});

test('物理 commit 位置拒绝身份错配、非法会话和旧 candidate 的迟到状态', () => {
  const expected = {
    playback_generation: 13,
    backend_epoch: 10,
    clock_epoch: 5,
    loop_index: 0,
  };
  const status = {
    ...expected,
    activation: 'available',
    process_id: 43,
    physical_paused: false,
    eof: null,
    presented_pts_ms: 6_900,
  };

  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, playback_generation: 12 }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, backend_epoch: 9 }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, clock_epoch: 4 }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, loop_index: 1 }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, process_id: null }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, physical_paused: true }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, eof: {} }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, presented_pts_ms: 10_000 }, expected, 10_000), null);
  assert.equal(sync.buildRealtimeVideoPhysicalCommitPosition({ ...status, presented_pts_ms: 10_001 }, expected, 10_000), null);
});

test('GPU 降级恢复只在同代稳定 owner 的 epoch、进程或 launch mode 变化后重建流水线', () => {
  const gpu = {
    playback_generation: 12,
    backend_epoch: 9,
    backend: 'realtime_gpu',
    activation: 'active',
    process_id: 42,
    fallback_floor_mode: 'gpu_d3d11_copy',
    n: { sequence: 6 },
    n1: { sequence: 7 },
    n2: { sequence: 8 },
  };
  const owner = sync.stableRealtimeVideoPipelineOwner(gpu);

  assert.notEqual(owner, null);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(null, gpu), null);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(owner, gpu), null);
  assert.match(sync.realtimeVideoPipelineRecoveryKey(owner, {
    ...gpu,
    backend_epoch: 10,
  }), /^pipeline:/);
  assert.match(sync.realtimeVideoPipelineRecoveryKey(owner, {
    ...gpu,
    process_id: 43,
  }), /^pipeline:/);
  assert.match(sync.realtimeVideoPipelineRecoveryKey(owner, {
    ...gpu,
    backend: 'cpu4',
    fallback_floor_mode: 'cpu4',
  }), /^pipeline:/);
  const transitionedOrphan = {
    ...gpu,
    backend_epoch: 10,
    process_id: 43,
    fallback_floor_mode: 'gpu_vulkan_copy',
    n1: null,
    n2: { sequence: 7 },
  };
  const transitionRecoveryKey = sync.realtimeVideoPipelineRecoveryKey(owner, transitionedOrphan);
  assert.equal(
    sync.realtimeVideoPipelineRecoveryKey(
      sync.stableRealtimeVideoPipelineOwner(transitionedOrphan),
      transitionedOrphan,
    ),
    transitionRecoveryKey,
  );
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(owner, {
    ...gpu,
    activation: 'configured',
    process_id: null,
  }), null);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(owner, {
    ...gpu,
    playback_generation: 13,
    backend_epoch: 1,
    process_id: 44,
  }), null);
});

test('同 epoch 的 N+1 缺失必须先建立接受基线，且正常 N1 提交不误清流水线', () => {
  const orphan = {
    playback_generation: 12,
    backend_epoch: 10,
    backend: 'realtime_gpu',
    activation: 'active',
    process_id: 43,
    fallback_floor_mode: 'gpu_vulkan_copy',
    n: { sequence: 6 },
    n1: null,
    n2: { sequence: 7 },
  };
  const orphanBaseline = sync.stableRealtimeVideoPipelineOwner(orphan);
  const submitted = {
    ...orphan,
    n1: { sequence: 7 },
    n2: { sequence: 8 },
  };
  const submittedBaseline = sync.stableRealtimeVideoPipelineOwner(submitted);

  assert.equal(sync.realtimeVideoPipelineRecoveryKey(null, orphan), null);
  assert.match(sync.realtimeVideoPipelineRecoveryKey(orphanBaseline, orphan), /^pipeline:/);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(submittedBaseline, submitted), null);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(submittedBaseline, {
    ...orphan,
    n: { sequence: 7 },
    n2: { sequence: 8 },
  }), null);
  assert.equal(sync.realtimeVideoPipelineRecoveryKey(orphanBaseline, {
    ...orphan,
    n2: null,
  }), null);
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

test('latest-wins 只提交最新 desired，旧响应不得更新状态', async () => {
  const first = deferred();
  const second = deferred();
  const invoked = [];
  const accepted = [];
  const coordinator = sync.createRealtimeVideoSyncCoordinator({
    invokeSync: (request) => {
      invoked.push(request);
      return invoked.length === 1 ? first.promise : second.promise;
    },
    readStatus: async () => null,
    refreshLatestDesired: async () => null,
    isApplied: (status) => status.id === 'latest',
    classifyError: () => 'transport',
    onStatus: (status) => accepted.push(status),
    onFailure: () => {},
  });
  const firstRequest = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false);
  const secondRequest = { ...firstRequest, loop_index: 4, position_ms: 250 };

  coordinator.submit(firstRequest);
  coordinator.submit(secondRequest);
  first.resolve({ id: 'old' });
  await new Promise(setImmediate);
  assert.equal(invoked.length, 2);
  assert.deepEqual(accepted, []);

  second.resolve({ id: 'latest' });
  await new Promise(setImmediate);
  assert.deepEqual(accepted, [{ id: 'latest' }]);
  coordinator.dispose();
});

test('result_unknown 先读取状态确认，已应用时不重复 sync', async () => {
  let invokeCount = 0;
  let readCount = 0;
  const accepted = [];
  const request = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false);
  const currentStatus = { id: 'applied' };
  const coordinator = sync.createRealtimeVideoSyncCoordinator({
    invokeSync: async () => {
      invokeCount += 1;
      throw { code: 'realtime_video_sync_result_unknown', message: 'response lost' };
    },
    readStatus: async () => {
      readCount += 1;
      return currentStatus;
    },
    refreshLatestDesired: async () => null,
    isApplied: (status) => status === currentStatus,
    classifyError: (cause) => cause.code === 'realtime_video_sync_result_unknown'
      ? 'result_unknown'
      : 'permanent',
    onStatus: (status) => accepted.push(status),
    onFailure: () => {},
  });

  coordinator.submit(request);
  await new Promise(setImmediate);
  assert.equal(readCount, 1);
  assert.equal(invokeCount, 1);
  assert.deepEqual(accepted, [currentStatus]);
  coordinator.dispose();
});

test('sync IPC 正常返回但物理 PTS 未到目标时不得结算', async () => {
  const accepted = [];
  const failures = [];
  const request = sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false);
  const unconfirmed = {
    playback_generation: 12,
    backend_epoch: 9,
    clock_epoch: 4,
    loop_index: 3,
    activation: 'active',
    process_id: 42,
    physical_paused: false,
    eof: null,
    presented_pts_ms: 1_000,
  };
  const coordinator = sync.createRealtimeVideoSyncCoordinator({
    invokeSync: async () => unconfirmed,
    readStatus: async () => unconfirmed,
    refreshLatestDesired: async () => null,
    isApplied: sync.isRealtimeVideoSyncApplied,
    classifyError: () => 'permanent',
    onStatus: (status) => accepted.push(status),
    onFailure: (_cause, disposition) => failures.push(disposition),
    retryDelaysMs: [],
  });

  coordinator.submit(request);
  await new Promise(setImmediate);
  assert.deepEqual(accepted, []);
  assert.deepEqual(failures, ['result_unknown']);
  coordinator.dispose();
});

test('dispose 清理 busy 重试并忽略迟到响应', async () => {
  let invokeCount = 0;
  let accepted = 0;
  const coordinator = sync.createRealtimeVideoSyncCoordinator({
    invokeSync: async () => {
      invokeCount += 1;
      throw { code: 'realtime_video_sync_busy', message: 'busy' };
    },
    readStatus: async () => null,
    refreshLatestDesired: async () => null,
    isApplied: () => false,
    classifyError: (cause) => cause.code === 'realtime_video_sync_busy' ? 'busy' : 'permanent',
    onStatus: () => { accepted += 1; },
    onFailure: () => {},
    retryDelaysMs: [20, 20, 20],
  });

  coordinator.submit(sync.buildRealtimeVideoSyncRequest(clock, {
    playback_generation: 12,
    backend_epoch: 9,
  }, false));
  await new Promise(setImmediate);
  coordinator.dispose();
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(invokeCount, 1);
  assert.equal(accepted, 0);
});
