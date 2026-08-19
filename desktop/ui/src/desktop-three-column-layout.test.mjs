import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const audioCapabilityPath = new URL('./audio-processing-capabilities.ts', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);
const mainPath = new URL('./main.tsx', import.meta.url);
const tauriCommandsPath = new URL('../../src-tauri/src/commands.rs', import.meta.url);
const tauriMainPath = new URL('../../src-tauri/src/main.rs', import.meta.url);
const audioMixerPath = new URL('../../src-tauri/src/audio_mixer.rs', import.meta.url);
const audioCycleOutputPath = new URL('../../src-tauri/src/audio_cycle_output.rs', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  const audioCapability = await readFile(audioCapabilityPath, 'utf8');
  const main = await readFile(mainPath, 'utf8');
  let css = '';
  try {
    css = await readFile(layoutPath, 'utf8');
  } catch {
    // Keep the failure an assertion failure until the layout stylesheet exists.
  }

  assert.match(app, /desktop-workspace/);
  assert.match(app, /desktop-column-source/);
  assert.match(app, /desktop-column-audio/);
  assert.match(app, /desktop-column-video/);
  assert.doesNotMatch(app, /单源循环播放/);
  assert.doesNotMatch(app, /不生成 N 个离线视频，不创建版本队列/);
  const sourceIndex = app.indexOf('desktop-column-source');
  const audioIndex = app.indexOf('desktop-column-audio');
  const videoIndex = app.indexOf('desktop-column-video');
  assert.ok(sourceIndex < audioIndex && audioIndex < videoIndex, 'desktop columns stay in source/audio/video order');
  const hiddenVideoRef = app.indexOf('ref={pictureInPictureVideoRef}');
  const hiddenVideoStart = app.lastIndexOf('<video', hiddenVideoRef);
  assert.notEqual(hiddenVideoStart, -1, 'Picture-in-Picture media element remains available');
  const hiddenVideoEnd = app.indexOf('/>', hiddenVideoStart);
  const hiddenVideo = app.slice(hiddenVideoStart, hiddenVideoEnd);
  assert.match(hiddenVideo, /aria-hidden="true"/);
  assert.match(hiddenVideo, /position: 'fixed'/);
  assert.match(app, /视频实时参数预览/);
  assert.doesNotMatch(app, /title="视频实时预览"/);
  assert.doesNotMatch(app, /className="desktop-preview-video"/);
  assert.match(app, /aria-label="声音处理"/);
  assert.match(app, /aria-label="实时话术幻化"/);
  assert.match(app, /aria-label="视频处理"/);
  assert.match(app, /应用视频参数/);
  assert.match(app, /应用音频参数/);
  assert.doesNotMatch(app, /应用当前处理参数/);
  assert.match(app, /video_processing_status/);
  assert.match(app, /audio_processing_status/);
  assert.match(app, /applyMediaProcessing\('video'\)/);
  assert.match(app, /applyMediaProcessing\('audio'\)/);
  assert.match(app, /applyMediaProcessing\('both'\)/);
  assert.match(app, /<Descriptions\.Item label="声音处理状态">\{audioProcessingStatus\}<\/Descriptions\.Item>/);
  assert.match(app, /视频状态：\{videoProcessingStatus\}/);
  assert.match(app, /音频实时参数与生效状态/);
  assert.doesNotMatch(app, /title="声音处理参数"/);
  assert.doesNotMatch(app, /音频实时参数预览/);
  assert.doesNotMatch(app, /音频处理能力与生效范围/);
  assert.match(app, /audioCapabilityRows\.map/);
  assert.doesNotMatch(app, /unavailableAudioCapabilityRows/);
  assert.match(app, /基线/);
  assert.match(app, /变化/);
  for (const label of ['总增益', '输入增益', '输出增益', '响度调整', '低频 EQ', '中频 EQ', '高频 EQ', '音高', '变速', '淡入', '淡出', '轻混响', '采样率', '输出码率']) {
    assert.match(audioCapability, new RegExp(label), `audio capability exposes ${label}`);
  }
  assert.doesNotMatch(audioCapability, /buildUnavailableAudioCapabilityRows/);
  assert.match(app, /scope === 'video'[\s\S]*videoProcessingEnabled && !audioProcessingEnabled/);
  assert.match(app, /scope === 'audio'[\s\S]*audioProcessingEnabled && !videoProcessingEnabled/);
  assert.match(app, /scopeEnabled = scope === 'video'/);
  assert.match(app, /<ConfigProvider\b/);
  assert.match(app, /<AntApp>/);
  assert.doesNotMatch(main, /darkAlgorithm|colorPrimary|colorBgBase/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*28fr\)\s+minmax\(0,\s*38fr\)\s+minmax\(0,\s*34fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*800px\)/);
  assert.match(css, /grid-template-columns:\s*1fr/);
});

