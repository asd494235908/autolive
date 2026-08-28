import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const mainPath = new URL('./main.tsx', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);
const tauriCommandsPath = new URL('../../src-tauri/src/commands.rs', import.meta.url);
const tauriMainPath = new URL('../../src-tauri/src/main.rs', import.meta.url);
const audioMixerPath = new URL('../../src-tauri/src/audio_mixer.rs', import.meta.url);
const audioCycleOutputPath = new URL('../../src-tauri/src/audio_cycle_output.rs', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  const main = await readFile(mainPath, 'utf8');
  const css = await readFile(layoutPath, 'utf8');

  assert.match(app, /<DesktopShell>/);
  const sourceIndex = app.indexOf('area="source"');
  const mediaIndex = app.indexOf('area="media"');
  const outputIndex = app.indexOf('area="output"');
  assert.ok(sourceIndex < mediaIndex && mediaIndex < outputIndex, 'desktop columns stay in source/media/output order');
  const hiddenVideoRef = app.indexOf('ref={pictureInPictureVideoRef}');
  const hiddenVideoStart = app.lastIndexOf('<video', hiddenVideoRef);
  assert.notEqual(hiddenVideoStart, -1, 'Picture-in-Picture media element remains available');
  const hiddenVideoEnd = app.indexOf('/>', hiddenVideoStart);
  const hiddenVideo = app.slice(hiddenVideoStart, hiddenVideoEnd);
  assert.match(hiddenVideo, /aria-hidden="true"/);
  assert.match(hiddenVideo, /position: 'fixed'/);
  assert.match(app, /title="音频"/);
  assert.match(app, /title="插话声音预设"/);
  assert.match(app, /title="画面"/);
  assert.doesNotMatch(app, /title="插话音频"|title="视频"/);
  assert.doesNotMatch(app.slice(app.indexOf('function DesktopApp()')), /实时话术幻化/);
  assert.match(app, /aria-label="声音处理"/);
  assert.match(app, /aria-label="视频处理"/);
  assert.match(app, /<Typography\.Text>视频处理<\/Typography\.Text>/);
  assert.doesNotMatch(app, /应用声音参数/);
  assert.match(app, /mediaVideoBackendStatus/);
  assert.match(app, /audio_processing_status/);
  assert.match(app, /retryVideoProcessing\(\)/);
  assert.doesNotMatch(app, /audioSettingsApplyScope/);
  assert.doesNotMatch(app, /MediaProcessingScope|schedulePeriodMediaRender|schedulePeriodRenderRef/);
  assert.match(app, /audioCapabilityRows/);
  assert.match(app, /<MediaParameterPanels/);
  assert.doesNotMatch(app, /row\.key !== 'spectral_perturbation_percent'/);
  assert.match(app, /!videoProcessingEnabledRef\.current/);
  assert.match(app, /source\.media_kind !== 'video'/);
  assert.match(main, /<ConfigProvider\b/);
  assert.match(main, /<AntApp\b/);
  assert.match(css, /grid-template-columns:\s*320px minmax\(0,\s*1fr\) 272px/);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*1fr\)/);
});

test('高级声音设置只保留参数重新生成，不提供手动声音处理入口', async () => {
  const app = await readFile(appPath, 'utf8');
  const drawerStart = app.indexOf('title="高级声音设置"');
  const drawerEnd = app.indexOf('</FeatureDrawer>', drawerStart);
  const drawer = app.slice(drawerStart, drawerEnd);

  assert.ok(drawerStart >= 0 && drawerEnd > drawerStart);
  assert.match(drawer, /onClick=\{rerollSubtleAudioParams\}[\s\S]*>重新生成本周期参数<\/Button>/);
  assert.doesNotMatch(drawer, /应用声音参数|schedulePeriodMediaRender/);
});

