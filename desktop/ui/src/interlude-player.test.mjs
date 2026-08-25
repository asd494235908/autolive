import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const helperPath = path.join(currentDir, 'interlude-player.ts');
const require = createRequire(import.meta.url);
const typescript = require('../node_modules/typescript');

async function loadInterludeModule() {
  const source = await readFile(helperPath, 'utf8');
  const transpiled = typescript.transpileModule(source, {
    compilerOptions: {
      module: typescript.ModuleKind.ESNext,
      target: typescript.ScriptTarget.ES2022,
    },
  }).outputText;
  const moduleUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(transpiled)}#${Date.now()}`;
  return import(moduleUrl);
}

async function loadRuntimeParameterScheduler() {
  const presetSource = await readFile(path.join(currentDir, 'audio-value-presets.ts'), 'utf8');
  const presetTranspiled = typescript.transpileModule(presetSource, {
    compilerOptions: {
      module: typescript.ModuleKind.ESNext,
      target: typescript.ScriptTarget.ES2022,
    },
  }).outputText;
  const presetUrl = `data:text/javascript;charset=utf-8,${encodeURIComponent(presetTranspiled)}`;
  const source = await readFile(path.join(currentDir, 'runtime-parameter-scheduler.ts'), 'utf8');
  const transpiled = typescript.transpileModule(source, {
    compilerOptions: {
      module: typescript.ModuleKind.ESNext,
      target: typescript.ScriptTarget.ES2022,
    },
  }).outputText.replaceAll("'./audio-value-presets'", JSON.stringify(presetUrl));
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(transpiled)}#${Date.now()}`);
}

test('resolveBaseAudioSource 按实时变体、处理后原声、原声优先级解析', async () => {
  const { resolveBaseAudioSource } = await loadInterludeModule();

  assert.equal(
    resolveBaseAudioSource({
      realtimeVariantActive: true,
      processedOriginalActive: true,
    }),
    'realtime_variant',
  );
  assert.equal(
    resolveBaseAudioSource({
      realtimeVariantActive: false,
      processedOriginalActive: true,
    }),
    'processed_original',
  );
  assert.equal(
    resolveBaseAudioSource({
      realtimeVariantActive: false,
      processedOriginalActive: false,
    }),
    'original',
  );
});

test('randomIntervalMs 始终落在最小和最大值之间', async () => {
  const { randomIntervalMs } = await loadInterludeModule();

  assert.equal(randomIntervalMs(1_000, 5_000, () => 0), 1_000);
  assert.equal(randomIntervalMs(1_000, 5_000, () => 1), 5_000);
  assert.equal(randomIntervalMs(1_000, 5_000, () => 0.5), 3_000);
});

test('插话触发间隔在秒制 UI 与毫秒 IPC 之间精确换算', async () => {
  const {
    interludeIntervalMsToSeconds,
    interludeIntervalSecondsToMs,
  } = await loadInterludeModule();

  assert.equal(interludeIntervalMsToSeconds(500), 0.5);
  assert.equal(interludeIntervalMsToSeconds(60_000), 60);
  assert.equal(interludeIntervalSecondsToMs(0.5), 500);
  assert.equal(interludeIntervalSecondsToMs(1.2344), 1_234);
  assert.equal(interludeIntervalSecondsToMs(1.2345), 1_235);
});

test('首个插话立即播放，后续插话从上一段结束后等待随机间隔', async () => {
  const { nextInterludeAtMs } = await loadInterludeModule();

  assert.equal(nextInterludeAtMs(0, false, 8_000, 13_000, () => 0.5), 0);
  assert.equal(nextInterludeAtMs(4_200, true, 8_000, 13_000, () => 0), 12_200);
  assert.equal(nextInterludeAtMs(4_200, true, 8_000, 13_000, () => 1), 17_200);
});