test('PortAudio Host API selector only exposes available host APIs', async () => {
  const app = await readFile(appPath, 'utf8');
  const hostApiOptionsMatch = app.match(
    /aria-label="PortAudio Host API"[\s\S]*?options=\{\[([\s\S]*?)\]\}/,
  );

  assert.ok(hostApiOptionsMatch, 'PortAudio Host API options remain statically discoverable');
  const hostApiOptions = hostApiOptionsMatch[1];
  assert.doesNotMatch(hostApiOptions, /\bASIO\b|value:\s*['"]asio['"]/i);
  for (const hostApi of ['wasapi', 'mme', 'dsound', 'wdmks']) {
    assert.match(hostApiOptions, new RegExp(`value:\s*['"]${hostApi}['"]`), `${hostApi} remains available`);
  }
});

test('PortAudio 内存缓冲默认 1024KiB 且允许手动输入到 2048KiB', async () => {
  const app = await readFile(appPath, 'utf8');
  const bufferStart = app.indexOf('<InputNumber', app.indexOf('PortAudio Host API'));
  const bufferEnd = app.indexOf('</Space>', bufferStart);
  const buffer = app.slice(bufferStart, bufferEnd);

  assert.match(app, /PORTAUDIO_DEFAULT_MEMORY_BUFFER_KIB\s*=\s*1_024/);
  assert.match(buffer, /aria-label="PortAudio 内存缓冲区大小"/);
  assert.match(buffer, /min=\{PORTAUDIO_MIN_MEMORY_BUFFER_KIB\}/);
  assert.match(buffer, /max=\{PORTAUDIO_MAX_MEMORY_BUFFER_KIB\}/);
  assert.match(buffer, /内存缓冲 128–2048 KiB，默认 1024 KiB/);
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

test('声音周期仅在提交成功后升级当前快照，且 PortAudio 正式路径不创建 ScriptProcessor', async () => {
  const app = await readFile(appPath, 'utf8');
  const sampleCalls = [...app.matchAll(/sampleAudioCycle\(/g)];
  assert.equal(sampleCalls.length, 2, 'App 只允许当前轮提交和下一轮规划直接抽样');
  assert.match(app, /function sampleAndCommitAudioCycle\(/);
  const commitStart = app.indexOf('function commitAudioCycleSample');
  const commitEnd = app.indexOf('function sampleAndCommitAudioCycle', commitStart);
  const commit = app.slice(commitStart, commitEnd);
  const planStart = app.indexOf('function planNextAudioCycle');
  const planEnd = app.indexOf('function postAudioCycleCommand', planStart);
  const plan = app.slice(planStart, planEnd);
  assert.ok(commitStart >= 0 && commitEnd > commitStart);
  assert.ok(planStart >= 0 && planEnd > planStart);
  assert.match(commit, /appendAudioCycleSnapshot\([\s\S]*at:\s*new Date\(\)\.toISOString\(\)/);
  assert.doesNotMatch(plan, /appendAudioCycleSnapshot|audioCycleSampleRef\.current\s*=/);
  assert.match(app, /invoke<PrepareAudioCycleCandidateResult>\('prepare_audio_cycle_candidate'/);
  assert.match(app, /invoke<CommitAudioCycleCandidateResult>\('commit_audio_cycle_candidate'/);
  assert.match(app, /'cancel_audio_cycle_candidate'/);
  assert.match(app, /Math\.round\(video\.currentTime \* 1_000\)/);
  const initialStart = app.indexOf("void invoke<ResearchParams>('get_default_local_research_params')");
  const initialEnd = app.indexOf("void invoke<SpeechToSpeechWorkerCapabilities", initialStart);
  const initial = app.slice(initialStart, initialEnd);
  const reset = app.slice(
    app.indexOf('async function resetResearchParams'),
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
  assert.match(app, /invoke\(['"]sync_audio_output_source['"]\s*,\s*\{\s*request:\s*\{\s*position_ms/);
  assert.match(app, /portAudioHardwareEnabled/);
  assert.match(app, /playback_generation:\s*snapshot\?\.playback_generation/);
  assert.match(app, /set_audio_output_backend/);
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

test('本周期声音预设可点击打开抽屉并展示完整参数值', async () => {
  const app = await readFile(appPath, 'utf8');
  assert.match(app, /audioPresetDrawerOpen/);
  assert.match(app, /查看当前声音预设参数/);
  assert.match(app, /setAudioPresetDrawerOpen\(true\)/);
  assert.match(app, /当前声音预设/);
  assert.match(app, /AUDIO_PRESET_FIELD_DEFINITIONS\.map/);
  assert.match(app, /preset\.values\[field\.key\]/);
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
  assert.match(app, /PortAudio 回退到 WebView；多轨当前未进入正式输出。/);
  assert.doesNotMatch(app, /const audioMixTrackCount/);
});
