import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('最终效果窗口只渲染媒体并把内部错误留给主窗口诊断', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));

  assert.doesNotMatch(finalEffectWindow, /<Alert\b/);
  assert.match(finalEffectWindow, /error:\s*playbackError \?\? finalEffectResizeError/);
  assert.match(desktopApp, /<PlaybackPoolPanel[\s\S]*error=\{error\}/);
});

test('最终效果窗口不叠加实时 filter + transform', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );

  assert.doesNotMatch(finalEffectWindow, /filter:\s*runtimeVideoStyle\.filter/);
  assert.doesNotMatch(finalEffectWindow, /transform:\s*runtimeVideoStyle\.transform/);
  assert.doesNotMatch(app, /buildRuntimePreviewParameters/);
  assert.doesNotMatch(app, /runtimePreview/);
  assert.match(finalEffectWindow, /runtimeAudioParamsRef\.current\s*=\s*event\.data\.payload/);
  assert.match(finalEffectWindow, /resolveSynchronizedVideoPlaybackRate\([\s\S]*params\?\.audio_playback_speed/);
});

test('最终效果兼容 URL 固定使用源播放引用，处理画面只走 mpv', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const playbackVideoUrl = app.slice(
    app.indexOf('function playbackVideoUrl('),
    app.indexOf('const INTERLUDE_AUDIO_PRESET_IDS'),
  );

  assert.match(playbackVideoUrl, /snapshot\?\.source_media\?\.playback_reference/);
  assert.doesNotMatch(playbackVideoUrl, /current_video_reference|current_video_source/);
});

test('最终效果视频身份只绑定当前播放代次与 Original 源引用', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const boundary = app.slice(
    app.indexOf('function isCurrentFinalEffectVideo('),
    app.indexOf('useEffect(() => {', app.indexOf('function isCurrentFinalEffectVideo(')),
  );

  assert.match(boundary, /const currentSourceIdentity = playbackVideoUrl\(currentSnapshot\)/);
  assert.match(boundary, /playbackGeneration: currentSnapshot\?\.playback_generation/);
  assert.match(boundary, /source_media\?\.media_kind === 'video'[\s\S]*sourceAudioRef\.current[\s\S]*sourceIdentityElement\?\.getAttribute\('src'\) === currentSourceIdentity/);
  assert.doesNotMatch(boundary, /video_stream|videoMse|MediaSource|SourceBuffer/);
});

test('纯音频继续复用主媒体时钟，但渲染表面透明且不参与视频能力', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const panel = await readFile(new URL('./desktop/playback-pool-panel.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));
  const sourceGuard = app.slice(
    app.indexOf('function isPlaybackPoolSource('),
    app.indexOf('function isInterludeSnapshot('),
  );

  assert.match(panel, /media_kind: 'video' \| 'audio'/);
  assert.match(panel, /playback_reference: string/);
  assert.match(panel, /compatibility_mode: 'direct' \| 'remuxed' \| 'transcoded'/);
  assert.match(sourceGuard, /record\.media_kind === 'video' \|\| record\.media_kind === 'audio'/);
  assert.match(sourceGuard, /isBoundedString\(record\.playback_reference, false\)/);
  assert.match(sourceGuard, /\['direct', 'remuxed', 'transcoded'\]\.includes\(record\.compatibility_mode as string\)/);
  assert.match(finalEffectWindow, /const currentMediaIsAudio = snapshot\?\.source_media\?\.media_kind === 'audio'/);
  assert.match(finalEffectWindow, /opacity: currentMediaIsAudio \? 0 : 1/);
  assert.doesNotMatch(finalEffectWindow, /video_stream|VideoMseStreamController|MediaSource|SourceBuffer/);
  assert.match(desktopApp, /const currentMediaIsVideo = snapshot\?\.source_media\?\.media_kind === 'video'/);
  assert.match(desktopApp, /const runtimeActive = MPV_REALTIME_VIDEO_ENABLED[\s\S]*playbackActive[\s\S]*currentMediaIsVideo[\s\S]*videoProcessingEnabled/);
  assert.match(desktopApp, /const pictureInPictureSourceUrl = currentMediaIsVideo[\s\S]*playbackVideoUrl\(snapshot\)/);
  assert.match(desktopApp, /当前音频素材不适用/);
});

test('画中画隐藏 video 只在用户操作时加载，退出后立即释放解码资源', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));
  const pipElementStart = desktopApp.indexOf('<video\n        ref={pictureInPictureVideoRef}');
  const pipElementEnd = desktopApp.indexOf('/>', pipElementStart);
  const pipElement = desktopApp.slice(pipElementStart, pipElementEnd);

  assert.match(desktopApp, /function releasePictureInPictureVideo[\s\S]*video\.pause\(\)[\s\S]*removeAttribute\('src'\)[\s\S]*video\.load\(\)/);
  assert.doesNotMatch(pipElement, /\bsrc=/);
  assert.match(pipElement, /preload="none"/);
  assert.match(desktopApp, /if \(!pictureInPictureActive\) return;[\s\S]*syncPictureInPictureVideo\(\)/);
});