test('chooseInterludeIndex 在有多个文件时避免连续重复，空目录返回 null', async () => {
  const { chooseInterludeIndex } = await loadInterludeModule();

  assert.equal(chooseInterludeIndex(0, null, () => 0.2), null);
  assert.equal(chooseInterludeIndex(1, 0, () => 0.8), 0);
  assert.equal(chooseInterludeIndex(3, 1, () => 0), 0);
  assert.equal(chooseInterludeIndex(3, 1, () => 0.49), 0);
  assert.equal(chooseInterludeIndex(3, 1, () => 0.5), 2);
  assert.equal(chooseInterludeIndex(3, 1, () => 0.99), 2);
});

test('shouldPauseInterlude 在视频暂停或系统语音朗读时返回 true', async () => {
  const { shouldPauseInterlude } = await loadInterludeModule();

  assert.equal(
    shouldPauseInterlude({
      playbackState: 'playing',
      fixedSpeechActive: false,
    }),
    false,
  );
  assert.equal(
    shouldPauseInterlude({
      playbackState: 'paused',
      fixedSpeechActive: false,
    }),
    true,
  );
  assert.equal(
    shouldPauseInterlude({
      playbackState: 'playing',
      fixedSpeechActive: true,
    }),
    true,
  );
});

test('resolvePlaybackAudioSource 使用当前视频处理状态作为旧快照回退', async () => {
  const { resolvePlaybackAudioSource } = await loadInterludeModule();

  assert.equal(
    resolvePlaybackAudioSource({
      effectiveAudioSource: null,
      currentAudioSource: 'original',
      currentVideoSource: 'processed',
    }),
    'processed_original',
  );
});

test('插话输入范围与 Rust 契约一致', async () => {
  const { INTERLUDE_LIMITS } = await loadInterludeModule();

  assert.deepEqual(INTERLUDE_LIMITS, {
    intervalMinMs: { min: 500, max: 60_000 },
    volumeDb: { min: -60, max: 12 },
    duckingDepthDb: { min: -60, max: 0 },
    duckingAttackMs: { min: 0, max: 1_000 },
    duckingReleaseMs: { min: 0, max: 3_000 },
  });
});

test('旧快照缺少插话字段时使用 Rust 的过渡默认值', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const buildDraft = appSource.slice(
    appSource.indexOf('function buildInterludeDraft('),
    appSource.indexOf('function FinalEffectWindow()'),
  );

  assert.match(buildDraft, /duckingAttackMs: interlude\?\.ducking_attack_ms \?\? 50/);
  assert.match(buildDraft, /duckingReleaseMs: interlude\?\.ducking_release_ms \?\? 250/);
});

test('随机插话抽屉以秒编辑触发间隔并继续保存毫秒契约', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(drawer, /ariaLabel="插话最小间隔（秒）"[^>]*unit="秒"[^>]*step=\{0\.5\}/);
  assert.match(drawer, /ariaLabel="插话最大间隔（秒）"[^>]*unit="秒"[^>]*step=\{0\.5\}/);
  assert.match(drawer, /value=\{interludeIntervalMsToSeconds\(interludeDraft\.intervalMinMs\)\}/);
  assert.match(drawer, /value=\{interludeIntervalMsToSeconds\(interludeDraft\.intervalMaxMs\)\}/);
  assert.match(drawer, /intervalMinMs: interludeIntervalSecondsToMs\(value\)/);
  assert.match(drawer, /intervalMaxMs: interludeIntervalSecondsToMs\(value\)/);
  assert.match(saveHandler, /interval_min_ms: interludeDraft\.intervalMinMs/);
  assert.match(saveHandler, /interval_max_ms: interludeDraft\.intervalMaxMs/);
});

test('插话配置保存保留 Tauri 结构化失败原因并在抽屉内展示', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );

  assert.match(appSource, /const \{ message: messageApi \} = AntApp\.useApp\(\)/);
  assert.match(saveHandler, /setInterludeDirty\(false\);[\s\S]*void messageApi\.success\('保存成功'\)/);
  assert.match(saveHandler, /getDisplayErrorMessage\(cause, '保存插话配置失败'\)/);
  assert.match(saveHandler, /setInterludeSaveError\(message\)/);
  assert.doesNotMatch(saveHandler, /cause instanceof Error/);
  assert.match(appSource, /interludeSaveError \? <Alert type="error" showIcon message=\{interludeSaveError\}/);
});

