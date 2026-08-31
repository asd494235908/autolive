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

test('PortAudio 插话结束时钟跟随预设变速，WebView 处理缓存保持原速', async () => {
  const { resolveInterludeClockPlaybackRate } = await loadInterludeModule();

  assert.equal(resolveInterludeClockPlaybackRate(1.18, true), 1.18);
  assert.equal(resolveInterludeClockPlaybackRate(0.1, true), 0.5);
  assert.equal(resolveInterludeClockPlaybackRate(3, true), 2);
  assert.equal(resolveInterludeClockPlaybackRate(Number.NaN, true), 1);
  assert.equal(resolveInterludeClockPlaybackRate(1.18, false), 1);
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

test('插话音量界面只使用 0–100 的整数百分比', async () => {
  const {
    interludeVolumeDbToPercent,
    interludeVolumePercentToDb,
  } = await loadInterludeModule();

  assert.equal(interludeVolumeDbToPercent(-60), 0);
  assert.equal(interludeVolumeDbToPercent(0), 100);
  assert.equal(interludeVolumeDbToPercent(12), 100);
  assert.equal(interludeVolumePercentToDb(0), -60);
  assert.ok(Math.abs(interludeVolumePercentToDb(50) - -6.0206) < 0.0001);
  assert.equal(interludeVolumePercentToDb(100), 0);
});

test('首个插话立即播放，后续插话从上一段结束后等待随机间隔', async () => {
  const { nextInterludeAtMs } = await loadInterludeModule();

  assert.equal(nextInterludeAtMs(0, false, 8_000, 13_000, () => 0.5), 0);
  assert.equal(nextInterludeAtMs(4_200, true, 8_000, 13_000, () => 0), 12_200);
  assert.equal(nextInterludeAtMs(4_200, true, 8_000, 13_000, () => 1), 17_200);
});

test('插话声音周期进度按本次随机等待区间独立计算', async () => {
  const { interludeIntervalProgress } = await loadInterludeModule();

  assert.equal(interludeIntervalProgress(2_000, 12_000, 2_000), 0);
  assert.equal(interludeIntervalProgress(2_000, 12_000, 7_000), 50);
  assert.equal(interludeIntervalProgress(2_000, 12_000, 12_000), 100);
  assert.equal(interludeIntervalProgress(2_000, 12_000, 15_000), 100);
  assert.equal(interludeIntervalProgress(2_000, 2_000, 2_000), 100);
});

test('插话声音周期卡片绑定文件等待进度，预设周期继续使用段内进度', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const fileCycleCard = appSource.slice(
    appSource.indexOf('title="插话声音周期"'),
    appSource.indexOf('title="插话预设变化周期"'),
  );
  const presetCycleCard = appSource.slice(
    appSource.indexOf('title="插话预设变化周期"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(appSource, /file_progress_percent: number/);
  assert.match(fileCycleCard, /progress=\{interludeRuntime\?\.file_progress_percent \?\? 0\}/);
  assert.doesNotMatch(fileCycleCard, /progress=\{0\}/);
  assert.match(presetCycleCard, /progress=\{interludeRuntime\?\.progress_percent \?\? 0\}/);
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

test('插话预设变化周期独立支持 10 分钟并按媒体时间推进', async () => {
  const {
    INTERLUDE_PRESET_PERIOD_LIMITS,
    createInterludePresetPeriodPlan,
    advanceInterludePresetPeriodPlan,
    interludePresetPeriodProgress,
  } = await loadInterludeModule();

  assert.deepEqual(INTERLUDE_PRESET_PERIOD_LIMITS, { min: 1_000, max: 600_000 });
  const first = createInterludePresetPeriodPlan(0, 600_000, 600_000);
  assert.deepEqual(first, {
    segment: 1,
    segmentStartMs: 0,
    periodMs: 600_000,
    nextBoundaryMs: 600_000,
  });
  assert.equal(advanceInterludePresetPeriodPlan(first, 599_999, 600_000, 600_000), null);
  assert.equal(interludePresetPeriodProgress(first, 300_000), 50);
  assert.deepEqual(advanceInterludePresetPeriodPlan(first, 600_000, 600_000, 600_000), {
    segment: 2,
    segmentStartMs: 600_000,
    periodMs: 600_000,
    nextBoundaryMs: 1_200_000,
  });
});

test('当前插话文件只展示基本名称并拒绝异常长名称', async () => {
  const { interludeFileNameFromPath } = await loadInterludeModule();

  assert.equal(interludeFileNameFromPath('C:\\media\\直播插话03.mp3'), '直播插话03.mp3');
  assert.equal(interludeFileNameFromPath('/media/nested/voice.wav'), 'voice.wav');
  assert.equal(interludeFileNameFromPath(''), null);
  assert.equal(interludeFileNameFromPath(`C:\\media\\${'a'.repeat(513)}`), null);
});

test('主页显示当前随机文件、预设变化周期，并把插话音量放在插话声音周期卡片', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const audioLane = appSource.slice(
    appSource.indexOf('title="插话声音周期"'),
    appSource.indexOf('desktop-media-domain-lane--video'),
  );

  assert.match(audioLane, /title="插话声音周期"[\s\S]*aria-label="插话音量"/);
  assert.match(audioLane, /aria-valuetext=\{`\$\{interludeVolumePercent\}%`\}/);
  assert.match(audioLane, /min=\{0\}[\s\S]*max=\{100\}[\s\S]*step=\{1\}[\s\S]*value=\{interludeVolumePercent\}/);
  assert.match(audioLane, /interludeVolumePercentToDb\(volumePercent\)/);
  assert.match(audioLane, /\{interludeVolumePercent\}%<\/Typography\.Text>/);
  assert.doesNotMatch(audioLane, /interludeDraft\.volumeDb\.toFixed\(1\).*dB/);
  assert.match(audioLane, /title="插话预设变化周期"/);
  assert.match(audioLane, /title="插话声音预设"[\s\S]*className="desktop-interlude-current-summary"[\s\S]*>当前文件<[\s\S]*currentInterludeFileName[\s\S]*>当前预设<[\s\S]*currentInterludePresetLabel/);
  assert.match(appSource, /const currentInterludeFileName =[\s\S]*: '无';/);
  assert.match(appSource, /const configuredInterludePresetId =[\s\S]*audio_fixed_preset_id[\s\S]*audio_preset_ids\?\.\[0\][\s\S]*const displayedInterludePreset = activeInterludePresets\[0\]/);
  assert.match(audioLane, /aria-label="插话声音预设当前参数"[\s\S]*displayedInterludePreset\.values\[field\.key\]/);
  assert.doesNotMatch(audioLane, /插话开始后展示本段实际参数/);
  assert.match(appSource, /activeInterludePresets\.map\(\(preset\) => `\$\{preset\.label\}（\$\{preset\.id\}）`\)\.join\('、'\)/);
  assert.match(appSource, /fileName: selectedFileName/);
  assert.doesNotMatch(appSource, /file_name: selectedPath/);
});

test('插话文件触发次数与当前文件内预设切换次数使用独立计数', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp()'),
  );
  const audioLane = appSource.slice(
    appSource.indexOf('title="插话声音周期"'),
    appSource.indexOf('desktop-media-domain-lane--video'),
  );

  assert.match(finalWindow, /const interludeFileCycleRef = useRef\(0\)/);
  assert.match(finalWindow, /interludeFileCycleRef\.current \+= 1/);
  assert.match(finalWindow, /file_cycle: interludeFileCycleRef\.current/);
  assert.match(finalWindow, /preset_segment: options\?\.presetSegment \?\?/);
  assert.match(audioLane, /title="插话声音周期"[\s\S]*changes=\{interludeRuntime\?\.file_cycle \?\? 0\}/);
  assert.match(audioLane, /title="插话预设变化周期"[\s\S]*changes=\{Math\.max\(0, \(interludeRuntime\?\.preset_segment \?\? 1\) - 1\)\}/);
  assert.doesNotMatch(audioLane, /changes=\{interludeRuntime\?\.cycle/);
  assert.match(finalWindow, /interludeFileCycleRef\.current = 0;[\s\S]*publishInterludeRuntime\('idle'\)/);
});

test('插话波形使用稳定的本段开始时间，不被周期进度消息持续推迟', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const runtimePublisher = appSource.slice(
    appSource.indexOf('function publishInterludeRuntime('),
    appSource.indexOf('function handleInterludeError'),
  );
  const waveformProjection = appSource.slice(
    appSource.indexOf('const interludeWaveformActive = Boolean('),
    appSource.indexOf('const actualAudioBranches'),
  );

  assert.match(runtimePublisher, /const sentAtMs = Date\.now\(\)/);
  assert.match(runtimePublisher, /started_at_ms: status === 'playing'[\s\S]*previous\.started_at_ms[\s\S]*sentAtMs/);
  assert.match(waveformProjection, /diagnosticMessage\.sent_at_ms >= interludeRuntime\.started_at_ms/);
  assert.doesNotMatch(waveformProjection, /diagnosticMessage\.sent_at_ms >= interludeRuntime\.sent_at_ms/);
});

test('旧快照缺少插话字段时使用 Rust 的过渡默认值', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const buildDraft = appSource.slice(
    appSource.indexOf('function buildInterludeDraft('),
    appSource.indexOf('function FinalEffectWindow()'),
  );

  assert.match(buildDraft, /duckingAttackMs: interlude\?\.ducking_attack_ms \?\? 50/);
  assert.match(buildDraft, /duckingReleaseMs: interlude\?\.ducking_release_ms \?\? 250/);
  assert.match(buildDraft, /duckingDepthDb: interlude\?\.ducking_depth_db \?\? -60/);
  assert.match(appSource, /function toGainValue\(db: number\)\s*\{\s*return db <= -60 \? 0 : Math\.pow\(10, db \/ 20\);\s*\}/);
});

test('随机插话抽屉以独立周期范围编辑并继续保存毫秒契约', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const saveHandler = appSource.slice(
    appSource.indexOf('async function saveInterludeConfig()'),
    appSource.indexOf('const runtimeRemainingMs'),
  );
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(drawer, /<strong>插话声音周期<\/strong>[\s\S]*ariaLabel="插话声音周期最小值（秒）"[^>]*unit="秒"[^>]*step=\{0\.5\}/);
  assert.match(drawer, /ariaLabel="插话声音周期最大值（秒）"[^>]*unit="秒"[^>]*step=\{0\.5\}/);
  assert.match(drawer, /value=\{interludeIntervalMsToSeconds\(interludeDraft\.intervalMinMs\)\}/);
  assert.match(drawer, /value=\{interludeIntervalMsToSeconds\(interludeDraft\.intervalMaxMs\)\}/);
  assert.match(drawer, /intervalMinMs: interludeIntervalSecondsToMs\(value\)/);
  assert.match(drawer, /intervalMaxMs: interludeIntervalSecondsToMs\(value\)/);
  assert.match(saveHandler, /interval_min_ms: interludeDraft\.intervalMinMs/);
  assert.match(saveHandler, /interval_max_ms: interludeDraft\.intervalMaxMs/);
});

