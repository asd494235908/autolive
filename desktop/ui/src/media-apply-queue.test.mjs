import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function loadQueueModule() {
  const source = await readFile(new URL('./media-apply-queue.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('实时提交在外层 apply 释放前推进下一轮时，释放后自动执行最新 pending', async () => {
  const { flushLatestPendingApply } = await loadQueueModule();
  const pendingRef = { current: null };
  const applied = [];
  let applyInFlight = false;
  let finishOuterApply;
  const outerCanFinish = new Promise((resolve) => {
    finishOuterApply = resolve;
  });
  let realtimeCommitted;
  const realtimeCommitDone = new Promise((resolve) => {
    realtimeCommitted = resolve;
  });
  const flush = () => flushLatestPendingApply(
    pendingRef,
    () => applyInFlight,
    async (candidate) => {
      applied.push(candidate.sequence);
    },
  );

  const outerApply = (async () => {
    applyInFlight = true;
    try {
      queueMicrotask(() => {
        pendingRef.current = { sequence: 2 };
        pendingRef.current = { sequence: 3 };
        realtimeCommitted();
      });
      await realtimeCommitDone;
      await outerCanFinish;
    } finally {
      applyInFlight = false;
      await flush();
    }
  })();

  await realtimeCommitDone;
  assert.deepEqual(applied, []);
  finishOuterApply();
  await outerApply;
  assert.deepEqual(applied, [3]);
  assert.equal(pendingRef.current, null);
});

test('暂停、停止或旧代次门禁阻止刷新时保留 pending，恢复后仍只执行最新项', async () => {
  const { flushLatestPendingApply } = await loadQueueModule();
  const pendingRef = { current: { sequence: 4 } };
  const applied = [];
  let blocked = true;

  assert.equal(await flushLatestPendingApply(
    pendingRef,
    () => blocked,
    async (candidate) => applied.push(candidate.sequence),
  ), false);
  pendingRef.current = { sequence: 5 };
  blocked = false;
  assert.equal(await flushLatestPendingApply(
    pendingRef,
    () => blocked,
    async (candidate) => applied.push(candidate.sequence),
  ), true);

  assert.deepEqual(applied, [5]);
  assert.equal(pendingRef.current, null);
});

test('App 在两种异步完成顺序都触发刷新，且不使用 React busy state 互斥', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const apply = app.slice(
    app.indexOf('async function applyVideoProcessing('),
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
  );
  const commit = app.slice(
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
    app.indexOf('useEffect(() => {', app.indexOf('function commitPreparedRealtimeVideoCandidate(')),
  );
  const flush = app.slice(
    app.indexOf('async function flushPendingMediaApply('),
    app.indexOf('async function cleanupLocalCaches('),
  );

  assert.match(apply, /mediaApplyInFlightRef\.current = false;[\s\S]*flushPendingMediaApply\(\)/);
  assert.match(commit, /realtimeVideoCommitInFlightRef\.current = false;[\s\S]*flushPendingMediaApply\(\)/);
  assert.doesNotMatch(flush, /mediaProcessingBusy/);
});

test('视频 prepare 瞬时失败使用有界退避，暂停停止与换代会取消旧重试', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const videoApply = app.slice(
    app.indexOf('async function applyVideoProcessing('),
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
  );
  const realtimeCommit = app.slice(
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
    app.indexOf('useEffect(() => {', app.indexOf('function commitPreparedRealtimeVideoCandidate(')),
  );
  const clearVideoPlans = app.slice(
    app.indexOf('function clearVideoFutureMediaCyclePlans('),
    app.indexOf('function videoRenderIdentity('),
  );

  assert.match(videoApply, /scheduleVideoPrepareRetry\(params, mediaCandidate\)/);
  assert.match(realtimeCommit, /scheduleVideoPrepareRetry\(candidate\.params, candidate\)/);
  assert.doesNotMatch(realtimeCommit, /FFmpeg 回退/);
  assert.doesNotMatch(videoApply, /setTimeout\(\(\) => void flushPendingMediaApply\(\), 250\)/);
  assert.match(clearVideoPlans, /resetVideoPrepareRetry\(\)/);
  assert.match(clearVideoPlans, /pendingMediaApplyRef\.current = null/);
  assert.match(app, /if \(!playbackActive\) cancelVideoPrepareRetry\(\)/);
  assert.match(app, /videoCycleRetryRef\.current = recordCycleRetryFailure/);
});

test('当前普通声音链不再调用实时话术候选 commit 命令', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.doesNotMatch(app, /commit_audio_variant_candidate_if_due/);
  assert.match(app, /pending_audio_media_plan_id/);
  assert.match(app, /prepare_audio_media_candidate/);
});