test('保存关闭或不可用配置不清理当前插话，只取消后续调度', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );
  const schedulerStart = appSource.indexOf("const interlude = currentSnapshot?.interlude ?? null;", appSource.indexOf('function FinalEffectWindow()'));
  const scheduler = appSource.slice(schedulerStart, appSource.indexOf('  }, [sourceUrl]);', schedulerStart));
  const unavailableConfig = scheduler.slice(
    scheduler.indexOf('if (\n        !interlude'),
    scheduler.indexOf('const currentClockMs'),
  );

  assert.doesNotMatch(saveHandler, /clearInterludePlayback|stop_portaudio_interlude/);
  assert.match(scheduler, /if \(currentSnapshot\.playback_state === 'stopped'\) \{\s*if \(interludeActiveRef\.current \|\| interludeStartingRef\.current\) \{\s*clearInterludePlayback\(\{ resetSchedule: true \}\);\s*\}\s*return;/);
  assert.match(unavailableConfig, /interlude\.status !== 'ready'/);
  assert.match(unavailableConfig, /nextInterludeAtMsRef\.current = null/);
  assert.doesNotMatch(unavailableConfig, /clearInterludePlayback|stop_portaudio_interlude/);
  assert.ok(
    scheduler.indexOf('shouldPauseInterlude({') < scheduler.indexOf('interlude.status !== \'ready\''),
    '暂停中的当前插话必须先暂停，不能因新配置不可用而绕过暂停语义',
  );
  assert.match(appSource, /const interludeReleaseMsRef = useRef\(0\)/);
  assert.match(appSource, /const releaseMs = options\?\.releaseMs \?\? interludeReleaseMsRef\.current/);
  assert.match(appSource, /releaseMs: interludeReleaseMsRef\.current/);
  assert.match(appSource, /interludeReleaseMsRef\.current = interlude\.ducking_release_ms/);
  assert.match(appSource, /interludeReleaseMsRef\.current = 0/);
  assert.doesNotMatch(appSource, /releaseMs: interlude\?\.ducking_release_ms/);
});

