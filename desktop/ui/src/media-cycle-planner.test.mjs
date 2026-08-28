import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-cycle-planner.ts', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const planner = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

const seed = (planId, periodMediaMs, payload) => ({ planId, periodMediaMs, payload });

test('初始化只创建 N+1 和 N+2 两个绝对媒体时间计划', () => {
  const queue = planner.createMediaCycleQueue(12_000, [
    seed('audio-1', 5_000, { seed: 11 }),
    seed('audio-2', 8_000, { seed: 22 }),
  ]);

  assert.equal(queue.length, 2);
  assert.deepEqual(queue.map(({ planId, sequence, targetAbsolutePositionMs }) => ({
    planId,
    sequence,
    targetAbsolutePositionMs,
  })), [
    { planId: 'audio-1', sequence: 1, targetAbsolutePositionMs: 17_000 },
    { planId: 'audio-2', sequence: 2, targetAbsolutePositionMs: 25_000 },
  ]);
  assert.deepEqual(Object.keys(queue[1]).sort(), [
    'payload',
    'periodMediaMs',
    'planId',
    'sequence',
    'targetAbsolutePositionMs',
  ]);
});

test('推进时旧 N+2 原样晋升 N+1，并只补一个新的 N+2', () => {
  const initial = planner.createMediaCycleQueue(0, [
    seed('one', 5_000, 'one'),
    seed('two', 6_000, 'two'),
  ]);
  const advanced = planner.advanceMediaCycleQueue(initial, seed('three', 7_000, 'three'));

  assert.equal(advanced.length, 2);
  assert.strictEqual(advanced[0], initial[1]);
  assert.deepEqual(advanced[1], {
    planId: 'three',
    sequence: 3,
    periodMediaMs: 7_000,
    targetAbsolutePositionMs: 18_000,
    payload: 'three',
  });
});

test('独立模式为声音和视频维护不同的两周期队列', () => {
  const queues = planner.createIndependentMediaCycleQueues(2_000, {
    audio: [seed('a1', 5_000, 'audio-1'), seed('a2', 7_000, 'audio-2')],
    video: [seed('v1', 8_000, 'video-1'), seed('v2', 9_000, 'video-2')],
  });

  assert.deepEqual(queues.audio.map((item) => item.targetAbsolutePositionMs), [7_000, 14_000]);
  assert.deepEqual(queues.video.map((item) => item.targetAbsolutePositionMs), [10_000, 19_000]);
});

function assertFrameAligned(targetMs, fps) {
  const framePosition = targetMs * fps / 1_000;
  assert.ok(
    Math.abs(framePosition - Math.round(framePosition)) <= fps / 2_000 + 1e-9,
    `${targetMs}ms should represent a ${fps}fps frame`,
  );
}

test('30fps 视频目标向后对齐到最近帧且不早于原计划', () => {
  const align = (targetMs) => planner.alignMediaPositionToVideoFrame(targetMs, 30);
  const queue = planner.createMediaCycleQueue(128, [
    seed('video-1', 8_000, 'one'),
    seed('video-2', 9_000, 'two'),
  ], 1, align);

  assert.deepEqual(queue.map((item) => item.targetAbsolutePositionMs), [8_133, 17_133]);
  assert.ok(queue[0].targetAbsolutePositionMs >= 8_128);
  assert.ok(queue[1].targetAbsolutePositionMs >= queue[0].targetAbsolutePositionMs + 9_000);
  queue.forEach((item) => assertFrameAligned(item.targetAbsolutePositionMs, 30));
});

test('30000/1001fps 视频目标保持单调帧对齐', () => {
  const fps = 30_000 / 1_001;
  const align = (targetMs) => planner.alignMediaPositionToVideoFrame(targetMs, fps);
  const queue = planner.createMediaCycleQueue(123, [
    seed('video-1', 8_000, 'one'),
    seed('video-2', 9_000, 'two'),
  ], 1, align);

  assert.ok(queue[0].targetAbsolutePositionMs >= 8_123);
  assert.ok(queue[1].targetAbsolutePositionMs >= queue[0].targetAbsolutePositionMs + 9_000);
  queue.forEach((item) => assertFrameAligned(item.targetAbsolutePositionMs, fps));
});

