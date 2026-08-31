import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

function section(start, end) {
  const startIndex = source.indexOf(start);
  assert.notEqual(startIndex, -1, `missing start marker: ${start}`);
  const endIndex = source.indexOf(end, startIndex);
  assert.notEqual(endIndex, -1, `missing end marker: ${end}`);
  return source.slice(startIndex, endIndex);
}

test('视频开始播放时只启动一次受管 Original，并保留 WebView 失败兜底', () => {
  const original = section(
    "const playbackState = snapshot?.playback_state?.toLowerCase();",
    'const managedBackend =',
  );

  assert.match(original, /!currentMediaIsVideo[\s\S]*!mediaState[\s\S]*mediaState\.playback_generation !== snapshot\?\.playback_generation[\s\S]*mediaState\.loop_index !== snapshot\?\.loop_index/);
  assert.match(original, /\['playing', 'paused'\]\.includes/);
  assert.match(original, /canUseManagedNativeVideo\(mediaVideoBackendStatus,[\s\S]*playback_generation: mediaState\.playback_generation,[\s\S]*clock_epoch: mediaState\.clock_epoch,[\s\S]*loop_index: mediaState\.loop_index/);
  assert.match(original, /originalVideoEnsureAttemptRef\.current === attemptKey[\s\S]*return/);
  assert.match(original, /const expectedBackendEpoch = mediaVideoBackendStatus\?\.playback_generation[\s\S]*=== mediaState\.playback_generation[\s\S]*mediaVideoBackendStatus\.backend_epoch[\s\S]*invoke<unknown>\('ensure_original_video_renderer',[\s\S]*playback_generation: mediaState\.playback_generation,[\s\S]*backend_epoch: expectedBackendEpoch,[\s\S]*clock_epoch: mediaState\.clock_epoch,[\s\S]*loop_index: mediaState\.loop_index,[\s\S]*position_ms: mediaState\.position_ms/);
  assert.match(original, /expectedBackendEpoch,[\s\S]*\.catch\([\s\S]*get_media_video_backend_status[\s\S]*isPermanentRealtimeVideoError\(cause\)[\s\S]*recordCycleRetryFailure[\s\S]*window\.setTimeout[\s\S]*originalVideoEnsureAttemptRef\.current = null/);
  assert.match(original, /status\?\.backend !== 'source'[\s\S]*status\.activation !== 'active'[\s\S]*!hasManagedVideoProcess\(status\)/);
  assert.match(original, /Original 播放器启动失败，当前保留 WebView 兼容画面/);
  assert.doesNotMatch(original, /setMediaVideoBackendStatus\(null\)/);
  assert.doesNotMatch(original, /scheduleVideoPrepareRetry|stop_realtime_video_renderer/);
});

test('受管 source 与 realtime_gpu 共用代次、时钟、循环及暂停同步', () => {
  const sync = section('const managedBackend =', 'function commitCompletedAudioRender');

  assert.match(sync, /backend === 'realtime_gpu'[\s\S]*\['available', 'active'\]\.includes/);
  assert.match(sync, /backend === 'cpu4'[\s\S]*\['available', 'active'\]\.includes/);
  assert.match(sync, /backend === 'source'[\s\S]*activation === 'active'[\s\S]*hasManagedVideoProcess/);
  assert.match(sync, /buildRealtimeVideoSyncRequest\([\s\S]*mediaState,[\s\S]*mediaVideoBackendStatus,[\s\S]*playbackDisplayState !== 'playing'/);
  assert.match(sync, /invokeSync: \(request\) => invoke<unknown>\('sync_realtime_video_renderer', \{ request \}\)/);
  assert.match(sync, /getRealtimeVideoSyncCoordinator\(\)\.submit\(syncRequest\)/);
  assert.match(sync, /isRealtimeVideoSyncApplied\(parsed\.status, request\)/);
  assert.match(sync, /get_media_video_backend_status'[\s\S]*acceptMediaVideoBackendResponse\(response\)/);
  assert.match(sync, /mediaState\?\.clock_epoch[\s\S]*mediaState\?\.loop_index[\s\S]*playbackDisplayState[\s\S]*hasManagedVideoProcess\(mediaVideoBackendStatus\)/);
  assert.match(sync, /get_media_video_backend_status[\s\S]*VIDEO_BACKEND_STATUS_POLL_MS/);
});

test('开启视频处理接受 GPU 或 CPU4，并把降级 Original 视为同代次终点', () => {
  const prepare = section('async function applyVideoProcessing(', 'function commitPreparedRealtimeVideoCandidate');
  const commit = section('function commitPreparedRealtimeVideoCandidate', 'useEffect(() => {\n    if (mediaState)');

  assert.match(prepare, /invoke<unknown>\('prepare_realtime_video_plan'/);
  assert.match(prepare, /canPrepareManagedVideoBackend\(status\)[\s\S]*realtimeIdentity\.playback_generation[\s\S]*mediaCandidate\.realtimeIdentity = realtimeIdentity/);
  assert.match(prepare, /if \(isTerminalMediaVideoBackendStatus\(status\)\)[\s\S]*releaseUnstartedCandidate\(mediaCandidate\)[\s\S]*if \(!isTerminalVideoSource\(status\)\)/);
  assert.match(commit, /if \(isTerminalMediaVideoBackendStatus\(status\)\)[\s\S]*releaseUnstartedCandidate\(candidate\)[\s\S]*if \(!terminalOriginal\)[\s\S]*return/);
  const terminalBranch = commit.slice(commit.indexOf('if (isTerminalMediaVideoBackendStatus(status))'), commit.indexOf('if (!managedEffectsBackend)'));
  assert.doesNotMatch(terminalBranch, /applyPlannedVideoCycle/);
  assert.doesNotMatch(prepare, /ensure_original_video_renderer/);
  assert.doesNotMatch(prepare, /stop_realtime_video_renderer/);
  assert.doesNotMatch(commit, /stop_realtime_video_renderer/);
});

test('最终效果窗只在 Rust 原生身份确认后停止 WebView 解码，EOF 推进仍归 Rust', () => {
  const finalEffect = section('function FinalEffectWindow()', 'function DesktopApp()');

  assert.match(finalEffect, /invoke<unknown>\('get_media_video_backend_status'\)/);
  assert.match(finalEffect, /resolveManagedNativeVideoOwnership/);
  assert.match(finalEffect, /managedNativeVideoOwnsPlayback\s*\?\s*undefined\s*:\s*sourceUrl/);
  assert.match(finalEffect, /preload=\{managedNativeVideoOwnsPlayback\s*\?\s*'none'\s*:\s*'auto'\}/);
  assert.match(finalEffect, /managedNativeVideoOwnsPlayback[\s\S]*clearVideoElement\(video\)/);
  assert.doesNotMatch(finalEffect, /mediaVideoEofMatches|nativeEofTriggerRef/);
  const nativeStatusPolling = finalEffect.slice(
    finalEffect.indexOf("if (!documentVisible || snapshot?.source_media?.media_kind !== 'video')"),
    finalEffect.indexOf('// 最终效果窗自行探测 PortAudio'),
  );
  assert.doesNotMatch(nativeStatusPolling, /complete_playback_item|\.currentTime\s*=|\.play\(\)/);
  assert.match(finalEffect, /managedNativeVideoOwnershipRef/);
  assert.match(finalEffect, /opacity: currentMediaIsAudio \|\| managedNativeVideoOwnsPlayback \? 0 : 1/);
});

test('final-effect 只新增现有视频状态只读权限', async () => {
  const commands = await readFile(new URL('../../src-tauri/permissions/command-sets.toml', import.meta.url), 'utf8');
  const finalEffect = commands.slice(commands.indexOf('identifier = "final-effect-commands"'));
  assert.match(finalEffect, /allow-get-media-video-backend-status/);
  const commandSource = await readFile(new URL('../../src-tauri/src/commands.rs', import.meta.url), 'utf8');
  const statusStart = commandSource.indexOf('pub fn get_media_video_backend_status(');
  const statusCommand = commandSource.slice(
    statusStart,
    commandSource.indexOf('#[tauri::command', statusStart + 1),
  );
  assert.match(statusCommand, /ensure_playback_window/);
  assert.doesNotMatch(statusCommand, /ensure_main_window/);
});