test('随机插话抽屉把全部可编辑输入集中在单一配置区', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.equal(drawer.match(/<FeatureDrawerSection/g)?.length, 1);
  assert.match(drawer, /title="插话配置"/);
  assert.match(drawer, /title="插话配置"[\s\S]*aria-label="启用随机插话"[\s\S]*label="插话媒体目录"[\s\S]*ariaLabel="插话声音周期最小值（秒）"[\s\S]*aria-label="插话音轨选择方式"[\s\S]*ariaLabel="原声压低"/);
});

test('随机插话抽屉列出音视频格式并说明视频仅使用音轨', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(appSource, /const SUPPORTED_MEDIA_EXTENSIONS = \[[\s\S]*'mp4'[\s\S]*'m2ts'[\s\S]*'wmv'[\s\S]*'3gp'/);
  assert.match(appSource, /const SUPPORTED_VIDEO_EXTENSIONS = SUPPORTED_MEDIA_EXTENSIONS\.slice\(0, 11\)/);
  assert.match(drawer, /音频：mp3、wav、m4a、aac、ogg、flac/);
  assert.match(drawer, /视频：mp4、mov、mkv、avi、webm、m4v、ts、m2ts、flv、wmv、3gp/);
  assert.match(drawer, /视频仅使用音轨，不显示画面/);
  assert.match(drawer, /label="插话媒体目录"/);
});