test('视频队列晋升保留 N+1 并只对齐新 N+2', () => {
  const align = (targetMs) => planner.alignMediaPositionToVideoFrame(targetMs, 30);
  const initial = planner.createMediaCycleQueue(0, [
    seed('one', 5_001, 'one'),
    seed('two', 6_002, 'two'),
  ], 1, align);
  const advanced = planner.advanceMediaCycleQueue(
    initial,
    seed('three', 7_003, 'three'),
    align,
  );

  assert.strictEqual(advanced[0], initial[1]);
  assert.ok(advanced[1].targetAbsolutePositionMs >= initial[1].targetAbsolutePositionMs + 7_003);
  assertFrameAligned(advanced[1].targetAbsolutePositionMs, 30);
});

test('帧率缺失或超出媒体契约时保留原整数毫秒目标', () => {
  for (const fps of [null, undefined, 0, -1, 0.5, 241, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(planner.alignMediaPositionToVideoFrame(8_123, fps), 8_123);
  }
});

test('独立队列只对齐视频目标，音频目标保持原值', () => {
  const queues = planner.createIndependentMediaCycleQueues(2_001, {
    audio: [seed('a1', 5_000, 'audio-1'), seed('a2', 7_000, 'audio-2')],
    video: [seed('v1', 8_000, 'video-1'), seed('v2', 9_000, 'video-2')],
  }, 1, {
    video: (targetMs) => planner.alignMediaPositionToVideoFrame(targetMs, 30),
  });

  assert.deepEqual(queues.audio.map((item) => item.targetAbsolutePositionMs), [7_001, 14_001]);
  assert.deepEqual(queues.video.map((item) => item.targetAbsolutePositionMs), [10_033, 19_033]);
});

test('单项循环视频的 N+1/N+2 目标都不能跨源 EOF', () => {
  assert.deepEqual(
    planner.resolveSourceBoundedVideoCycleQueueTargets(65_000, 13_000, 8_000, 72_300),
    [
      { targetAbsolutePositionMs: 72_300, skipVideoProcessing: false },
      { targetAbsolutePositionMs: 80_300, skipVideoProcessing: false },
    ],
  );
});

test('尾段处理标记属于覆盖到 EOF 的前一个计划，不属于 EOF 后计划', () => {
  assert.deepEqual(
    planner.resolveSourceBoundedVideoCycleQueueTargets(55_000, 10_000, 13_000, 72_300),
    [
      { targetAbsolutePositionMs: 65_000, skipVideoProcessing: true },
      { targetAbsolutePositionMs: 72_300, skipVideoProcessing: false },
    ],
  );
});

test('从源 EOF 边界开始的新队列恢复正常随机目标', () => {
  assert.deepEqual(
    planner.resolveSourceBoundedVideoCycleQueueTargets(72_300, 8_000, 9_000, 72_300),
    [
      { targetAbsolutePositionMs: 80_300, skipVideoProcessing: false },
      { targetAbsolutePositionMs: 89_300, skipVideoProcessing: false },
    ],
  );
  assert.doesNotMatch(appSource, /skip_video_processing|prepare_media_video_stream_period/);
});

test('单项 EOF 后按 loop_index 重建调度身份并从本地下一 sequence 继续', () => {
  const initializeStart = appSource.indexOf('function initializeFutureMediaCyclePlans(');
  const initializeEnd = appSource.indexOf('function createPlannedArtifactTimeline(', initializeStart);
  const initialize = appSource.slice(initializeStart, initializeEnd);
  assert.match(
    initialize,
    /`\$\{clock\.playback_generation\}:\$\{clock\.source_revision\}:\$\{clock\.clock_epoch\}:\$\{currentSnapshot\?\.loop_index \?\? clock\.loop_index\}`/,
  );
  assert.match(initialize, /mediaCandidateSequenceRef\.current \+ 1/);
  assert.match(initialize, /clearFutureMediaCyclePlans\(\)/);

  const queue = planner.createMediaCycleQueue(72_300, [
    seed('video-4', 8_000, 'four'),
    seed('video-5', 9_000, 'five'),
  ], 4);
  assert.deepEqual(queue.map(({ sequence, targetAbsolutePositionMs }) => ({
    sequence,
    targetAbsolutePositionMs,
  })), [
    { sequence: 4, targetAbsolutePositionMs: 80_300 },
    { sequence: 5, targetAbsolutePositionMs: 89_300 },
  ]);

  assert.match(
    appSource,
    /`\$\{mediaState\.playback_generation\}:\$\{mediaState\.source_revision\}:\$\{mediaState\.clock_epoch\}:\$\{snapshot\.loop_index\}`/,
  );

  const clearStart = appSource.indexOf('function clearVideoFutureMediaCyclePlans(');
  const clearEnd = appSource.indexOf('function videoRenderIdentity(', clearStart);
  const clear = appSource.slice(clearStart, clearEnd);
  assert.match(clear, /resetVideoPrepareRetry\(\)/);
  assert.match(clear, /activeVideoRenderRef\.current = null/);
  assert.match(clear, /pendingMediaApplyRef\.current = null/);
  assert.doesNotMatch(clear, /preserveStream|video_stream/);
});

test('调度器不再提供把音视频边界合并成同一候选的入口', () => {
  assert.equal(planner.resolveNextMediaCycleBoundary, undefined);
});

test('周期进度只读取同一条 N+1 的绝对媒体时间，跨循环连续并在晋升后归零', () => {
  const initial = planner.createMediaCycleQueue(12_000, [
    seed('one', 8_000, 'one'),
    seed('two', 15_000, 'two'),
  ]);

  assert.equal(planner.getMediaCycleProgressPercent(initial[0], 12_000), 0);
  assert.equal(planner.getMediaCycleProgressPercent(initial[0], 16_000), 50);
  assert.equal(planner.getMediaCycleProgressPercent(initial[0], 19_999), 99);
  assert.equal(planner.getMediaCycleProgressPercent(initial[0], 20_000), 100);
  assert.equal(planner.getMediaCycleProgressPercent(initial[0], 21_000), 100);

  const advanced = planner.advanceMediaCycleQueue(initial, seed('three', 10_000, 'three'));
  assert.equal(planner.getMediaCycleProgressPercent(advanced[0], 20_000), 0);
  assert.equal(planner.getMediaCycleProgressPercent(advanced[0], 27_500), 50);
});

test('周期进度拒绝缺失或越界时钟，暂停与缓冲可复用最后真实位置自然冻结', () => {
  const [plan] = planner.createMediaCycleQueue(10_000, [
    seed('one', 10_000, 'one'),
    seed('two', 10_000, 'two'),
  ]);

  assert.equal(planner.getMediaCycleProgressPercent(null, 15_000), 0);
  assert.equal(planner.getMediaCycleProgressPercent(plan, null), 0);
  assert.equal(planner.getMediaCycleProgressPercent(plan, Number.MAX_SAFE_INTEGER + 1), 0);
  assert.equal(planner.getMediaCycleProgressPercent(plan, 15_000), 50);
  assert.equal(planner.getMediaCycleProgressPercent(plan, 15_000), 50);
});

test('主页视频周期进度只读取当前 N+1 绝对时钟，不混用旧处理百分比', () => {
  const progressStart = appSource.indexOf('const nextVideoPlan =');
  const progressEnd = appSource.indexOf('const diagnosticMessage =', progressStart);
  const progress = appSource.slice(progressStart, progressEnd);
  assert.match(progress, /const runtimeProgressPercent = scheduledVideoProgressPercent/);
  assert.doesNotMatch(progress, /video_processing_progress_percent|videoProcessingProgressPercent/);
  assert.match(progress, /activeVideoCandidate\?\.videoCyclePlan\?\.planId === nextVideoPlan\.planId/);
  assert.match(progress, /activeVideoCandidate\.videoCyclePlan\.sequence === nextVideoPlan\.sequence/);
  assert.match(progress, /activeVideoCandidate\.playbackGeneration === snapshot\.playback_generation/);

  const cardStart = appSource.indexOf('title="视频周期"');
  const cardEnd = appSource.indexOf('\n            />', cardStart);
  const card = appSource.slice(cardStart, cardEnd);
  assert.match(card, /status=\{videoCycleStatus\}/);
  assert.match(card, /statusColor=\{videoCycleStatusColor\}/);
});

test('主页视频状态只认当前 N+1 身份，旧 N 呈现不能清除新计划状态', () => {
  const statusStart = appSource.indexOf('const videoProcessingStatus: ProcessingStatusKey');
  const statusEnd = appSource.indexOf('const audioProcessingStatus =', statusStart);
  const status = appSource.slice(statusStart, statusEnd);

  assert.match(status, /currentVideoCandidatePreparing/);
  assert.match(status, /currentVideoCandidateReady/);
  assert.match(status, /currentVideoCandidatePresented/);
  assert.match(status, /currentVideoCandidateFailed/);
  assert.doesNotMatch(status, /snapshot\?\.video_processing_status|video_processing_progress_percent/);
  assert.doesNotMatch(status, /playbackClockCycleStatus|videoBoundaryApplyPending/);
  assert.doesNotMatch(appSource, /setVideoBoundaryApplyPending/);
});

test('视频周期到点只提交已准备候选，不再启动渲染', () => {
  const applyStart = appSource.indexOf('function applyPlannedVideoCycle(');
  const applyEnd = appSource.indexOf('function applyPlannedAudioCycle(', applyStart);
  const applyVideoCycle = appSource.slice(applyStart, applyEnd);

  assert.ok(applyStart >= 0 && applyEnd > applyStart);
  assert.doesNotMatch(applyVideoCycle, /schedulePeriodRenderRef\.current|start_media_processing/);
  assert.doesNotMatch(applyVideoCycle, /update_video_effect_stream|video_stream/);

  const schedulerStart = appSource.indexOf('// 定时器只负责唤醒');
  const schedulerEnd = appSource.indexOf('// 仅播放中才推送实时参数', schedulerStart);
  const scheduler = appSource.slice(schedulerStart, schedulerEnd);
  const commitStart = appSource.indexOf('function commitPreparedRealtimeVideoCandidate(');
  const commit = appSource.slice(commitStart, appSource.indexOf('function commitCompletedAudioRender(', commitStart));
  assert.doesNotMatch(scheduler, /applyPlannedVideoCycle\(|applyPlannedAudioCycle\(/);
  assert.match(commit, /clock\.absolute_position_ms < candidate\.timeline\.targetAbsolutePositionMs/);
  assert.match(commit, /applyPlannedVideoCycle\(candidate\.videoCyclePlan\)/);
  assert.doesNotMatch(commit, /video_stream|current_video_reference|activate_ffmpeg_video_backend/);
});

test('视频周期提交原子更新参数并立即广播，正式视频候选优先进入 mpv 实时链', () => {
  const applyStart = appSource.indexOf('function applyPlannedVideoCycle(');
  const applyEnd = appSource.indexOf('function applyPlannedAudioCycle(', applyStart);
  const applyCycle = appSource.slice(applyStart, applyEnd);
  const realtimeStart = appSource.indexOf('async function applyVideoProcessing(');
  const realtimeEnd = appSource.indexOf('function commitPreparedRealtimeVideoCandidate(', realtimeStart);
  const realtimePrepare = appSource.slice(realtimeStart, realtimeEnd);

  assert.match(applyCycle, /mediaEffectParamsMutationVersionRef\.current \+= 1/);
  assert.match(applyCycle, /publishRuntimeParameterMessage\(nextParams\)/);
  assert.match(realtimePrepare, /prepare_realtime_video_plan/);
  assert.match(realtimePrepare, /canUseRealtimeVideoBackend\(status\)/);
  assert.match(realtimePrepare, /mediaCandidate\.realtimePrepared = true/);
  assert.doesNotMatch(realtimePrepare, /prepare_media_video_stream_period|video_stream|MediaSource|SourceBuffer/);
});

test('初始化和晋升队列后分别准备音频与视频 N+1，N+2 不触发 IPC', () => {
  const initializeStart = appSource.indexOf('function initializeFutureMediaCyclePlans(');
  const initializeEnd = appSource.indexOf('function applyPlannedVideoCycle(', initializeStart);
  const initialize = appSource.slice(initializeStart, initializeEnd);
  const advanceStart = appSource.indexOf('function advanceIndependentAudioQueue(');
  const advanceEnd = appSource.indexOf('const [runtimeChannelError', advanceStart);
  const advance = appSource.slice(advanceStart, advanceEnd);

  assert.match(initialize, /prepareNextAudioMediaCandidate/);
  assert.match(initialize, /prepareNextVideoMediaCandidate/);
  assert.match(advance, /prepareNextAudioMediaCandidate/);
  assert.match(advance, /prepareNextVideoMediaCandidate/);
  assert.doesNotMatch(initialize, /\[1\][\s\S]*start_media_processing/);
});

test('声音和视频候选使用独立 IPC、独立队列推进且互不重编码', () => {
  const audioPrepare = appSource.slice(
    appSource.indexOf('async function prepareNextAudioMediaCandidate('),
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
  );
  const videoPrepare = appSource.slice(
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
    appSource.indexOf('function applyPlannedVideoCycle('),
  );
  const audioAdvance = appSource.slice(
    appSource.indexOf('function advanceIndependentAudioQueue('),
    appSource.indexOf('function advanceIndependentVideoQueue('),
  );
  const videoAdvance = appSource.slice(
    appSource.indexOf('function advanceIndependentVideoQueue('),
    appSource.indexOf('const [runtimeChannelError'),
  );

  assert.match(audioPrepare, /prepare_audio_media_candidate/);
  assert.doesNotMatch(audioPrepare, /start_media_processing|videoFuturePlansRef/);
  assert.match(videoPrepare, /applyVideoProcessing\(params, candidate\)/);
  assert.doesNotMatch(videoPrepare, /prepare_audio_media_candidate|audioFuturePlansRef/);
  assert.doesNotMatch(audioAdvance, /videoFuturePlansRef|advanceIndependentVideoQueue/);
  assert.doesNotMatch(videoAdvance, /audioFuturePlansRef|advanceIndependentAudioQueue/);
});

test('实时视频计划绑定播放器视频源修订，声音 revision 变化不使画面 N+1 过期', () => {
  const audioPrepare = appSource.slice(
    appSource.indexOf('async function prepareNextAudioMediaCandidate('),
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
  );
  const videoPrepare = appSource.slice(
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
    appSource.indexOf('function applyPlannedVideoCycle('),
  );
  const manualVideoPrepare = appSource.slice(
    appSource.indexOf('async function applyVideoProcessing('),
    appSource.indexOf('function commitPreparedRealtimeVideoCandidate('),
  );

  assert.match(audioPrepare, /createPlannedArtifactTimeline\([\s\S]*currentSnapshot\.audio_stream_revision/);
  assert.match(videoPrepare, /createPlannedArtifactTimeline\([\s\S]*clock\.source_revision/);
  assert.doesNotMatch(videoPrepare, /currentSnapshot\.audio_stream_revision/);
  assert.match(manualVideoPrepare, /sourceRevision:\s*clock\.source_revision/);
  assert.doesNotMatch(manualVideoPrepare, /sourceRevision:\s*currentSnapshot\.audio_stream_revision/);
});

test('正式周期使用 mpv 实时候选，失败后保持 Original 且不进入旧 MSE 链', () => {
  const videoApplyStart = appSource.indexOf('async function applyVideoProcessing(');
  const videoApplyEnd = appSource.indexOf('function commitPreparedRealtimeVideoCandidate(', videoApplyStart);
  const videoApply = appSource.slice(videoApplyStart, videoApplyEnd);
  const realtimeIndex = videoApply.indexOf("invoke<unknown>('prepare_realtime_video_plan'");
  const failedRealtime = videoApply.slice(realtimeIndex);

  assert.ok(realtimeIndex >= 0);
  assert.match(failedRealtime, /stop_realtime_video_renderer/);
  assert.match(failedRealtime, /releaseUnstartedCandidate\(mediaCandidate\)/);
  assert.match(failedRealtime, /scheduleVideoCycleRetry\(\)/);
  assert.match(failedRealtime, /return;/);
  assert.doesNotMatch(videoApply, /prepare_media_video_stream_period|video_stream|MediaSource|SourceBuffer/);
});

test('声音只走自动 M4A 候选，视频重试只走 mpv 候选', () => {
  const videoApply = appSource.slice(
    appSource.indexOf('async function applyVideoProcessing('),
    appSource.indexOf('function commitCompletedAudioRender('),
  );
  const videoPanel = appSource.slice(
    appSource.indexOf('title="画面"'),
    appSource.indexOf('title="最终效果窗口"'),
  );

  assert.doesNotMatch(appSource, /schedulePeriodMediaRender|schedulePeriodRenderRef|audio-manual-/);
  assert.match(videoApply, /prepare_realtime_video_plan/);
  assert.doesNotMatch(videoApply, /prepare_media_video_stream_period|period_id:|video_stream/);
  assert.doesNotMatch(videoApply, /audio_variants|ambient_sound_path|['"]both['"]/);
  assert.match(videoPanel, /retryVideoProcessing/);
});

test('音频与视频候选使用独立 worker，不因另一领域 processing 或目标迟到而重建', () => {
  const audioPrepare = appSource.slice(
    appSource.indexOf('async function prepareNextAudioMediaCandidate('),
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
  );
  const videoPrepare = appSource.slice(
    appSource.indexOf('function prepareNextVideoMediaCandidate('),
    appSource.indexOf('function applyPlannedVideoCycle('),
  );
  const applyMedia = appSource.slice(
    appSource.indexOf('async function applyVideoProcessing('),
    appSource.indexOf('function commitCompletedAudioRender('),
  );

  assert.doesNotMatch(audioPrepare, /mediaWorkerReservationRef|video_processing_status === 'processing'/);
  assert.match(audioPrepare, /isMediaWorkerBusyError\(cause\)[\s\S]*return/);
  assert.doesNotMatch(audioPrepare, /clock\.absolute_position_ms >= plan\.targetAbsolutePositionMs/);
  assert.doesNotMatch(videoPrepare, /mediaWorkerReservationRef|audio_processing_status === 'processing'/);
  assert.doesNotMatch(videoPrepare, /clock\.absolute_position_ms >= plan\.targetAbsolutePositionMs/);
  assert.doesNotMatch(applyMedia, /video_processing_status === 'processing'|isVideoPrewarmRetryableError|media_video_stream/);
  assert.match(applyMedia, /scheduleVideoPrepareRetry\(params, mediaCandidate\)/);
});

test('处理开关变化后按仍启用的音视频域生成候选文件', () => {
  const switchesStart = appSource.indexOf('async function updateProcessingSwitches(');
  const switchesEnd = appSource.indexOf('async function applyVideoProcessing(', switchesStart);
  const switches = appSource.slice(switchesStart, switchesEnd);

  assert.match(switches, /wakeMediaCycleScheduling\('switch'\)/);
  assert.doesNotMatch(switches, /applyVideoProcessing\(|schedulePeriodRenderRef\.current/);
});

test('声音与画面快速双开读取同步开关引用，不用上一帧状态覆盖另一域', () => {
  const audioSwitch = appSource.slice(
    appSource.indexOf('aria-label="声音处理"'),
    appSource.indexOf('</Switch>', appSource.indexOf('aria-label="声音处理"')),
  );
  const videoSwitch = appSource.slice(
    appSource.indexOf('aria-label="视频处理"'),
    appSource.indexOf('</Switch>', appSource.indexOf('aria-label="视频处理"')),
  );

  assert.match(audioSwitch, /video_processing_enabled: videoProcessingEnabledRef\.current/);
  assert.match(videoSwitch, /audio_processing_enabled: audioProcessingEnabledRef\.current/);
});

test('同一时钟内声音提交先同步参数引用，随后画面提交不会覆盖新声音参数', () => {
  const audioCommit = appSource.slice(
    appSource.indexOf('function commitAudioCycleSample('),
    appSource.indexOf('function sampleAndCommitAudioCycle(', appSource.indexOf('function commitAudioCycleSample(')),
  );

  assert.match(audioCommit, /const source = baseParams \?\? mediaEffectParamsRef\.current/);
  assert.match(audioCommit, /mediaEffectParamsRef\.current = next;[\s\S]*setMediaEffectParams\(next\)/);
  assert.doesNotMatch(audioCommit, /setMediaEffectParams\(\(current\)/);
});

test('音频设置只刷新自己的未来计划，视频设置仍只清理视频队列', () => {
  const localSettings = appSource.slice(
    appSource.indexOf('audioPeriodRangeRef.current = audioPeriodRange'),
    appSource.indexOf('// 定时器只负责唤醒'),
  );
  const parameterUpdate = appSource.slice(
    appSource.indexOf('function updateMediaEffectParam('),
    appSource.indexOf('async function resetMediaEffectParams'),
  );

  assert.match(localSettings, /audioPeriodRangeRef\.current[\s\S]*refreshAudioFutureMediaCyclePlansAfterSettingsChange\(\)/);
  assert.match(localSettings, /videoPeriodRangeRef\.current[\s\S]*clearVideoFutureMediaCyclePlans\(\)/);
  assert.match(parameterUpdate, /section === 'audio'[\s\S]*refreshAudioFutureMediaCyclePlansAfterSettingsChange\(\)/);
  assert.match(parameterUpdate, /if \(section === 'audio'\) refreshAudioFutureMediaCyclePlansAfterSettingsChange\(\);\s*else clearVideoFutureMediaCyclePlans\(\)/);
});

test('候选处理状态不再伪装成实时流，也不使用后端处理百分比冒充周期进度', () => {
  const videoCycleCardStart = appSource.indexOf('title="视频周期"');
  const videoCycleCard = appSource.slice(
    videoCycleCardStart,
    appSource.indexOf('<MediaParameterPanels', videoCycleCardStart),
  );

  assert.ok(videoCycleCardStart >= 0);
  assert.match(videoCycleCard, /status=\{videoCycleStatus\}/);
  assert.match(videoCycleCard, /statusColor=\{videoCycleStatusColor\}/);
  assert.match(videoCycleCard, /progress=\{runtimeProgressPercent\}/);
  assert.doesNotMatch(videoCycleCard, /实时流缓冲中/);
  assert.doesNotMatch(videoCycleCard, /video_processing_progress_percent/);
});

test('mpv 实时候选由权威播放时钟消息直接唤醒首次提交', () => {
  const playbackStateBranch = appSource.indexOf('if (isPlaybackMediaStateMessage(event.data)) {');
  const messageHandler = appSource.slice(
    playbackStateBranch,
    appSource.indexOf('if (isInterludeRuntimeMessage(event.data))', playbackStateBranch),
  );
  const realtimeCommitStart = appSource.indexOf('function commitPreparedRealtimeVideoCandidate(');
  const realtimeCommit = appSource.slice(
    realtimeCommitStart,
    appSource.indexOf('useEffect(() => {', realtimeCommitStart),
  );

  assert.match(messageHandler, /commitPreparedRealtimeVideoCandidate\(event\.data\)/);
  assert.match(realtimeCommit, /candidate\.realtimePrepared/);
  assert.match(realtimeCommit, /clock\.absolute_position_ms < candidate\.timeline\.targetAbsolutePositionMs/);
  assert.match(realtimeCommit, /commit_realtime_video_plan/);
});