test('最终效果窗可见时持续同步快照，诊断与 PortAudio 仍只在播放中轮询', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));
  const refreshSnapshotIndex = finalEffectWindow.indexOf('const refreshSnapshot = () => {');
  const snapshotPolling = finalEffectWindow.slice(
    finalEffectWindow.lastIndexOf('useEffect(() => {', refreshSnapshotIndex),
    finalEffectWindow.indexOf('// 最终效果窗自行探测 PortAudio'),
  );
  const applySnapshot = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function applyPlayerSnapshot('),
    finalEffectWindow.indexOf('function setRealtimeAudioPlaybackState('),
  );

  assert.match(app, /function useDocumentVisibility\(\)/);
  assert.match(finalEffectWindow, /const finalEffectPollingActive\s*=\s*documentVisible\s*&&\s*snapshot\?\.playback_state\s*===\s*'playing'/);
  assert.match(snapshotPolling, /if \(!documentVisible\) return;/);
  assert.match(snapshotPolling, /window\.setInterval\(refreshSnapshot, PLAYBACK_SNAPSHOT_POLL_MS\)/);
  assert.match(snapshotPolling, /\}, \[documentVisible\]\);/);
  assert.doesNotMatch(snapshotPolling, /finalEffectPollingActive/);
  assert.match(applySnapshot, /nextSnapshot\.playback_state === 'playing'/);
  assert.match(applySnapshot, /videoRef\.current/);
  assert.match(applySnapshot, /video\?\.paused/);
  assert.match(applySnapshot, /video\.play\(\)/);
  assert.match(finalEffectWindow, /if \(!finalEffectPollingActive\) return;[\s\S]*get_audio_cycle_diagnostic/);
  assert.match(finalEffectWindow, /if \(!finalEffectPollingActive \|\| !snapshot\?\.audio_processing_enabled\) return;/);
  assert.match(desktopApp, /const documentVisible = useDocumentVisibility\(\)/);
});