test('插话保存独立的 22 项随机预设池、混轨和变化周期契约', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const buildDraft = appSource.slice(
    appSource.indexOf('function buildInterludeDraft('),
    appSource.indexOf('function FinalEffectWindow()'),
  );
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.doesNotMatch(appSource, /INTERLUDE_DEFAULT_AUDIO_PRESET_IDS/);
  assert.match(appSource, /return presetIds\.length > 0 \? presetIds : \[\.\.\.DEFAULT_AUDIO_VALUE_PRESET_IDS\]/);
  assert.match(buildDraft, /audioPresetIds: normalizeInterludeAudioPresetIds\(interlude\?\.audio_preset_ids\)/);
  assert.match(buildDraft, /audioMixEnabled: interlude\?\.audio_mix_enabled \?\? false/);
  assert.match(buildDraft, /audioMixPickMin:[\s\S]*DEFAULT_AUDIO_MIX_PICK_MIN/);
  assert.match(buildDraft, /audio_mix_pick_max \?\? DEFAULT_AUDIO_MIX_PICK_MAX/);
  assert.match(buildDraft, /audioMixPickMax,/);
  assert.match(buildDraft, /audioVariationMode: interlude\?\.audio_variation_mode === 'periodic' \? 'periodic' : 'each_playback'/);
  assert.match(buildDraft, /audio_variation_period_min_ms \?\? 8_000/);
  assert.match(buildDraft, /audio_variation_period_max_ms \?\? 15_000/);
  assert.match(buildDraft, /audioVariationPeriodMinMs: audioVariationPeriod\.minMs/);
  assert.match(buildDraft, /audioVariationPeriodMaxMs: audioVariationPeriod\.maxMs/);
  assert.match(saveHandler, /audio_preset_ids: audioPresetIds/);
  assert.match(saveHandler, /audio_mix_enabled: interludeDraft\.audioMixEnabled/);
  assert.match(saveHandler, /audio_mix_pick_min: audioMixPickMin/);
  assert.match(saveHandler, /audio_mix_pick_max: audioMixPickMax/);
  assert.match(saveHandler, /audio_variation_mode: interludeDraft\.audioVariationMode/);
  assert.match(saveHandler, /audio_variation_period_min_ms: audioVariationPeriod\.minMs/);
  assert.match(saveHandler, /audio_variation_period_max_ms: audioVariationPeriod\.maxMs/);
  assert.match(drawer, /label="插话声音预设"/);
  assert.match(drawer, /<Checkbox\.Group[\s\S]*AUDIO_VALUE_PRESETS\.map/);
  assert.doesNotMatch(drawer, /AUDIO_VALUE_PRESETS\.filter\(\(preset\) => preset\.id !== 'p21'\)/);
  assert.match(drawer, /aria-label="随机多轨合一"/);
  assert.match(drawer, /最少随机轨数/);
  assert.match(drawer, /最多随机轨数/);
  assert.match(drawer, /每次插话重新随机/);
  assert.match(drawer, /按周期更新/);
  assert.match(drawer, /变化周期最小值/);
  assert.match(drawer, /变化周期最大值/);
  assert.match(drawer, /PortAudio 与 WebView 均应用固定或随机预设与多轨合一/);
  assert.match(drawer, /WebView 本地处理失败时明确回退插话原声/);
});

test('插话支持固定音轨、所选池随机与一键全选，并保存选择模式契约', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const buildDraft = appSource.slice(
    appSource.indexOf('function buildInterludeDraft('),
    appSource.indexOf('function FinalEffectWindow()'),
  );
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(appSource, /type InterludeAudioSelectionMode = 'fixed' \| 'random'/);
  assert.match(buildDraft, /audioSelectionMode: interlude\?\.audio_selection_mode === 'fixed' \? 'fixed' : 'random'/);
  assert.match(buildDraft, /audioFixedPresetId: normalizeInterludeAudioPresetIds\(\[interlude\?\.audio_fixed_preset_id \?\? 'p01'\]\)\[0\]/);
  assert.match(saveHandler, /audio_selection_mode: interludeDraft\.audioSelectionMode/);
  assert.match(saveHandler, /audio_fixed_preset_id: audioFixedPresetId/);
  assert.match(drawer, /固定选择/);
  assert.match(drawer, /从所选音轨随机/);
  assert.match(drawer, /aria-label="插话固定声音预设"/);
  assert.match(drawer, /value=\{interludeDraft\.audioFixedPresetId\}/);
  assert.match(drawer, /aria-label="全选插话声音预设"/);
  assert.match(drawer, /audioPresetIds: AUDIO_VALUE_PRESETS\.map\(\(preset\) => preset\.id\)/);
  assert.match(drawer, />全选<\/Button>/);
  assert.match(drawer, /interludeDraft\.audioSelectionMode === 'fixed' \? \([\s\S]*aria-label="插话固定声音预设"[\s\S]*\) : \([\s\S]*aria-label="随机多轨合一"/);
});

