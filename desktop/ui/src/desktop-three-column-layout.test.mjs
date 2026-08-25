import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);
const tauriCommandsPath = new URL('../../src-tauri/src/commands.rs', import.meta.url);
const tauriMainPath = new URL('../../src-tauri/src/main.rs', import.meta.url);
const audioMixerPath = new URL('../../src-tauri/src/audio_mixer.rs', import.meta.url);
const audioCycleOutputPath = new URL('../../src-tauri/src/audio_cycle_output.rs', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  const css = await readFile(layoutPath, 'utf8');

  assert.match(app, /<DesktopShell>/);
  const sourceIndex = app.indexOf('area="source"');
  const videoIndex = app.indexOf('area="video"');
  const audioIndex = app.indexOf('area="audio-output"');
  assert.ok(sourceIndex < videoIndex && videoIndex < audioIndex, 'desktop columns stay in source/video/audio-output order');
  const hiddenVideoRef = app.indexOf('ref={pictureInPictureVideoRef}');
  const hiddenVideoStart = app.lastIndexOf('<video', hiddenVideoRef);
  assert.notEqual(hiddenVideoStart, -1, 'Picture-in-Picture media element remains available');
  const hiddenVideoEnd = app.indexOf('/>', hiddenVideoStart);
  const hiddenVideo = app.slice(hiddenVideoStart, hiddenVideoEnd);
  assert.match(hiddenVideo, /aria-hidden="true"/);
  assert.match(hiddenVideo, /position: 'fixed'/);
  assert.match(app, /音视频处理 · 实时参数/);
  assert.doesNotMatch(app.slice(app.indexOf('function DesktopApp()')), /实时话术幻化/);
  assert.match(app, /aria-label="声音处理"/);
  assert.match(app, /aria-label="视频处理"/);
  assert.match(app, /应用视频/);
  assert.match(app, /应用声音参数/);
  assert.match(app, /video_processing_status/);
  assert.match(app, /audio_processing_status/);
  assert.match(app, /applyMediaProcessing\('video'\)/);
  assert.match(app, /audioSettingsApplyScope/);
  assert.match(app, /applyMediaProcessing\('both'\)/);
  assert.match(app, /audioCapabilityRows/);
  assert.match(app, /<MediaParameterPanels/);
  assert.doesNotMatch(app, /row\.key !== 'spectral_perturbation_percent'/);
  assert.match(app, /scope === 'video'[\s\S]*videoProcessingEnabledRef\.current && !audioProcessingEnabledRef\.current/);
  assert.match(app, /scope === 'audio'[\s\S]*audioProcessingEnabledRef\.current && !videoProcessingEnabledRef\.current/);
  assert.match(app, /scopeEnabled = scope === 'video'/);
  assert.match(app, /<ConfigProvider\b/);
  assert.match(app, /<AntApp>/);
  assert.match(css, /grid-template-columns:\s*320px minmax\(0,\s*1fr\) 272px/);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)/);
  assert.match(css, /@media\s*\(max-width:\s*900px\)/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*1fr\)/);
});

test('高级声音应用在视频开启时复用音视频处理且不被视频开关禁用', async () => {
  const app = await readFile(appPath, 'utf8');
  const drawerStart = app.indexOf('title="高级声音设置"');
  const drawerEnd = app.indexOf('</FeatureDrawer>', drawerStart);
  const drawer = app.slice(drawerStart, drawerEnd);

  assert.ok(drawerStart >= 0 && drawerEnd > drawerStart);
  assert.match(app, /const audioSettingsApplyScope: MediaProcessingScope = videoProcessingEnabled \? 'both' : 'audio';/);
  assert.match(drawer, /applyMediaProcessing\(audioSettingsApplyScope, undefined, true\)/);
  assert.match(drawer, /loading=\{mediaProcessingBusy === audioSettingsApplyScope\}/);
  assert.match(drawer, /audioSettingsApplyScope === 'both' \? '音视频同时应用' : '应用声音参数'/);

  const applyCallIndex = drawer.indexOf('applyMediaProcessing(audioSettingsApplyScope, undefined, true)');
  const buttonStart = drawer.lastIndexOf('<Button', applyCallIndex);
  const buttonEnd = drawer.indexOf('</Button>', applyCallIndex);
  const button = drawer.slice(buttonStart, buttonEnd);
  assert.doesNotMatch(button, /videoProcessingEnabled/);
  assert.match(button, /runtimeResourceBusy/);
});