test('最终效果窗内部 play、pause 与 reload 不得改写 Rust 播放状态', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const recovery = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function startPlaybackClockRecovery('),
    finalEffectWindow.indexOf('function handlePlaybackClockFailure('),
  );
  const recoveryFailure = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function handlePlaybackClockFailure('),
    finalEffectWindow.indexOf('useEffect(() => {', finalEffectWindow.indexOf('function handlePlaybackClockFailure(')),
  );
  const videoStart = finalEffectWindow.indexOf('<video');
  const videoElement = finalEffectWindow.slice(videoStart, finalEffectWindow.indexOf('<audio', videoStart));

  assert.match(recovery, /video\.pause\(\);[\s\S]*video\.load\(\)/);
  assert.doesNotMatch(recovery, /pause_playback/);
  assert.doesNotMatch(recoveryFailure, /pause_playback/);
  assert.doesNotMatch(recoveryFailure, /video\.pause\(\)/);
  assert.doesNotMatch(recoveryFailure, /播放已暂停/);
  assert.match(recoveryFailure, /video\.play\(\)/);
  assert.doesNotMatch(videoElement, /onPause=\{[\s\S]*pause_playback/);
  assert.doesNotMatch(videoElement, /onPlay=\{[\s\S]*(?:resume_playback|start_playback)/);
});

test('只有主页显式播放控制才通过 runPlaybackAction 调用后端命令', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));
  const action = desktopApp.slice(
    desktopApp.indexOf('async function runPlaybackAction('),
    desktopApp.indexOf('async function startPlaybackFromHome('),
  );

  assert.match(action, /const nextSnapshot = await invokePlaybackSnapshot\(command\)/);
  assert.match(desktopApp, /onClick=\{\(\) => void runPlaybackAction\('pause', 'pause_playback'\)\}/);
  assert.match(desktopApp, /onClick=\{\(\) => void runPlaybackAction\('resume', 'resume_playback'\)\}/);
  assert.match(desktopApp, /openFinalEffectWindowFromHome\(\);[\s\S]*runPlaybackAction\('resume', 'start_playback'\)/);
});

test('播放中开启普通声音不停播，PortAudio 未就绪时保持 WebView 回退', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const switchUpdate = app.slice(
    app.indexOf('async function updateProcessingSwitches('),
    app.indexOf('async function applyVideoProcessing'),
  );
  const backendMessage = app.slice(
    app.indexOf('if (isAudioOutputBackendMessage(event.data))'),
    app.indexOf('if (!isRuntimeParameterMessage(event.data))'),
  );
  const outputLabel = app.slice(
    app.indexOf('function getActualAudioOutputLabel('),
    app.indexOf('function getActualAudioStreamVariantCount('),
  );
  const webAudioClock = app.slice(
    app.indexOf('function syncWebAudioContextState()'),
    app.indexOf('function setPortAudioHardwareActive('),
  );

  assert.doesNotMatch(switchUpdate, /pause_playback|stop_playback/);
  assert.doesNotMatch(webAudioClock, /!portAudioHardwareRef\.current\s*&&/);
  assert.match(webAudioClock, /snapshotRef\.current\?\.playback_state === 'playing'/);
  assert.match(backendMessage, /event\.data\.preferred_portaudio[\s\S]*event\.data\.running[\s\S]*event\.data\.selected_backend === 'portaudio'/);
  assert.match(outputLabel, /status\?\.running === true[\s\S]*status\.selected_backend\?\.trim\(\)\.toLowerCase\(\) === 'portaudio'[\s\S]*\? 'PortAudio'[\s\S]*: 'WebView'/);
});

test('处理开关变化后按仍启用的音视频域重建候选文件', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const updateSwitches = app.slice(
    app.indexOf('async function updateProcessingSwitches('),
    app.indexOf('async function resetVideoEffectParams()'),
  );

  assert.match(updateSwitches, /next\.video_processing_enabled[\s\S]*next\.audio_processing_enabled/);
  assert.match(updateSwitches, /wakeMediaCycleScheduling\('switch'\)/);
  assert.doesNotMatch(updateSwitches, /applyVideoProcessing\(|schedulePeriodRenderRef\.current/);
});

test('同一渲染帧快速开启声音和画面时合并最新值，且旧响应不覆盖最新快照', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const updateSwitches = app.slice(
    app.indexOf('async function updateProcessingSwitches('),
    app.indexOf('async function resetVideoEffectParams()'),
  );
  const audioSwitchStart = app.indexOf('aria-label="声音处理"');
  const videoSwitchStart = app.indexOf('aria-label="视频处理"');
  const audioSwitch = app.slice(audioSwitchStart, audioSwitchStart + 400);
  const videoSwitch = app.slice(videoSwitchStart, videoSwitchStart + 400);

  assert.match(audioSwitch, /videoProcessingEnabledRef\.current/);
  assert.match(videoSwitch, /audioProcessingEnabledRef\.current/);
  assert.match(updateSwitches, /processingSwitchQueueRef\.current\.then/);
  assert.match(updateSwitches, /const requestId = \+\+processingSwitchRequestRef\.current/);
  assert.match(updateSwitches, /requestId !== processingSwitchRequestRef\.current/);
  assert.match(updateSwitches, /wakeMediaCycleScheduling\('switch'\)/);
});