test('最终效果窗按每段或周期边界抽样，PortAudio 发送单轨或全部混合支路', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp('),
  );
  const sampling = finalWindow.slice(
    finalWindow.indexOf('function resolveInterludeAudioCycle('),
    finalWindow.indexOf('function startInterludePlayback('),
  );
  const startPlayback = finalWindow.slice(
    finalWindow.indexOf('async function startInterludePlayback('),
    finalWindow.indexOf('function publishMediaState()'),
  );

  assert.doesNotMatch(appSource, /function sampleInterludeAudioCycle\(/);
  assert.match(sampling, /sampleAudioCycle\(presetIds,/);
  assert.doesNotMatch(sampling, /allowP21/);
  assert.match(sampling, /audio_variation_mode === 'periodic'/);
  assert.match(sampling, /nextInterludeAudioVariationAtMsRef\.current/);
  assert.match(sampling, /samplePeriodMsInRange\(/);
  assert.match(sampling, /lastInterludeAudioPresetIdsRef\.current/);
  assert.doesNotMatch(sampling, /setInterval|setTimeout|new Audio|convertFileSrc/);
  assert.match(startPlayback, /const audioCycle = resolveInterludeAudioCycle\(interlude, performance\.now\(\)\)/);
  assert.match(startPlayback, /const audio = \{[\s\S]*\.\.\.audioCycle\.values,[\s\S]*random_change_period_ms: audioPeriodMs,[\s\S]*current_formant_hz: null/);
  assert.match(startPlayback, /buildAudioVariantsFromCycle\(audio, audioCycle\)/);
  assert.match(startPlayback, /invoke<PrepareWebViewInterludeResult>\('prepare_webview_interlude'/);
  assert.match(startPlayback, /prepared\.state === 'processed'/);
  assert.match(startPlayback, /prepared\.state === 'original_fallback'/);
  assert.match(finalWindow, /if \(interludeActiveRef\.current \|\| interludeStartingRef\.current\) return;[\s\S]*startInterludePlayback\(interlude\)/);
});

test('固定模式每段复用现有 helper 选择指定音轨且不发送混合支路', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp('),
  );
  const sampling = finalWindow.slice(
    finalWindow.indexOf('function resolveInterludeAudioCycle('),
    finalWindow.indexOf('function startInterludePlayback('),
  );
  const startPlayback = finalWindow.slice(
    finalWindow.indexOf('async function startInterludePlayback('),
    finalWindow.indexOf('function publishMediaState()'),
  );

  assert.match(sampling, /const selectionMode = interlude\.audio_selection_mode === 'fixed' \? 'fixed' : 'random'/);
  assert.match(sampling, /sampleAudioCycle\(\[fixedPresetId\], \{[\s\S]*mixEnabled: false/);
  assert.doesNotMatch(sampling, /allowP21/);
  assert.doesNotMatch(sampling, /function sampleFixed/);
  assert.match(startPlayback, /const audioVariants = interlude\.audio_selection_mode !== 'fixed'[\s\S]*\? buildAudioVariantsFromCycle\(audio, audioCycle\)[\s\S]*: \[\]/);
  assert.match(startPlayback, /audio_variants: audioVariants/);
});

test('插话与普通声音共用选择规则，p21 显式可选且多轨仍限制 1–4 条', async () => {
  const { getAudioValuePreset, sampleAudioCycle } = await loadRuntimeParameterScheduler();
  const constantRandom = () => 0;
  const p21 = sampleAudioCycle(['p01', 'p21'], {
    mixEnabled: false,
    pickMin: 1,
    pickMax: 2,
    previousPresetIds: ['p01'],
    random: constantRandom,
  });
  assert.deepEqual(p21.presetIds, ['p21']);
  assert.deepEqual(p21.values, getAudioValuePreset('p21').values);

  const p01 = sampleAudioCycle(['p01', 'p21'], {
    mixEnabled: false,
    pickMin: 1,
    pickMax: 2,
    previousPresetIds: ['p21'],
    random: constantRandom,
  });
  assert.deepEqual(p01.presetIds, ['p01']);

  const mixed = sampleAudioCycle(['p01', 'p02', 'p03', 'p21'], {
    mixEnabled: true,
    pickMin: 2,
    pickMax: 2,
    previousPresetIds: ['p01'],
    random: constantRandom,
  });
  assert.equal(mixed.presetIds.length, 2);
  assert.ok(mixed.presetIds.includes('p21'));
  assert.ok(!mixed.presetIds.includes('p01'));
  assert.equal(mixed.variants.length, 2);
  assert.deepEqual(mixed.weights, [0.5, 0.5]);
});

test('WebView 插话处理缓存覆盖完成、错误、替换、停止和卸载释放路径', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp('),
  );
  const shutdown = finalWindow.slice(
    finalWindow.indexOf('function shutdownInterludePlayback()'),
    finalWindow.indexOf('function clearInterludePlayback('),
  );
  const channelLifecycle = finalWindow.slice(
    finalWindow.indexOf('const handlePageHide = () =>'),
    finalWindow.indexOf('  }, []);', finalWindow.indexOf('const handlePageHide = () =>')),
  );

  assert.match(finalWindow, /const webviewInterludeCachePathRef = useRef<string \| null>\(null\)/);
  assert.match(finalWindow, /invoke<ReleaseWebViewInterludeCacheResult>\('release_webview_interlude_cache'/);
  assert.match(finalWindow, /releaseWebViewInterludeCache\(processedPath\)/);
  assert.match(finalWindow, /handleInterludeEnded[\s\S]*clearInterludePlayback/);
  assert.match(finalWindow, /handleInterludeError[\s\S]*clearInterludePlayback/);
  assert.match(shutdown, /interludeOperationRef\.current \+= 1/);
  assert.match(shutdown, /interludePortAudioRef\.current \|\| interludeStartingRef\.current/);
  assert.match(shutdown, /invoke<void>\('stop_portaudio_interlude'/);
  assert.match(shutdown, /releaseCurrentWebViewInterludeCache\(\)/);
  assert.match(shutdown, /interludeActiveRef\.current = false/);
  assert.match(channelLifecycle, /handlePageHide[\s\S]*shutdownInterludePlayback\(\)/);
  assert.match(channelLifecycle, /return \(\) => \{[\s\S]*shutdownInterludePlayback\(\)/);
  assert.match(finalWindow, /if \(operation !== interludeOperationRef\.current\)[\s\S]*releaseWebViewInterludeCache/);
  assert.match(finalWindow, /if \(interludeActiveRef\.current \|\| interludeStartingRef\.current\) return/);
});

test('WebView 与 PortAudio 插话都消费 set_interlude_config 返回的同一快照', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const commandSource = await readFile(new URL('../../src-tauri/src/commands.rs', import.meta.url), 'utf8');
  const saveCommand = commandSource.slice(
    commandSource.indexOf('pub fn set_interlude_config('),
    commandSource.indexOf('pub async fn start_portaudio_interlude('),
  );
  const portAudioStart = commandSource.slice(
    commandSource.indexOf('fn start_portaudio_interlude_blocking('),
    commandSource.indexOf('pub fn pause_portaudio_interlude('),
  );

  assert.match(saveCommand, /playback\.set_interlude_snapshot\(snapshot\.clone\(\)\)/);
  assert.match(portAudioStart, /let interlude = &snapshot\.interlude/);
  assert.match(appSource, /const interlude = currentSnapshot\?\.interlude \?\? null/);
  assert.doesNotMatch(saveCommand, /audio_output_control\(\)\?\.set_interlude/);
});

test('接入 Web Audio 的本地媒体会在 src 前启用匿名 CORS，避免跨源音轨静音', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /audioContextCleanupTimerRef/);
  assert.match(appSource, /ref=\{videoRef\}\s+crossOrigin="anonymous"\s+src=\{sourceUrl\}/);
  assert.match(appSource, /ref=\{audioRef\}\s+crossOrigin="anonymous"\s+src=\{audioUrl \?\? undefined\}/);
  assert.match(appSource, /ref=\{processedAudioARef\}\s+crossOrigin="anonymous"/);
  assert.match(appSource, /ref=\{processedAudioBRef\}\s+crossOrigin="anonymous"/);
  assert.match(appSource, /ref=\{interludeAudioRef\}\s+crossOrigin="anonymous"\s+src=\{interludeAudioUrl \?\? undefined\}/);
  assert.match(appSource, /createMediaElementSource\(video\)\.connect\(dryGain\)/);
  assert.match(appSource, /插话不会播放/);
});

test('实时诊断以至少 20Hz 发送 128 点采样窗口，避免曲线跳变', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /const DIAGNOSTIC_PUBLISH_INTERVAL_MS = 50;/);
  assert.match(appSource, /const DIAGNOSTIC_SAMPLE_COUNT = 128;/);
  assert.match(appSource, /waveform\.slice\(0, DIAGNOSTIC_SAMPLE_COUNT\)/);
  assert.match(appSource, /spectrum\.slice\(0, DIAGNOSTIC_SAMPLE_COUNT\)/);
});

test('实时音频为 Worker 处理和候选换轨预留足够安全余量', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /const REALTIME_AUDIO_SAFETY_LEAD_MS = 6_000;/);
  assert.match(appSource, /const REALTIME_AUDIO_WORKER_TIMEOUT_MS = 5_000;/);
  assert.match(appSource, /const safetyLeadMs = REALTIME_AUDIO_SAFETY_LEAD_MS;/);
  assert.match(appSource, /timeout_ms: REALTIME_AUDIO_WORKER_TIMEOUT_MS,/);
});

