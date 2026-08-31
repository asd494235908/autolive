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

test('App 不再维护视频 apply queue，音频候选仍独立提交', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /mediaApplyInFlightRef|realtimeVideoCommitInFlightRef|flushPendingMediaApply/);
  assert.match(app, /prepareNextAudioMediaCandidate/);
  assert.match(app, /commitCompletedAudioRender/);
});

test('视频周期配置幂等重试，不再保留 WebView prepare 退避', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(app, /realtimeVideoCycleConfigurationKeyRef/);
  assert.match(app, /setVideoCycleConfigureRevision/);
  assert.doesNotMatch(app, /scheduleVideoPrepareRetry|videoCycleRetryRef|pendingMediaApplyRef/);
});

test('当前普通声音链不再调用实时话术候选 commit 命令', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.doesNotMatch(app, /commit_audio_variant_candidate_if_due/);
  assert.match(app, /pending_audio_media_plan_id/);
  assert.match(app, /prepare_audio_media_candidate/);
});