test('异步音频出口和周期提交始终从最新播放引用生成组合参数，不用旧渲染闭包清除画面滤镜', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const publishStart = app.indexOf('function publishRuntimeParameterMessage(');
  const publishRuntime = app.slice(
    publishStart,
    app.indexOf('// 仅播放中才推送实时参数', publishStart),
  );
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const portAudioHandler = finalEffectWindow.slice(
    finalEffectWindow.indexOf('if (isAudioOutputBackendMessage(event.data))'),
    finalEffectWindow.indexOf('if (!isRuntimeParameterMessage(event.data))'),
  );

  assert.match(publishRuntime, /const currentSnapshot = snapshotRefHome\.current/);
  assert.match(publishRuntime, /const clock = mediaStateRef\.current/);
  assert.match(publishRuntime, /audioProcessingEnabledRef\.current/);
  assert.match(publishRuntime, /const liveVideo = false/);
  assert.doesNotMatch(publishRuntime, /videoProcessingEnabledRef\.current|const liveVideo = runtimeActive/);
  assert.doesNotMatch(portAudioHandler, /setRuntimeVideoFilter|setSourceUrl|setActiveVideoSlot|applyPlayerSnapshot/);
});

test('关闭视频处理时不启动 mpv 周期，最终效果窗保持 Original 直播', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const prepareNext = app.slice(
    app.indexOf('function prepareNextVideoMediaCandidate()'),
    app.indexOf('function applyPlannedVideoCycle('),
  );
  const applyMedia = app.slice(
    app.indexOf('async function applyVideoProcessing('),
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
  );

  assert.match(app, /const videoStreamActive = MPV_REALTIME_VIDEO_ENABLED[\s\S]*playbackActive[\s\S]*currentMediaIsVideo[\s\S]*videoProcessingEnabled[\s\S]*Boolean\(mediaEffectParams\)/);
  assert.match(prepareNext, /videoEffectsEnabled:\s*plan\.payload\.videoEffectsEnabled/);
  assert.match(finalEffectWindow, /<video[\s\S]*src=\{sourceUrl \?\? undefined\}/);
  assert.match(applyMedia, /!MPV_REALTIME_VIDEO_ENABLED \|\| !mediaCandidate\.videoEffectsEnabled[\s\S]*releaseUnstartedCandidate\(mediaCandidate\)[\s\S]*return/);
  assert.match(applyMedia, /prepare_realtime_video_plan/);
  assert.doesNotMatch(applyMedia, /prepare_media_video_stream_period|video_stream|MediaSource|SourceBuffer/);
  assert.match(app, /function retryVideoProcessing\(\) \{\s*if \(!videoStreamActive\) return;/);
  assert.match(app, /onAction=\{videoStreamActive \? retryVideoProcessing : undefined\}/);
  assert.doesNotMatch(applyMedia, /scope:\s*'video'/);
  assert.doesNotMatch(applyMedia, /audio_variants|ambient_sound_path|['"]both['"]/);
});

test('Original 直播保留时钟与兜底，同时启用 mpv 实时处理调度', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp()'));

  assert.match(app, /const MPV_REALTIME_VIDEO_ENABLED = true/);
  assert.match(finalEffectWindow, /<video[\s\S]*src=\{sourceUrl \?\? undefined\}[\s\S]*muted/);
  assert.match(desktopApp, /const videoStreamActive = MPV_REALTIME_VIDEO_ENABLED/);
  assert.doesNotMatch(app, /DIRECT_VIDEO_SRC_PLAYBACK|video_stream|VideoMseStreamController/);
});

test('最终效果窗口保留单 Original video，视频处理交给 mpv 原生表面', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );

  assert.match(finalEffectWindow, /<video[\s\S]*ref=\{videoRef\}/);
  assert.doesNotMatch(finalEffectWindow, /videoSlotARef|videoSlotBRef|activeVideoSlot/);
  assert.doesNotMatch(finalEffectWindow, /MediaSource|SourceBuffer|VideoMseStreamController|video_stream|commit_media_processing_if_ready|release_media_processing_artifact/);
  assert.match(finalEffectWindow, /createMediaElementSource\(sourceAudio\)/);
  assert.match(finalEffectWindow, /<audio[\s\S]*ref=\{sourceAudioRef\}[\s\S]*src=\{sourceUrl/);
});