test('实时候选真正播放前保留源视频音频，并预加载候选元素', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /const realtimeAudioPlayingRef = useRef\(false\);/);
  assert.match(appSource, /function handleRealtimeAudioPlaying\(\)/);
  assert.match(appSource, /onPlaying=\{handleRealtimeAudioPlaying\}/);
  assert.match(appSource, /onError=\{handleRealtimeAudioElementError\}/);
  assert.match(appSource, /effectiveAudioSource === 'realtime_variant'\s*&&\s*realtimeAudioPlayingRef\.current/);
  assert.match(appSource, /createMediaElementSource\(audio\)\.connect\(analyser\)/);
  assert.match(appSource, /createBiquadFilter\(\)/);
  assert.match(appSource, /createDelay\(0\.2\)/);
  assert.match(appSource, /noiseSource\.loop = true/);
  assert.match(appSource, /preload="auto"[\s\S]*?onPlaying=\{handleRealtimeAudioPlaying\}/);
});

test('插话音频预加载，正常播放到结束后才切换，并显示加载失败原因', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /preload="auto"/);
  assert.match(appSource, /if \(interludeActiveRef\.current \|\| interludeStartingRef\.current\) return;/);
  assert.match(appSource, /onEnded=\{handleInterludeEnded\}/);
  assert.match(appSource, /onError=\{handleInterludeError\}/);
  assert.match(appSource, /插话音频播放失败/);
});