test('随机插话前端最多接收 1000 个文件并显示已保存快照数量', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const drawer = appSource.slice(
    appSource.indexOf('title="随机插话"'),
    appSource.indexOf('title="固定话术"'),
  );

  assert.match(appSource, /const MAX_INTERLUDE_AUDIO_FILES = 1_000;/);
  assert.match(appSource, /audioFiles\.length > MAX_INTERLUDE_AUDIO_FILES/);
  assert.match(drawer, /已保存受支持文件 \{snapshot\?\.interlude\?\.audio_count \?\? 0\}\/\{MAX_INTERLUDE_AUDIO_FILES\}/);
  assert.doesNotMatch(drawer, /已保存受支持文件 \{interludeDraft/);
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
  assert.doesNotMatch(buildDraft, /audioVariationMode/);
  assert.match(buildDraft, /audio_variation_period_min_ms \?\? 8_000/);
  assert.match(buildDraft, /audio_variation_period_max_ms \?\? 15_000/);
  assert.match(buildDraft, /audioVariationPeriodMinMs: audioVariationPeriod\.minMs/);
  assert.match(buildDraft, /audioVariationPeriodMaxMs: audioVariationPeriod\.maxMs/);
  assert.match(saveHandler, /audio_preset_ids: audioPresetIds/);
  assert.match(saveHandler, /audio_mix_enabled: interludeDraft\.audioMixEnabled/);
  assert.match(saveHandler, /audio_mix_pick_min: audioMixPickMin/);
  assert.match(saveHandler, /audio_mix_pick_max: audioMixPickMax/);
  assert.match(saveHandler, /audio_variation_mode: 'periodic'/);
  assert.match(saveHandler, /audio_variation_period_min_ms: audioVariationPeriod\.minMs/);
  assert.match(saveHandler, /audio_variation_period_max_ms: audioVariationPeriod\.maxMs/);
  assert.match(drawer, /label="插话声音预设"/);
  assert.match(drawer, /<Checkbox\.Group[\s\S]*AUDIO_VALUE_PRESETS\.map/);
  assert.doesNotMatch(drawer, /AUDIO_VALUE_PRESETS\.filter\(\(preset\) => preset\.id !== 'p21'\)/);
  assert.match(drawer, /aria-label="随机多轨合一"/);
  assert.match(drawer, /最少随机轨数/);
  assert.match(drawer, /最多随机轨数/);
  assert.doesNotMatch(drawer, /每次插话重新随机|插话声音抽样方式/);
  assert.match(drawer, /变化周期最小值/);
  assert.match(drawer, /变化周期最大值/);
  assert.match(drawer, /PortAudio 与 WebView 均应用固定或随机预设与多轨合一/);
  assert.match(drawer, /周期按当前插话媒体时间推进，到期后保持同一文件和播放位置并切换声音预设/);
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

test('最终效果窗按插话媒体时间边界抽样并保持同一文件位置', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp('),
  );
  const sampling = finalWindow.slice(
    finalWindow.indexOf('function sampleInterludeAudioCycle('),
    finalWindow.indexOf('function startInterludePlayback('),
  );
  const startPlayback = finalWindow.slice(
    finalWindow.indexOf('async function startInterludePlayback('),
    finalWindow.indexOf('function publishMediaState()'),
  );

  assert.match(appSource, /function sampleInterludeAudioCycle\(/);
  assert.match(sampling, /sampleAudioCycle\(presetIds,/);
  assert.doesNotMatch(sampling, /allowP21/);
  assert.match(sampling, /const periodic = selectionMode === 'random'/);
  assert.match(sampling, /createInterludePresetPeriodPlan\(0, period\.minMs, period\.maxMs\)/);
  assert.match(sampling, /lastInterludeAudioPresetIdsRef\.current/);
  assert.doesNotMatch(sampling, /setInterval|setTimeout|new Audio|convertFileSrc/);
  assert.match(startPlayback, /const audioCycle = resolveInterludeAudioCycle\(interlude\)/);
  assert.match(startPlayback, /const audio = \{[\s\S]*\.\.\.audioCycle\.values,[\s\S]*current_formant_hz: null/);
  assert.doesNotMatch(startPlayback, /random_change_period_ms: audioPeriodMs/);
  assert.match(startPlayback, /buildAudioVariantsFromCycle\(audio, audioCycle\)/);
  assert.match(startPlayback, /invoke<unknown>\('prepare_webview_interlude'/);
  assert.match(startPlayback, /isPrepareWebViewInterludeResult\(preparedResponse\)/);
  assert.match(startPlayback, /prepared\.state === 'processed'/);
  assert.match(startPlayback, /prepared\.state === 'original_fallback'/);
  assert.match(startPlayback, /advanceInterludePresetPeriodPlan\([\s\S]*switchActiveInterludePreset/);
  assert.match(startPlayback, /if \(!plan\) \{[\s\S]*createInterludePresetPeriodPlan\(mediaPositionMs, period\.minMs, period\.maxMs\)/);
  assert.match(startPlayback, /switch_portaudio_interlude_preset/);
});

test('固定模式每段复用现有 helper 选择指定音轨且不发送混合支路', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp('),
  );
  const sampling = finalWindow.slice(
    finalWindow.indexOf('function sampleInterludeAudioCycle('),
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
  assert.match(finalWindow, /invoke<unknown>\('release_webview_interlude_cache'/);
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
  assert.match(finalWindow, /const candidate = document\.createElement\('audio'\)/);
  assert.match(finalWindow, /candidate\.addEventListener\('canplay'/);
  assert.match(finalWindow, /webviewPrevious = \{/);
  assert.match(finalWindow, /element\.src = url;[\s\S]*void element\.play\(\)\.catch/);
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
  assert.match(appSource, /ref=\{videoRef\}[\s\S]{0,200}crossOrigin="anonymous"[\s\S]{0,200}src=\{managedNativeVideoOwnsPlayback \? undefined : sourceUrl \?\? undefined\}/);
  assert.match(appSource, /ref=\{sourceAudioRef\}\s+crossOrigin="anonymous"\s+src=\{sourceUrl \?\? undefined\}/);
  assert.match(appSource, /ref=\{audioRef\}\s+crossOrigin="anonymous"\s+src=\{audioUrl \?\? undefined\}/);
  assert.match(appSource, /ref=\{processedAudioARef\}\s+crossOrigin="anonymous"/);
  assert.match(appSource, /ref=\{processedAudioBRef\}\s+crossOrigin="anonymous"/);
  assert.match(appSource, /ref=\{interludeAudioRef\}\s+crossOrigin="anonymous"\s+src=\{interludeAudioUrl \?\? undefined\}/);
  assert.match(appSource, /createMediaElementSource\(sourceAudio\)\.connect\(dryGain\)/);
  assert.doesNotMatch(appSource, /createMediaElementSource\(videoA\)|createMediaElementSource\(videoB\)/);
  assert.match(appSource, /插话不会播放/);
});

test('WebView 插话通过共享主轨增益压低所有主音轨并应用攻放过渡', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const graph = appSource.slice(
    appSource.indexOf('// 图只建一次'),
    appSource.indexOf('// 换源：只 resume'),
  );
  const startPlayback = appSource.slice(
    appSource.indexOf('async function startInterludePlayback('),
    appSource.indexOf('function publishMediaState()'),
  );
  const clearPlayback = appSource.slice(
    appSource.indexOf('function clearInterludePlayback('),
    appSource.indexOf('function pauseInterludePlayback()'),
  );

  assert.match(appSource, /const mainProgramGainRef = useRef<GainNode \| null>\(null\)/);
  assert.match(appSource, /function scheduleMainProgramDuck\(targetGain: number, durationMs: number\)/);
  assert.match(graph, /const mainProgramGain = context\.createGain\(\)/);
  assert.match(graph, /high\.connect\(mainProgramGain\)/);
  assert.match(graph, /reverbWet\.connect\(mainProgramGain\)/);
  assert.match(graph, /noiseGain\.connect\(mainProgramGain\)/);
  assert.match(graph, /slotGainA\.connect\(mainProgramGain\)/);
  assert.match(graph, /slotGainB\.connect\(mainProgramGain\)/);
  assert.match(graph, /createMediaElementSource\(audio\)\.connect\(mainProgramGain\)/);
  assert.match(graph, /mainProgramGain\.connect\(mainMediaVolumeGain\)/);
  assert.match(graph, /mainMediaVolumeGain\.connect\(analyser\)/);
  assert.match(graph, /createMediaElementSource\(interlude\)\.connect\(analyser\)/);
  assert.match(startPlayback, /scheduleMainProgramDuck\(duckGainLevelRef\.current, interlude\.ducking_attack_ms\)/);
  assert.match(clearPlayback, /scheduleMainProgramDuck\(1, releaseMs\)/);
});

test('实时诊断降为 4Hz 发送 128 点采样窗口，降低隐藏 WebView 的 CPU 占用', async () => {
  const [appSource, policySource] = await Promise.all([
    readFile(path.join(currentDir, 'App.tsx'), 'utf8'),
    readFile(path.join(currentDir, 'desktop/audio-diagnostic-policy.ts'), 'utf8'),
  ]);

  assert.match(policySource, /DIAGNOSTIC_PUBLISH_INTERVAL_MS = 250;/);
  assert.match(policySource, /DIAGNOSTIC_SAMPLE_COUNT = 128;/);
  assert.match(appSource, /setInterval\(publishDiagnostic, DIAGNOSTIC_PUBLISH_INTERVAL_MS\)/);
  assert.match(appSource, /waveform\.slice\(0, DIAGNOSTIC_SAMPLE_COUNT\)/);
  assert.match(appSource, /spectrum\.slice\(0, DIAGNOSTIC_SAMPLE_COUNT\)/);
});

test('当前版本不从桌面 UI 启动实时话术 Worker', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.doesNotMatch(appSource, /start_speech_to_speech_worker/);
  assert.doesNotMatch(appSource, /get_speech_to_speech_worker_capabilities/);
});

test('实时候选真正播放前保留源视频音频，并预加载候选元素', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /const realtimeAudioPlayingRef = useRef\(false\);/);
  assert.match(appSource, /function handleRealtimeAudioPlaying\(\)/);
  assert.match(appSource, /onPlaying=\{handleRealtimeAudioPlaying\}/);
  assert.match(appSource, /onError=\{handleRealtimeAudioElementError\}/);
  assert.match(appSource, /effectiveAudioSource === 'realtime_variant'\s*&&\s*realtimeAudioPlayingRef\.current/);
  assert.match(appSource, /createMediaElementSource\(audio\)\.connect\(mainProgramGain\)/);
  assert.match(appSource, /createBiquadFilter\(\)/);
  assert.match(appSource, /createDelay\(0\.2\)/);
  assert.match(appSource, /noiseSource\.loop = true/);
  assert.match(appSource, /preload="auto"[\s\S]*?onPlaying=\{handleRealtimeAudioPlaying\}/);
});

test('插话音频预加载，正常播放到结束后才切换，并显示加载失败原因', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /preload="auto"/);
  assert.match(appSource, /if \(interludeActiveRef\.current\) \{[\s\S]*updateActiveInterludePresetPeriod\(\);[\s\S]*return;/);
  assert.match(appSource, /onEnded=\{handleInterludeEnded\}/);
  assert.match(appSource, /onError=\{handleInterludeError\}/);
  assert.match(appSource, /插话音频播放失败/);
});

test('PortAudio 接管时插话进入唯一输出流且 WebView 元素只保留结束时钟', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const startPlayback = appSource.slice(
    appSource.indexOf('async function startInterludePlayback('),
    appSource.indexOf('function publishMediaState()'),
  );

  assert.match(appSource, /invoke<unknown>\('start_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('pause_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('resume_portaudio_interlude'/);
  assert.match(appSource, /invoke<void>\('stop_portaudio_interlude'/);
  assert.match(appSource, /interludeAudio\.muted = interludePortAudioRef\.current/);
  assert.match(appSource, /muted=\{portAudioHardwareEnabled\}/);
  assert.match(appSource, /set_portaudio_media_volume/);
  assert.match(appSource, /set_portaudio_interlude_volume/);
  assert.match(appSource, /主音轨保持播放/);
  assert.ok(
    startPlayback.indexOf("invoke<unknown>('prepare_webview_interlude'")
      < startPlayback.indexOf("invoke<unknown>('start_portaudio_interlude'"),
    'PortAudio 启动前必须先生成 WebView 可播放的音频时钟缓存',
  );
  assert.match(startPlayback, /isSupportedInterludeVideoSource\(selectedPath\)[\s\S]*主音轨保持播放/);
  assert.match(startPlayback, /if \(processedPath\) releaseWebViewInterludeCache\(processedPath\);/);
  assert.doesNotMatch(startPlayback, /interludeAudioUrlRef\.current = selectedUrl/);
  assert.match(startPlayback, /interludeAudio\.playbackRate = resolveInterludeClockPlaybackRate\([\s\S]*audio\.playback_speed,[\s\S]*usePortAudio && !processedPath/);
  assert.match(startPlayback, /switch_portaudio_interlude_preset[\s\S]*interludeAudio\.playbackRate = resolveInterludeClockPlaybackRate\([\s\S]*audio\.playback_speed,[\s\S]*webviewInterludeCachePathRef\.current === null/);
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