test('独立音频候选复用隐藏 A/B 音频槽，并按视频绝对时钟提交', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const activate = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function tryActivatePendingAudioArtifact('),
    finalEffectWindow.indexOf('function tryActivatePendingMediaArtifact('),
  );

  assert.match(finalEffectWindow, /processedAudioARef/);
  assert.match(finalEffectWindow, /processedAudioBRef/);
  assert.match(finalEffectWindow, /pending_audio_artifact_reference/);
  assert.match(finalEffectWindow, /pendingAudioArtifactTimeline/);
  assert.match(activate, /resolveMediaArtifactSwitchTime/);
  assert.match(activate, /commit_audio_media_candidate/);
  assert.match(activate, /scheduleProcessedAudioCrossfade/);
  assert.match(activate, /releaseCommittedAudioSlot/);
  assert.doesNotMatch(activate, /commit_media_processing_if_ready|setActiveVideoSlot/);
});

test('seek 与播放时钟恢复都直接作用于 Original video', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp()'),
  );
  const seek = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function applyPlaybackMediaControl('),
    finalEffectWindow.indexOf('function publishAudioCycleResult('),
  );
  const recovery = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function startPlaybackClockRecovery('),
    finalEffectWindow.indexOf('function handlePlaybackClockFailure('),
  );

  assert.match(seek, /video\.currentTime = sourcePositionSeconds/);
  assert.doesNotMatch(seek, /videoMseControllerRef|controller\.seek\(|seek_media_video_stream/);
  assert.match(recovery, /video\.load\(\)/);
  assert.match(recovery, /video\.currentTime = Math\.min\(Math\.max\(0, resumeAt\), safeEnd\)/);
  assert.doesNotMatch(app, /read_media_video_stream_chunk|MediaSource|SourceBuffer|video_stream/);
});

test('播放快照不再携带旧视频文件候选与 MSE 会话', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const snapshotType = app.slice(
    app.indexOf('type PlaybackSnapshot ='),
    app.indexOf('type InterludeRuntimeMessage'),
  );
  const validator = app.slice(app.indexOf('function isPlaybackSnapshot('), app.indexOf('function isPlaybackItemCompletionResult('));

  assert.doesNotMatch(snapshotType, /pending_video_|pending_media_|current_media_|video_stream|VideoMseStreamSession/);
  assert.doesNotMatch(validator, /pending_video_|pending_media_|current_media_|video_stream|isVideoMseStreamSession/);
});

test('候选局部时间只用于播放器内部，主窗口继续接收源媒体进度', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const publish = app.slice(
    app.indexOf('function publishMediaState()'),
    app.indexOf('function applyPlaybackMediaControl('),
  );

  assert.match(publish, /current_time: positionMs \/ 1_000/);
  assert.match(publish, /duration: durationMs \/ 1_000/);
  assert.match(publish, /const absolutePositionMs = directLoopIndex \* durationMs \+ Math\.min\(durationMs, localPositionMs\)/);
});

test('独立声音 N+1 使用预选变体并同步 Rust 返回的实际 revision', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const prepareAudio = app.slice(
    app.indexOf('async function prepareNextAudioMediaCandidate('),
    app.indexOf('function prepareNextVideoMediaCandidate('),
  );

  assert.match(prepareAudio, /audio_variants: candidate\.audioCyclePlan\.payload\.audioVariants/);
  assert.match(prepareAudio, /const pendingRevision = nextSnapshot\.pending_audio_media_source_revision/);
  assert.match(prepareAudio, /candidate\.timeline = \{[\s\S]*sourceRevision: pendingRevision/);
  assert.doesNotMatch(prepareAudio, /start_media_processing/);
});