test('PortAudio 接管时插话进入唯一输出流且 WebView 元素只保留结束时钟', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /invoke<void>\('start_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('pause_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('resume_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('stop_portaudio_interlude'/);
  assert.match(appSource, /interludeAudio\.muted = interludePortAudioRef\.current \|\| muted/);
  assert.match(appSource, /muted=\{userMuted \|\| portAudioHardwareEnabled\}/);
  assert.match(appSource, /主音轨保持播放/);
});

test('单项循环保留插话，多项换源前清理插话，插话调度使用独立时钟', async () => {
  const { buildInterludeScheduleKey } = await loadInterludeModule();
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const restartSource = appSource.slice(
    appSource.indexOf('function restartToNextLoop'),
    appSource.indexOf('function restartAtBoundary'),
  );

  assert.equal(buildInterludeScheduleKey(7, '/tmp/video.mp4'), '7:/tmp/video.mp4');
  assert.match(appSource, /buildInterludeScheduleKey\(currentSnapshot\.playback_generation, sourceKey\)/);
  assert.match(appSource, /const currentClockMs = performance\.now\(\);/);
  assert.match(
    restartSource,
    /if \(restartImmediately\) \{[\s\S]*restartCurrentPlayback[\s\S]*\} else \{[\s\S]*clearInterludePlayback[\s\S]*complete_playback_item/,
  );
});