test('当前 UI 的声音只走自动 M4A 候选，视频只走 mpv 正式入口', async () => {
  const app = await readFile(appPath, 'utf8');
  const audioStart = app.indexOf('async function prepareNextAudioMediaCandidate');
  const audioEnd = app.indexOf('function prepareNextVideoMediaCandidate', audioStart);
  const audioPrepare = app.slice(audioStart, audioEnd);
  const videoStart = app.indexOf('async function applyVideoProcessing');
  const videoEnd = app.indexOf('function commitCompletedAudioRender', videoStart);
  const videoApply = app.slice(videoStart, videoEnd);

  assert.ok(audioStart >= 0 && audioEnd > audioStart);
  assert.ok(videoStart >= 0 && videoEnd > videoStart);
  assert.match(audioPrepare, /prepare_audio_media_candidate/);
  assert.doesNotMatch(audioPrepare, /start_media_processing/);
  assert.match(videoApply, /prepare_realtime_video_plan/);
  assert.match(videoApply, /commit_realtime_video_plan/);
  assert.doesNotMatch(videoApply, /start_media_processing|prepare_audio_media_candidate/);
  assert.doesNotMatch(videoApply, /audio_variants|ambient_sound_path|['"]both['"]/);
  assert.doesNotMatch(app, /prepare_media_video_stream|read_media_video_stream|ack_media_video_stream|commit_media_video_stream|VideoMse|video_stream/);
  assert.doesNotMatch(app, /schedulePeriodMediaRender|schedulePeriodRenderRef|audio-manual-/);
});

test('音频与视频卡片始终展示同一 N+1 的真实绝对媒体时钟进度', async () => {
  const app = await readFile(appPath, 'utf8');
  assert.match(app, /mediaCycleClockIdentityRef\.current === mediaCycleClockIdentity/);
  assert.match(app, /getMediaCycleProgressPercent\(nextVideoPlan, mediaCycleAbsolutePositionMs\)/);
  assert.match(app, /getMediaCycleProgressPercent\(nextAudioPlan, mediaCycleAbsolutePositionMs\)/);
  assert.match(app, /const runtimeProgressPercent = scheduledVideoProgressPercent/);
  assert.match(app, /<MediaCycleCard[\s\S]*?title="视频周期"[\s\S]*?range=\{videoPeriodRange\}[\s\S]*?changes=\{runtimeCycle\}[\s\S]*?progress=\{runtimeProgressPercent\}/);
  assert.match(app, /<MediaCycleCard[\s\S]*?title="声音周期"[\s\S]*?range=\{audioPeriodRange\}[\s\S]*?changes=\{audioVariationCycle\}[\s\S]*?progress=\{audioProgressPercent\}/);
  assert.doesNotMatch(app, /progress=\{videoProcessingStatus === 'processing' \? snapshot\?\.video_processing_progress_percent/);
  assert.match(
    app,
    /<Tag color=\{getProcessingStatusColor\(audioProcessingStatus\)\}>\{getProcessingStatusLabel\(audioProcessingStatus\)\}<\/Tag>/,
  );
  assert.doesNotMatch(app, /const videoProcessingProgressPercent =/);
  assert.doesNotMatch(app, /候选迟到，仍在处理/);
});

test('PortAudio Host API selector shows ASIO and only enables it when an ASIO device is enumerated', async () => {
  const app = await readFile(appPath, 'utf8');
  const panel = await readFile(new URL('./desktop/portaudio-device-panel.tsx', import.meta.url), 'utf8');
  const hostApiOptionsMatch = panel.match(
    /aria-label="PortAudio Host API"[\s\S]*?options=\{\[([\s\S]*?)\]\}/,
  );
  const deviceLoad = app.slice(
    app.indexOf("invoke<unknown>('list_audio_output_devices')"),
    app.indexOf("invoke<unknown>('get_default_media_effect_params')"),
  );
  const deviceOptions = panel.slice(panel.indexOf('const visibleDevices'), panel.indexOf('const invalidBuffer'));

  assert.ok(hostApiOptionsMatch, 'PortAudio Host API options remain statically discoverable');
  const hostApiOptions = hostApiOptionsMatch[1];
  assert.match(hostApiOptions, /value:\s*['"]asio['"]/i);
  assert.match(hostApiOptions, /label:\s*hasAsioDevice\s*\?\s*['"]ASIO['"]\s*:\s*['"]ASIO（未检测到设备）['"]/);
  assert.match(hostApiOptions, /disabled:\s*!hasAsioDevice/);
  assert.match(
    app,
    /const hasAsioOutputDevice\s*=\s*audioOutputDevices\.some\(\s*\(device\)\s*=>\s*device\.host_api\.trim\(\)\.toLowerCase\(\)\s*===\s*['"]asio['"]\s*,?\s*\)/,
  );
  assert.match(deviceLoad, /if \(!isAudioOutputDeviceList\(devices\)\) \{[\s\S]*setAudioOutputDevices\(\[\]\)[\s\S]*return;[\s\S]*setAudioOutputDevices\(devices\)/);
  assert.doesNotMatch(deviceLoad, /filter[\s\S]*asio/i);
  assert.doesNotMatch(deviceOptions, /!==\s*['"]asio['"]/i);
  assert.match(deviceOptions, /device\.host_api\.trim\(\)\.toLowerCase\(\)\s*===\s*hostApi/);
  for (const hostApi of ['wasapi', 'mme', 'dsound', 'wdmks']) {
    assert.match(hostApiOptions, new RegExp(`value:\\s*['"]${hostApi}['"]`), `${hostApi} remains available`);
  }
});

test('PortAudio 内存缓冲默认 1024KiB 且允许手动输入到 2048KiB', async () => {
  const app = await readFile(appPath, 'utf8');
  const panel = await readFile(new URL('./desktop/portaudio-device-panel.tsx', import.meta.url), 'utf8');
  const bufferStart = panel.indexOf('<label>内存缓冲</label>');
  const bufferEnd = panel.indexOf('</div>', bufferStart);
  const buffer = panel.slice(bufferStart, bufferEnd);

  assert.match(app, /PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB\s*=\s*1_024/);
  assert.match(buffer, /ariaLabel="PortAudio 内存缓冲区大小"/);
  assert.match(buffer, /min=\{128\}/);
  assert.match(buffer, /max=\{2048\}/);
  assert.doesNotMatch(buffer, /<Select/);
});

test('PortAudio 和 WebView 音频路径默认统一到 44100Hz', async () => {
  const app = await readFile(appPath, 'utf8');
  const outputSync = app.slice(
    app.indexOf('// 最终效果窗自行探测 PortAudio'),
    app.indexOf('// 图只建一次：video'),
  );

  assert.match(app, /PORTAUDIO_SAMPLE_RATE_HZ\s*=\s*44_100/);
  assert.match(app, /new AudioContext\(\{ sampleRate: PORTAUDIO_SAMPLE_RATE_HZ \}\)/);
  assert.match(outputSync, /const targetRate = PORTAUDIO_SAMPLE_RATE_HZ/);
  assert.match(outputSync, /sample_rate_hz: targetRate/);
  assert.doesNotMatch(outputSync, /ctxRate/);
});

test('音高滤镜不再篡改视频时钟，只有 playback_speed 同步驱动画面倍速', async () => {
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(
    app,
    /resolveSynchronizedVideoPlaybackRate\([\s\S]*?params\?\.audio_playback_speed/,
  );
  assert.doesNotMatch(app, /video\.playbackRate\s*=\s*2\s*\*\*/);
});

test('处理候选槽切换使用 30ms 双向 Gain ramp，并在失败时保留旧轨', async () => {
  const app = await readFile(appPath, 'utf8');
  const crossfade = app.slice(
    app.indexOf('function scheduleProcessedAudioCrossfade'),
    app.indexOf('function syncUserAudioSettings'),
  );
  const sync = app.slice(
    app.indexOf('function syncUserAudioSettings'),
    app.indexOf('function setPortAudioHardwareActive'),
  );
  const candidateEffect = app.slice(
    app.indexOf('useEffect(() => {', app.indexOf('function restoreDryAudioOutput')),
    app.indexOf('// 真轨失效回干声'),
  );

  assert.match(app, /AUDIO_PARAM_CROSSFADE_SEC\s*=\s*0\.03|AUDIO_PARAM_CROSSFADE_SEC/);
  assert.match(crossfade, /setValueAtTime/);
  assert.match(crossfade, /linearRampToValueAtTime/);
  assert.match(crossfade, /processedSlotGainARef\.current/);
  assert.match(crossfade, /processedSlotGainBRef\.current/);
  assert.match(sync, /scheduleProcessedAudioCrossfade\(/);
  assert.doesNotMatch(sync, /processedSlotGain[AB]Ref\.current\.gain\.value\s*=/);
  assert.match(candidateEffect, /syncUserAudioSettings\(/);
  assert.match(candidateEffect, /AUDIO_PARAM_CROSSFADE_SEC\s*\*\s*1000/);
  assert.match(candidateEffect, /if \(!processedAudioPlayingRef\.current\) restoreDryAudioOutput\(\)/);
});

test('最终效果窗口切源时对称释放媒体监听器和候选切换定时器', async () => {
  const app = await readFile(appPath, 'utf8');
  const sourceRestoreAt = app.indexOf('const resumeAt = resolvePlaybackResumePosition(');
  const sourceEffect = app.slice(
    app.lastIndexOf('useEffect(() => {', sourceRestoreAt),
    app.indexOf('function restoreDryAudioOutput', sourceRestoreAt),
  );
  const candidateEffect = app.slice(
    app.indexOf('useEffect(() => {', app.indexOf('function restoreDryAudioOutput')),
    app.indexOf('// 真轨失效回干声'),
  );
  const candidateCleanup = candidateEffect.slice(candidateEffect.lastIndexOf('return () => {'));

  assert.match(sourceEffect, /video\.removeEventListener\('loadedmetadata', restore\)/);
  for (const eventName of ['loadedmetadata', 'error', 'seeked', 'playing', 'timeupdate']) {
    assert.match(
      candidateCleanup,
      new RegExp(`standby\\.removeEventListener\\('${eventName}'`),
      `${eventName} listener must be removed by the effect cleanup`,
    );
  }
  assert.match(candidateCleanup, /window\.clearTimeout\(audibleFallbackTimer\)/);
  assert.match(candidateCleanup, /window\.clearTimeout\(crossfadeTimer\)/);
  assert.doesNotMatch(candidateCleanup, /standby\.pause\(\)|active\.pause\(\)/);
});

test('声音 N+1/N+2 只预选参数，提交成功后才升级当前快照，且 PortAudio 不创建 ScriptProcessor', async () => {
  const app = await readFile(appPath, 'utf8');
  const finalEffectWindow = app.slice(
    app.indexOf('function FinalEffectWindow()'),
    app.indexOf('function DesktopApp('),
  );
  const interludeAudioResolver = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function sampleInterludeAudioCycle('),
    finalEffectWindow.indexOf('async function startInterludePlayback('),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp('));
  assert.equal(
    [...finalEffectWindow.matchAll(/sampleAudioCycle\(/g)].length,
    2,
    '最终效果窗口只允许固定和随机两个互斥分支抽样',
  );
  assert.equal([...interludeAudioResolver.matchAll(/sampleAudioCycle\(/g)].length, 2);
  assert.match(interludeAudioResolver, /if \(selectionMode === 'fixed'\) \{[\s\S]*return sample;[\s\S]*const periodic = selectionMode === 'random'/);
  assert.equal(
    [...desktopApp.matchAll(/sampleAudioCycle\(/g)].length,
    2,
    '普通声音只允许当前轮提交和未来计划构建直接抽样',
  );
  assert.match(app, /function sampleAndCommitAudioCycle\(/);
  const commitStart = app.indexOf('function commitAudioCycleSample');
  const commitEnd = app.indexOf('function sampleAndCommitAudioCycle', commitStart);
  const commit = app.slice(commitStart, commitEnd);
  const planStart = app.indexOf('function buildAudioCycleSeed');
  const planEnd = app.indexOf('function buildVideoCycleSeed', planStart);
  const plan = app.slice(planStart, planEnd);
  assert.ok(commitStart >= 0 && commitEnd > commitStart);
  assert.ok(planStart >= 0 && planEnd > planStart);
  assert.match(commit, /appendAudioCycleSnapshot\([\s\S]*at:\s*new Date\(\)\.toISOString\(\)/);
  assert.match(commit, /setAudioActivePresetIds\(sample\.presetIds\)/);
  assert.doesNotMatch(plan, /appendAudioCycleSnapshot|audioCycleSampleRef\.current\s*=/);
  assert.doesNotMatch(plan, /setAudioActivePresetIds/);
  assert.match(app, /target_absolute_position_ms:\s*candidate\.timeline\.targetAbsolutePositionMs/);
  assert.doesNotMatch(app, /target_at_ms|remainingWallMs/);
  assert.match(app, /invokePlaybackSnapshot\('prepare_audio_media_candidate'/);
  assert.match(app, /invokePlaybackSnapshot\('commit_audio_media_candidate'/);
  assert.match(app, /invokePlaybackSnapshot\('discard_audio_media_candidate'/);
  assert.match(app, /invoke\('release_audio_media_candidate'/);
  assert.match(app, /Math\.round\(video\.currentTime \* 1_000\)/);
  const initialStart = app.indexOf("void invoke<unknown>('get_default_media_effect_params')");
  const initialEnd = app.indexOf('return () => {', initialStart);
  const initial = app.slice(initialStart, initialEnd);
  const reset = app.slice(
    app.indexOf('async function resetMediaEffectParams'),
    app.indexOf('function rerollSubtleAudioParams'),
  );
  const switchUpdate = app.slice(
    app.indexOf('async function updateProcessingSwitches'),
    app.indexOf('async function applyVideoProcessing'),
  );
  assert.ok(initialStart >= 0 && initialEnd > initialStart);
  for (const section of [initial, reset]) {
    assert.match(section, /sampleAndCommitAudioCycle\(/);
  }
  assert.doesNotMatch(switchUpdate, /sampleAndCommitAudioCycle\(/);
  assert.match(switchUpdate, /wakeMediaCycleScheduling\('switch'\)/);
  assert.match(app, /const PORTAUDIO_FORMAL_SOURCE_SYNC_READY\s*=\s*true/);
  assert.match(
    app,
    /\(audioOutputBackend\?\.preferred_portaudio \|\| audioOutputBackend\?\.running\)\s*&&\s*!audioOutputBusy/,
    'N1/N2 文件音轨启用时必须清除失败后遗留的 PortAudio preferred 状态',
  );
  assert.doesNotMatch(app, /createScriptProcessor|write_portaudio_pcm/);
  assert.doesNotMatch(app, /portAudioTapRef|portAudioWriteBusyRef/);
  assert.match(app, /const nextActive\s*=\s*requested\s*&&\s*PORTAUDIO_FORMAL_SOURCE_SYNC_READY/);
  assert.match(app, /invokeAudioOutputBackendStatus\(['"]sync_audio_output_source['"]\s*,\s*\{\s*request:\s*\{[\s\S]*absolute_position_ms/);
  assert.match(app, /playback_generation:[\s\S]*loop_index:[\s\S]*position_ms:[\s\S]*duration_ms:[\s\S]*absolute_position_ms:/);
  assert.doesNotMatch(app, /recoverAudioCycleCandidate/);
  assert.match(app, /portAudioHardwareEnabled/);
  assert.match(app, /playback_generation:\s*currentSnapshot\?\.playback_generation \?\? null/);
  assert.match(app, /set_audio_output_backend/);
  assert.match(app, /const AUTO_PORTAUDIO_ENABLED\s*=\s*true/);
  assert.doesNotMatch(app, /aria-label="PortAudio 硬件出口"/);
  assert.doesNotMatch(app, /applyAudioOutputBackend\(checked\)/);
  assert.match(
    app,
    /audioOutputBackend\?\.preferred_portaudio[\s\S]*audioOutputBackend\.hardware_state === 'active'[\s\S]*clearRetryTimer\(\);[\s\S]*return;/,
    '活动 PortAudio 的源同步只由最终效果窗负责，主窗不能并发抢占候选',
  );
});

test('媒体效果参数使用正式模型和 IPC', async () => {
  const app = await readFile(appPath, 'utf8');
  const parameterTypes = await readFile(new URL('./media-parameter-panels/media-parameter-types.ts', import.meta.url), 'utf8');

  assert.match(app, /import \{ AudioParameterControls, MediaParameterPanels, type MediaEffectParams \}/);
  assert.match(parameterTypes, /export interface MediaEffectParams/);
  assert.match(parameterTypes, /advanced: AdvancedEffectParams/);
  assert.match(app, /invoke<unknown>\('get_default_media_effect_params'\)[\s\S]*isMediaEffectParams\(params\)/);
  assert.match(app, /invoke<unknown>\('validate_media_effect_params'[\s\S]*isMediaParameterValidationResult\(validationResponse\)/);
});

test('PortAudio 普通播放只有 audio_cycle_output 一个 PCM 生产者', async () => {
  const [commands, tauriMain, audioMixer, audioCycleOutput] = await Promise.all([
    readFile(tauriCommandsPath, 'utf8'),
    readFile(tauriMainPath, 'utf8'),
    readFile(audioMixerPath, 'utf8'),
    readFile(audioCycleOutputPath, 'utf8'),
  ]);

  assert.doesNotMatch(commands, /write_portaudio_pcm|PortAudioOutput::new|write_stereo_interleaved/);
  assert.doesNotMatch(tauriMain, /write_portaudio_pcm/);
  assert.doesNotMatch(audioMixer, /write_stereo_interleaved|prime_stereo_interleaved/);
  assert.match(audioCycleOutput, /PortAudioOutput::new/);
  assert.match(audioCycleOutput, /write_stereo_interleaved_available/);
  assert.match(audioCycleOutput, /prime_stereo_interleaved_available/);
  assert.match(audioCycleOutput, /linear_crossfade/);
});

test('历史 PCM 候选保留安全替换，普通声音改用独立文件候选', async () => {
  const commands = await readFile(tauriCommandsPath, 'utf8');
  const app = await readFile(appPath, 'utf8');
  const prepareStart = commands.indexOf('pub async fn prepare_audio_cycle_candidate');
  const prepareEnd = commands.indexOf('#[tauri::command]', prepareStart + 20);
  const prepare = commands.slice(prepareStart, prepareEnd);
  assert.ok(prepareStart >= 0 && prepareEnd > prepareStart);
  assert.ok(
    prepare.indexOf('validate_audio_cycle_candidate_request') < prepare.indexOf('begin_audio_mixer_prepare'),
    '必须先验证播放代次/revision，再取消或替换 pending 候选',
  );

  const syncStart = commands.indexOf('fn sync_audio_output_source_blocking');
  const syncEnd = commands.indexOf('\n#[tauri::command]', syncStart);
  const sync = commands.slice(syncStart, syncEnd);
  const recoveryStart = sync.indexOf('if request.recover_unhealthy');
  const firstPrewarm = sync.indexOf('audio_mixer_task_from_snapshot', recoveryStart);
  assert.ok(recoveryStart >= 0 && firstPrewarm > recoveryStart);
  assert.doesNotMatch(sync.slice(recoveryStart, firstPrewarm), /pause_audio_output|stop_audio_mixer|\.clear\(\)/);
  assert.match(app, /prepare_audio_media_candidate/);
  assert.match(app, /commit_audio_media_candidate/);
  assert.match(app, /discard_audio_media_candidate/);
  assert.match(app, /release_audio_media_candidate/);
  assert.match(app, /media_worker_already_running/);
  assert.match(app, /classifyAudioOutputSync\(status\)/);
  assert.match(app, /status\.reason_code/);
  assert.doesNotMatch(app, /`PortAudio 源同步失败，已回退 WebView：\$\{latestFailure\}`/);
  assert.match(app, /audioFuturePlansRef\.current = null/);
  assert.match(app, /advanceIndependentAudioQueue/);
});

test('本周期声音预设可点击打开抽屉并展示完整参数值', async () => {
  const app = await readFile(appPath, 'utf8');
  assert.match(app, /audioPresetDrawerOpen/);
  assert.match(app, /查看当前声音预设参数/);
  assert.match(app, /setAudioPresetDrawerOpen\(true\)/);
  assert.match(app, /当前声音预设/);
  assert.match(app, /AUDIO_PRESET_FIELD_DEFINITIONS\.map/);
  assert.match(app, /preset\.values\[field\.key\]/);
});

test('主页音频卡展示真实出口、处理状态和当前声音预设入口', async () => {
  const app = await readFile(appPath, 'utf8');
  const panelStart = app.indexOf('title="音频"');
  const panelEnd = app.indexOf('title="插话声音预设"', panelStart);
  const panel = app.slice(panelStart, panelEnd);

  assert.ok(panelStart >= 0 && panelEnd > panelStart);
  assert.match(panel, /actualOutput=\{<Tag[\s\S]*?>\{actualAudioOutputLabel\}<\/Tag>\}/);
  assert.match(panel, /processingStatus=\{<Tag[\s\S]*?>\{getProcessingStatusLabel\(audioProcessingStatus\)\}<\/Tag>\}/);
  assert.match(panel, /currentPreset=\{[\s\S]*activeAudioPresets\.length/);
  assert.match(panel, /查看本周期 \{activeAudioPresets\.length\} 套/);
  assert.match(panel, /等待首轮/);
});

test('生成缓存删除需要确认且 PortAudio 源同步保持最新请求串行', async () => {
  const app = await readFile(appPath, 'utf8');
  assert.match(app, /cleanup_local_caches_command/);
  assert.match(app, /删除已生成缓存/);
  assert.match(app, /Popconfirm/);
  assert.match(app, /syncAudioOutputSourceLatest/);
  assert.match(app, /sync\.running/);
  assert.match(app, /sync\.latestRequest/);
});

test('播放快照与 PortAudio 状态轮询低频且不允许请求重叠', async () => {
  const app = await readFile(appPath, 'utf8');

  assert.match(app, /const PLAYBACK_SNAPSHOT_POLL_MS\s*=\s*1_000/);
  assert.match(app, /const AUDIO_OUTPUT_STATUS_POLL_MS\s*=\s*2_000/);
  assert.match(app, /if \(outputStatusPollInFlight\) return;/);
  assert.match(app, /outputStatusPollInFlight = true;/);
  assert.match(app, /outputStatusPollInFlight = false;/);
  assert.doesNotMatch(app, /setInterval\(refreshSnapshot,\s*500\)/);
});

test('诊断原始采样保持 4Hz，主页摘要不再用重复 250ms React 定时器', async () => {
  const [app, diagnosticDisplay, diagnosticPolicy] = await Promise.all([
    readFile(appPath, 'utf8'),
    readFile(new URL('./desktop/audio-diagnostic-display.tsx', import.meta.url), 'utf8'),
    readFile(new URL('./desktop/audio-diagnostic-policy.ts', import.meta.url), 'utf8'),
  ]);

  assert.match(app, /audioDiagnosticDisplayRef\.current\?\.acceptDiagnostic\(event\.data\)/);
  assert.match(app, /<AudioDiagnosticDisplay/);
  assert.doesNotMatch(app, /diagnosticNow|setDiagnosticMessage|setInterval\(\(\)\s*=>\s*setDiagnosticNow/);
  assert.match(diagnosticPolicy, /DIAGNOSTIC_PUBLISH_INTERVAL_MS\s*=\s*250/);
  assert.match(diagnosticPolicy, /DIAGNOSTIC_UI_COMMIT_INTERVAL_MS\s*=\s*1_000/);
  assert.match(diagnosticPolicy, /DIAGNOSTIC_STALE_AFTER_MS\s*=\s*1_500/);
  assert.match(diagnosticDisplay, /drawCurrent\(selected\)/);
  assert.match(diagnosticDisplay, /clearCommitTimer\(\)[\s\S]*commitSummary\(selected, false\)/);
});

test('声音状态只展示快照实际支路数，并兼容旧快照与 PortAudio 回退', async () => {
  const app = await readFile(appPath, 'utf8');

  assert.match(app, /audio_stream_variant_count\?: number \| null;/);
  assert.match(app, /function getActualAudioStreamVariantCount\(/);
  assert.match(app, /实际输出：\{actualAudioOutputLabel\}/);
  assert.match(app, /实际混音：\{actualAudioMixLabel\}/);
  assert.match(app, /未上报（兼容旧快照）/);
  assert.match(app, /`\$\{actualAudioStreamVariantCount\} 条支路`/);
  assert.match(app, /PortAudio 已回退 WebView/);
  assert.doesNotMatch(app, /const audioMixTrackCount/);
});

test('PortAudio 卡分离编辑草稿、已应用配置和运行状态', async () => {
  const panel = await readFile(new URL('./desktop/portaudio-device-panel.tsx', import.meta.url), 'utf8');

  assert.match(panel, /appliedDeviceId: string \| null/);
  assert.match(panel, /appliedMemoryBufferKib: number/);
  assert.match(panel, /const configurationDirty = deviceId !== appliedDeviceId[\s\S]*memoryBufferKib !== appliedMemoryBufferKib/);
  assert.match(panel, />配置目标<[\s\S]*\{appliedConfiguration\}/);
  assert.match(panel, />实际出口<[\s\S]*\{actualOutput\}/);
  assert.match(panel, />处理状态<[\s\S]*configurationDirty \? '待应用' : processingStatus/);
});

test('PortAudio 应用覆盖能力、设备、缓冲、变更和忙碌禁用条件', async () => {
  const panel = await readFile(new URL('./desktop/portaudio-device-panel.tsx', import.meta.url), 'utf8');

  assert.match(panel, /const validDevice = deviceId === null[\s\S]*hostApi === 'all'[\s\S]*visibleDevices\.some/);
  assert.match(panel, /const invalidBuffer = [\s\S]*memoryBufferKib < 128 \|\| memoryBufferKib > 2048/);
  assert.match(panel, /const applyDisabled = !available \|\| invalidBuffer \|\| !validDevice \|\| \(!configurationDirty && running\) \|\| busy/);
  assert.match(panel, /<Button disabled=\{applyDisabled\} loading=\{busy\} onClick=\{onApply\}>应用设置<\/Button>/);
  assert.match(panel, /onClick=\{onTestTone\}>播放测试音<\/Button>/);
});

test('PortAudio 仅在真实应用成功后提交配置，失败保留草稿与旧配置', async () => {
  const app = await readFile(appPath, 'utf8');
  const applyFunction = app.slice(
    app.indexOf('async function applyAudioOutputBackend'),
    app.indexOf('const [mediaEffectParams'),
  );
  const applyHandler = app.slice(
    app.indexOf('onApply={() => {', app.indexOf('<PortAudioDevicePanel')),
    app.indexOf('onTestTone={() => {', app.indexOf('<PortAudioDevicePanel')),
  );

  assert.match(app, /const \[audioOutputDeviceId, setAudioOutputDeviceId\][\s\S]*const \[audioOutputDeviceIdInput, setAudioOutputDeviceIdInput\]/);
  assert.match(app, /deviceId=\{audioOutputDeviceIdInput\}[\s\S]*appliedDeviceId=\{audioOutputDeviceId\}/);
  assert.match(app, /memoryBufferKib=\{audioOutputMemoryKibInput\}[\s\S]*appliedMemoryBufferKib=\{audioOutputMemoryKib\}/);
  assert.match(applyFunction, /await invokeAudioOutputBackendStatus\('set_audio_output_backend'/);
  assert.match(applyFunction, /publishAudioOutputBackend\(status\)[\s\S]*return status/);
  assert.match(applyFunction, /catch \(cause\) \{[\s\S]*setError[\s\S]*return null/);
  assert.doesNotMatch(applyHandler, /setAudioOutputMemoryKib\(value\)/);
  assert.match(applyHandler, /applyAudioOutputBackend\(true, audioOutputDeviceIdInput, value\)\.then\(\(status\) => \{[\s\S]*if \(!status \|\| getActualAudioOutputLabel\(status\) !== 'PortAudio'\) return;[\s\S]*syncAudioOutputConfiguration\(status, true\)/);
});