test('单项池源音轨原生循环，并按 Original 绝对时钟持续轻量校时', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function MainApplication()'),
  );

  assert.match(finalEffectWindow, /loop=\{\(snapshot\?\.source_media_pool\.length \?\? 0\) === 1\}/);
  assert.match(finalEffectWindow, /resolveSourceAudioSync\([\s\S]*sourceAudio\.currentTime \* 1_000 - positionMs[\s\S]*sourceClock\.loopIndex/);
  assert.match(finalEffectWindow, /sourceAudio\.playbackRate = sync\.playbackRate/);
  assert.match(finalEffectWindow, /sync\.hardRealign[\s\S]*sourceAudio\.currentTime = positionMs \/ 1_000/);
  assert.match(finalEffectWindow, /playback_state === 'playing'[\s\S]*sourceAudio\.paused[\s\S]*sourceAudio\.play\(\)/);
});

test('视频周期目标按源帧率对齐，声音周期保持原毫秒时间线', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');

  assert.match(app, /alignMediaPositionToVideoFrame/);
  assert.match(app, /function createVideoMediaCycleQueue\([\s\S]*resolveSourceBoundedVideoCycleQueueTargets/);
  assert.match(app, /videoFuturePlansRef\.current = createVideoMediaCycleQueue\(/);
  assert.match(app, /audioFuturePlansRef\.current = createMediaCycleQueue\(/);
});

test('停止播放先释放音视频元素句柄，再删除当前与待切换临时文件', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const cleanup = app.slice(
    app.indexOf('function releaseStoppedMediaArtifacts('),
    app.indexOf('function currentProcessedAudioTimeSeconds(', app.indexOf('function releaseStoppedMediaArtifacts(')),
  );
  const control = app.slice(
    app.indexOf('if (isPlaybackControlMessage(event.data))'),
    app.indexOf('if (isAudioOutputBackendMessage(event.data))'),
  );

  assert.match(cleanup, /clearVideoElement/);
  assert.doesNotMatch(cleanup, /videoMseControllerRef|MediaSource|SourceBuffer|video_stream/);
  assert.match(cleanup, /clearProcessedAudioSlot/);
  assert.match(cleanup, /release_audio_media_candidate/);
  assert.doesNotMatch(cleanup, /release_media_processing_artifact|current_video_reference|pending_video_reference/);
  assert.match(cleanup, /current_audio_artifact_reference[\s\S]*pending_audio_artifact_reference/);
  assert.match(control, /event\.data\.action === 'stop'[\s\S]*releaseStoppedMediaArtifacts\(currentSnapshot\)/);
});

test('候选迟到时保持当前输出，成功提交后从最新时钟晋升 N+2', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const scheduler = app.slice(
    app.indexOf('// 定时器只负责唤醒'),
    app.indexOf('// 仅播放中才推送实时参数'),
  );
  const audioCommit = app.slice(
    app.indexOf('function commitCompletedAudioRender('),
    app.indexOf('async function flushPendingMediaApply('),
  );
  const videoCommit = app.slice(
    app.indexOf('function commitPreparedRealtimeVideoCandidate('),
    app.indexOf('function commitCompletedAudioRender('),
  );

  assert.doesNotMatch(scheduler, /validUntilAbsolutePositionMs|discardExpired/);
  assert.match(audioCommit, /advanceIndependentAudioQueue\([\s\S]*mediaStateRef\.current\?\.absolute_position_ms/);
  assert.match(videoCommit, /clock\.absolute_position_ms < candidate\.timeline\.targetAbsolutePositionMs/);
  assert.match(videoCommit, /advanceIndependentVideoQueue\(clock\.absolute_position_ms\)/);
});