test('高级声音人工应用提示保存成功且自动周期保持静默', async () => {
  const app = await readFile(appPath, 'utf8');
  const applyStart = app.indexOf('async function applyMediaProcessing');
  const applyEnd = app.indexOf('async function flushPendingMediaApply', applyStart);
  const apply = app.slice(applyStart, applyEnd);
  const schedulerStart = app.indexOf('function schedulePeriodMediaRender');
  const schedulerEnd = app.indexOf('schedulePeriodRenderRef.current = schedulePeriodMediaRender', schedulerStart);
  const scheduler = app.slice(schedulerStart, schedulerEnd);

  assert.ok(applyStart >= 0 && applyEnd > applyStart);
  assert.match(apply, /showSaveSuccess = false/);
  assert.match(apply, /if \(showSaveSuccess\) void messageApi\.success\('保存成功'\)/);
  assert.match(scheduler, /applyMediaProcessing\(scope, params\)/);
  assert.doesNotMatch(scheduler, /showSaveSuccess|, true/);
});

test('主参数面板同时展示视频与声音周期的真实进度', async () => {
  const app = await readFile(appPath, 'utf8');
  const panelTitleIndex = app.indexOf('title="音视频处理 · 实时参数"');
  const panelStart = app.lastIndexOf('<DesktopPanel', panelTitleIndex);
  const panelEnd = app.indexOf('</DesktopPanel>', panelStart);
  const panel = app.slice(panelStart, panelEnd);

  assert.ok(panelTitleIndex >= 0 && panelStart >= 0 && panelEnd > panelStart);
  assert.match(
    app,
    /const nextVideoPlan\s*=\s*videoFuturePlansRef\.current\?\.\[0\][\s\S]*?const runtimeProgressPercent\s*=\s*!nextVideoPlan\s*\|\|\s*nextVideoPlan\.periodMediaMs\s*<=\s*0[\s\S]*?Math\.max\(0,\s*Math\.min\(100,[\s\S]*?nextVideoPlan\.periodMediaMs\s*-\s*runtimeRemainingMs[\s\S]*?nextVideoPlan\.periodMediaMs/,
  );
  assert.match(
    app,
    /const nextAudioPlan\s*=\s*audioFuturePlansRef\.current\?\.\[0\][\s\S]*?const audioProgressPercent\s*=\s*!nextAudioPlan\s*\|\|\s*nextAudioPlan\.periodMediaMs\s*<=\s*0[\s\S]*?Math\.max\(0,\s*Math\.min\(100,[\s\S]*?nextAudioPlan\.periodMediaMs\s*-\s*audioRemainingMs[\s\S]*?nextAudioPlan\.periodMediaMs/,
  );
  assert.match(panel, /视频周期[\s\S]*?videoPeriodRange[\s\S]*?runtimeCycle/);
  assert.match(panel, /percent=\{runtimeProgressPercent\}/);
  assert.match(panel, /status=\{runtimeActive\s*&&\s*nextVideoPlan\s*\?\s*'active'\s*:\s*'normal'\}/);
  assert.match(panel, /声音周期[\s\S]*?audioPeriodRange[\s\S]*?audioVariationCycle/);
  assert.match(
    panel,
    /<Tag color=\{getProcessingStatusColor\(audioProcessingStatus\)\}>\{getProcessingStatusLabel\(audioProcessingStatus\)\}<\/Tag>/,
  );
  assert.match(panel, /percent=\{audioProgressPercent\}/);
  assert.match(panel, /status=\{audioPeriodActive\s*&&\s*nextAudioPlan\s*\?\s*'active'\s*:\s*'normal'\}/);
  assert.match(app, /desktop-status-line"><span>声音周期<\/span>/);
});

test('PortAudio Host API selector shows ASIO and only enables it when an ASIO device is enumerated', async () => {
  const app = await readFile(appPath, 'utf8');
  const hostApiOptionsMatch = app.match(
    /aria-label="PortAudio Host API"[\s\S]*?options=\{\[([\s\S]*?)\]\}/,
  );
  const deviceLoad = app.slice(
    app.indexOf("invoke<AudioOutputDevice[]>('list_audio_output_devices')"),
    app.indexOf("invoke<MediaEffectParams>('get_default_media_effect_params')"),
  );
  const deviceOptions = app.slice(
    app.indexOf('options={audioOutputDevices'),
    app.indexOf('onChange={(value) => {', app.indexOf('options={audioOutputDevices')),
  );

  assert.ok(hostApiOptionsMatch, 'PortAudio Host API options remain statically discoverable');
  const hostApiOptions = hostApiOptionsMatch[1];
  assert.match(hostApiOptions, /value:\s*['"]asio['"]/i);
  assert.match(hostApiOptions, /label:\s*hasAsioOutputDevice\s*\?\s*['"]ASIO['"]\s*:\s*['"]ASIO（运行时未检测到设备）['"]/);
  assert.match(hostApiOptions, /disabled:\s*!hasAsioOutputDevice/);
  assert.match(
    app,
    /const hasAsioOutputDevice\s*=\s*audioOutputDevices\.some\(\s*\(device\)\s*=>\s*device\.host_api\.trim\(\)\.toLowerCase\(\)\s*===\s*['"]asio['"]\s*,?\s*\)/,
  );
  assert.match(deviceLoad, /setAudioOutputDevices\(\s*devices\.filter\(\(device\)\s*=>\s*typeof device\.host_api\s*===\s*['"]string['"]\)\s*,?\s*\)/);
  assert.doesNotMatch(deviceLoad, /filter[\s\S]*asio/i);
  assert.doesNotMatch(deviceOptions, /!==\s*['"]asio['"]/i);
  assert.match(deviceOptions, /device\.host_api\.trim\(\)\.toLowerCase\(\)\s*===\s*audioOutputHostApiFilter/);
  for (const hostApi of ['wasapi', 'mme', 'dsound', 'wdmks']) {
    assert.match(hostApiOptions, new RegExp(`value:\\s*['"]${hostApi}['"]`), `${hostApi} remains available`);
  }
});

test('PortAudio 内存缓冲默认 1024KiB 且允许手动输入到 2048KiB', async () => {
  const app = await readFile(appPath, 'utf8');
  const bufferStart = app.indexOf('<FeatureDrawerField label="内存缓冲"', app.indexOf('PortAudio Host API'));
  const bufferEnd = app.indexOf('</FeatureDrawerField>', bufferStart);
  const buffer = app.slice(bufferStart, bufferEnd);

  assert.match(app, /PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB\s*=\s*1_024/);
  assert.match(buffer, /ariaLabel="PortAudio 内存缓冲区大小"/);
  assert.match(buffer, /min=\{PORTAUDIO_MIN_MEMORY_BUFFER_KIB\}/);
  assert.match(buffer, /max=\{PORTAUDIO_MAX_MEMORY_BUFFER_KIB\}/);
  assert.match(buffer, /范围 128–2048 KiB，默认 1024 KiB/);
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
    finalEffectWindow.indexOf('function resolveInterludeAudioCycle('),
    finalEffectWindow.indexOf('async function startInterludePlayback('),
  );
  const desktopApp = app.slice(app.indexOf('function DesktopApp('));
  assert.equal(
    [...finalEffectWindow.matchAll(/sampleAudioCycle\(/g)].length,
    2,
    '最终效果窗口只允许固定和随机两个互斥分支抽样',
  );
  assert.equal([...interludeAudioResolver.matchAll(/sampleAudioCycle\(/g)].length, 2);
  assert.match(interludeAudioResolver, /if \(selectionMode === 'fixed'\) \{[\s\S]*return sample;[\s\S]*const periodic/);
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
  assert.match(app, /target_absolute_position_ms:\s*plan\.targetAbsolutePositionMs/);
  assert.doesNotMatch(app, /target_at_ms|remainingWallMs/);
  assert.match(app, /invoke<PrepareAudioCycleCandidateResult>\('prepare_audio_cycle_candidate'/);
  assert.match(app, /invoke<CommitAudioCycleCandidateResult>\('commit_audio_cycle_candidate'/);
  assert.match(app, /'cancel_audio_cycle_candidate'/);
  assert.match(app, /Math\.round\(video\.currentTime \* 1_000\)/);
  const initialStart = app.indexOf("void invoke<MediaEffectParams>('get_default_media_effect_params')");
  const initialEnd = app.indexOf('return () => {', initialStart);
  const initial = app.slice(initialStart, initialEnd);
  const reset = app.slice(
    app.indexOf('async function resetMediaEffectParams'),
    app.indexOf('function rerollSubtleAudioParams'),
  );
  const switchUpdate = app.slice(
    app.indexOf('async function updateProcessingSwitches'),
    app.indexOf('function schedulePeriodMediaRender'),
  );
  assert.ok(initialStart >= 0 && initialEnd > initialStart);
  for (const section of [initial, reset, switchUpdate]) {
    assert.match(section, /sampleAndCommitAudioCycle\(/);
  }
  assert.match(app, /const PORTAUDIO_FORMAL_SOURCE_SYNC_READY\s*=\s*true/);
  assert.doesNotMatch(app, /createScriptProcessor|write_portaudio_pcm/);
  assert.doesNotMatch(app, /portAudioTapRef|portAudioWriteBusyRef/);
  assert.match(app, /const nextActive\s*=\s*requested\s*&&\s*PORTAUDIO_FORMAL_SOURCE_SYNC_READY/);
  assert.match(app, /invoke\(['"]sync_audio_output_source['"]\s*,\s*\{\s*request:\s*\{[\s\S]*absolute_position_ms/);
  assert.match(app, /playback_generation:[\s\S]*loop_index:[\s\S]*position_ms:[\s\S]*duration_ms:[\s\S]*absolute_position_ms:/);
  assert.doesNotMatch(app, /recoverAudioCycleCandidate/);
  assert.match(app, /portAudioHardwareEnabled/);
  assert.match(app, /playback_generation:\s*snapshot\?\.playback_generation/);
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
  assert.match(app, /invoke<MediaEffectParams>\('get_default_media_effect_params'\)/);
  assert.match(app, /invoke<MediaParameterValidationResult>\('validate_media_effect_params'/);
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

test('候选版本先校验再替换，PCM 恢复先预热再停旧生产者', async () => {
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
  assert.match(app, /audio_candidate_not_due/);
  assert.match(app, /audio_candidate_not_ready/);
  assert.match(app, /audio_candidate_commit_busy/);
  assert.match(app, /audio_candidate_recovery_in_progress/);
  assert.match(app, /audio_mixer_candidate_superseded/);
  assert.match(app, /classifyAudioOutputSync\(status\)/);
  assert.match(app, /status\.reason_code/);
  assert.doesNotMatch(app, /`PortAudio 源同步失败，已回退 WebView：\$\{latestFailure\}`/);
  assert.match(app, /刷新最新声音快照失败[\s\S]*advanceIndependentAudioQueue/);
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

test('主页右栏展示实际多轨状态和当前声音预设名称', async () => {
  const app = await readFile(appPath, 'utf8');
  const panelStart = app.indexOf('<DesktopPanel title="普通声音处理"');
  const panelEnd = app.indexOf('</DesktopPanel>', panelStart);
  const panel = app.slice(panelStart, panelEnd);

  assert.ok(panelStart >= 0 && panelEnd > panelStart);
  assert.match(panel, /<span>多轨与预设<\/span>[\s\S]*\{actualAudioMixLabel\}/);
  assert.match(panel, /<span>当前预设<\/span>[\s\S]*activeAudioPresets\.map/);
  assert.match(panel, /\{preset\.label\}/);
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
