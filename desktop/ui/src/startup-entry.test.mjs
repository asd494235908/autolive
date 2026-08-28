import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

async function readSource(fileName) {
  return readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
}

async function loadCspNonceModule() {
  const source = await readSource('cspNonce.ts');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  return import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);
}

test('HTML 入口在 React 执行前提供静态启动反馈', async () => {
  const source = await readSource('../index.html');

  assert.match(source, /<title>GpAutoLive<\/title>/);
  assert.match(source, /id="startup-splash"/);
  assert.match(source, /class="startup-splash-title">GpAutoLive<\/div>/);
  assert.match(source, /正在启动桌面端/);
  assert.match(source, /startup-splash-spinner/);
  assert.match(source, /animation/);
});

test('静态与 React 启动壳使用一致的深色主题色', async () => {
  const html = await readSource('../index.html');
  const loading = await readSource('startup-loader.tsx');

  for (const color of ['#0b0b0f', '#f7f7f8', '#9a9aa3', '#31d7aa']) {
    assert.match(html, new RegExp(color));
    assert.match(loading, new RegExp(color));
  }
  assert.match(html, /color-scheme:\s*dark/);
  assert.doesNotMatch(html, /linear-gradient|radial-gradient/);
  assert.doesNotMatch(loading, /linear-gradient|radial-gradient/);
});

test('主窗口锁定 body 滚动，避免懒加载期间出现白闪和页面级滚动', async () => {
  const html = await readSource('../index.html');

  assert.match(html, /body\s*\{[^}]*overflow:\s*hidden;/);
  assert.doesNotMatch(html, /body\s*\{[^}]*overflow-y:\s*auto;/);
});

test('Tauri 主窗口使用参考图尺寸、深色背景和无原生装饰', async () => {
  const [source, commands] = await Promise.all([
    readFile(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/src/commands.rs', import.meta.url), 'utf8'),
  ]);
  const config = JSON.parse(source);
  const mainWindow = config.app.windows.find((window) => window.label === 'main');

  assert.ok(mainWindow);
  assert.equal(config.productName, 'GpAutoLive');
  assert.equal(mainWindow.title, 'GpAutoLive');
  assert.match(commands, /\.title\("GpAutoLive 最终效果"\)/);
  assert.equal(mainWindow.width, 1728);
  assert.equal(mainWindow.height, 1044);
  assert.equal(mainWindow.minWidth, 960);
  assert.equal(mainWindow.minHeight, 680);
  assert.equal(mainWindow.backgroundColor, '#0b0b0f');
  assert.equal(mainWindow.decorations, false);
  assert.equal(mainWindow.resizable, true);
  assert.equal(mainWindow.center, true);
});

test('React 入口懒加载 App 并提供 Suspense 与错误恢复', async () => {
  const source = await readSource('main.tsx');

  assert.match(source, /lazy\(\(\) => import\(['"]\.\/App['"]\)\)/);
  assert.match(source, /<Suspense\b/);
  assert.match(source, /<StartupErrorBoundary\b/);
  assert.match(source, /import \{ DesktopWindowFrame \} from ['"]\.\/desktop\/desktop-shell['"]/);
  assert.match(source, /getCurrentWindow\(\)\.label === ['"]final-effect['"]/);
  assert.match(source, /isFinalEffectWindow \? content : <DesktopWindowFrame>\{content\}<\/DesktopWindowFrame>/);
  assert.doesNotMatch(source, /import App from ['"]\.\/App['"]/);
});

test('Tauri 生产构建使用相对资源路径，确保 CSS 和懒加载资源可从本地协议加载', async () => {
  const viteConfig = await readFile(new URL('../vite.config.ts', import.meta.url), 'utf8');

  assert.match(viteConfig, /base:\s*['"]\.\/['"]/);
});

test('最终效果页面由 Tauri 窗口标签识别，不把查询参数拼进本地资源路径', async () => {
  const app = await readSource('App.tsx');

  assert.match(app, /import \{ getCurrentWindow \} from ['"]@tauri-apps\/api\/window['"]/);
  assert.match(app, /getCurrentWindow\(\)\.label === ['"]final-effect['"]/);
  assert.doesNotMatch(app, /URLSearchParams\(window\.location\.search\)/);
});

test('Ant Design 动态样式复用 Tauri 注入的 CSP nonce', async () => {
  const main = await readSource('main.tsx');
  const nonce = await readSource('cspNonce.ts');

  assert.match(main, /<ConfigProvider[\s\S]*csp=\{\{\s*nonce:\s*getCspNonce\(\)\s*\}\}/);
  assert.match(nonce, /querySelector<HTMLStyleElement>\('style\[nonce\]'\)/);
  assert.match(nonce, /style\?\.nonce/);
});

test('CSP nonce 读取器从静态 style 标签读取 nonce，并在无 DOM 时安全降级', async () => {
  const { getCspNonce } = await loadCspNonceModule();
  const previousDocument = globalThis.document;

  try {
    globalThis.document = {
      querySelector(selector) {
        assert.equal(selector, 'style[nonce]');
        return { nonce: '123456' };
      },
    };
    assert.equal(getCspNonce(), '123456');

    delete globalThis.document;
    assert.equal(getCspNonce(), undefined);
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
  }
});

test('Ant Design 根外壳由主窗口入口拥有，使启动状态和业务页共享主题', async () => {
  const source = await readSource('main.tsx');

  assert.match(source, /App as AntApp/);
  assert.match(source, /ConfigProvider/);
  assert.match(source, /<ConfigProvider(?:\s[^>]*)?>/);
  assert.match(source, /<AntApp\s+message=/);
});

test('导入媒体使用单一同步 guard 原子提交有序多选池，不打开窗口不自动播放', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(source, /const \[importVideoBusy, setImportVideoBusy\] = useState\(false\);/);
  assert.match(source, /const importVideoInFlightRef = useRef\(false\);/);
  assert.match(
    importVideo,
    /if \(importVideoInFlightRef\.current\) return;[\s\S]*importVideoInFlightRef\.current = true;[\s\S]*setImportVideoBusy\(true\);/,
  );
  assert.match(
    importVideo,
    /try \{[\s\S]*await open\(\{[\s\S]*multiple:\s*true[\s\S]*probe_local_videos[\s\S]*request:\s*\{\s*paths\s*\}/,
  );
  assert.match(
    importVideo,
    /Array\.isArray\(selection\)[\s\S]*typeof selection === 'string'[\s\S]*\[selection\]/,
  );
  assert.doesNotMatch(importVideo, /probe_local_video['"]/);
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.match(
    importVideo,
    /catch \(cause\) \{[\s\S]*导入媒体失败[\s\S]*\} finally \{[\s\S]*importVideoInFlightRef\.current = false;[\s\S]*setImportVideoBusy\(false\);[\s\S]*\}/,
  );
  assert.doesNotMatch(importVideo, /setTimeout|waitForAbortableDelay/);
});

test('导入按钮在完整导入链路中显示 loading 并禁止重复点击', async () => {
  const [source, pool] = await Promise.all([
    readSource('App.tsx'),
    readSource('desktop/playback-pool-panel.tsx'),
  ]);

  assert.match(source, /importBusy=\{importVideoBusy\}/);
  assert.match(source, /importDisabled=\{importVideoBusy \|\| runtimeResourceBusy\}/);
  assert.match(pool, /loading=\{importBusy\}/);
  assert.match(pool, /disabled=\{importDisabled\}/);
  assert.match(pool, /导入媒体/);
  assert.doesNotMatch(pool, /导入媒体并播放/);
});

test('桌面启动和退出清理跨会话生成缓存', async () => {
  const [main, commands] = await Promise.all([
    readFile(new URL('../../src-tauri/src/main.rs', import.meta.url), 'utf8'),
    readFile(new URL('../../src-tauri/src/commands.rs', import.meta.url), 'utf8'),
  ]);

  assert.match(main, /RunEvent::Ready[\s\S]*cleanup_stale_generated_caches\(app_handle\)/);
  assert.match(main, /RunEvent::ExitRequested[\s\S]*cleanup_stale_generated_caches\(app_handle\)/);
  assert.match(commands, /fn cleanup_stale_generated_caches[\s\S]*media-compatibility[\s\S]*webview-interlude/);
});

test('主页提供独立播放按钮，有源即可反复点击', async () => {
  const source = await readSource('App.tsx');
  assert.match(source, /const canStartPlayback = Boolean\(currentSource\);/);
  assert.doesNotMatch(source, /canStartPlayback = Boolean\(currentSource\) &&/);
  assert.match(source, /async function startPlaybackFromHome\(\)/);
  assert.match(source, /openFinalEffectWindowFromHome\(\);[\s\S]*runPlaybackAction\('resume', 'start_playback'\)/);
  assert.match(source, /onClick=\{\(\) => void startPlaybackFromHome\(\)\}/);
  assert.match(source, /disabled=\{!canStartPlayback\}/);
});

test('启动不恢复上次导入的媒体，必须由用户手动选择文件', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );
  const finalEffectWindowStart = source.indexOf('function FinalEffectWindow()');
  const finalEffectVideoStart = source.indexOf('<video', finalEffectWindowStart);
  const finalEffectVideo = source.slice(
    finalEffectVideoStart,
    source.indexOf('/>', finalEffectVideoStart) + 2,
  );

  assert.doesNotMatch(source, /sourceRestoreAttemptedRef/);
  assert.doesNotMatch(source, /autolive\.source\.path/);
  assert.ok(finalEffectVideoStart > finalEffectWindowStart);
  assert.doesNotMatch(finalEffectVideo, /autoPlay/);
  assert.match(importVideo, /async function importVideo\(\)/);
  assert.match(importVideo, /const selection = await open\(/);
  assert.match(importVideo, /probe_local_videos[\s\S]*request:\s*\{\s*paths\s*\}/);
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.doesNotMatch(importVideo, /voice|model|worker|prepare/i);
});

test('打开文件选择器不会触发旧语音准备流程', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );
  assert.match(importVideo, /const selection =/);
  assert.match(importVideo, /await open\(/);
  assert.doesNotMatch(importVideo, /voiceClone|voice_clone|XTTS|Demucs|Whisper|prepareVoiceClone/i);
});

test('导入媒体先确保 media，资源就绪后再探测，不自动播放', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(importVideo, /ensureRuntimeResources\('media',[\s\S]*probe_local_videos[\s\S]*request:\s*\{\s*paths\s*\}/);
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.match(source, /pendingRuntimeActionRef/);
  assert.match(source, /token/);
  assert.match(source, /RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}position:\s*['"]fixed['"]/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}overflow:\s*['"]hidden['"]/);
});

test('导入文件选择器开放确认的视频与音频扩展名', async () => {
  const source = await readSource('App.tsx');
  const declaration = source.match(/const SUPPORTED_MEDIA_EXTENSIONS = \[([\s\S]*?)\] as const;/);

  assert.ok(declaration);
  assert.deepEqual(
    [...declaration[1].matchAll(/'([^']+)'/g)].map((match) => match[1]),
    ['mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'ts', 'm2ts', 'flv', 'wmv', '3gp', 'mp3', 'wav', 'm4a', 'aac', 'ogg', 'flac'],
  );
  assert.match(source, /filters: \[\{ name: '媒体文件', extensions: \[\.\.\.SUPPORTED_MEDIA_EXTENSIONS\] \}\]/);
  assert.match(source, /multiple:\s*true/);
});

test('高级声音抽屉展示本地 DSP 参数且主窗口不暴露实时话术', async () => {
  const source = await readSource('App.tsx');
  const capability = await readSource('audio-processing-capabilities.ts');
  const cycleCard = await readSource('desktop/media-cycle-card.tsx');
  for (const field of ['input_gain_db', 'output_gain_db', 'loudness_adjustment_db', 'low_eq_db', 'mid_eq_db', 'high_eq_db', 'noise_reduction_percent', 'phase_perturbation_percent', 'vibrato_frequency_hz', 'environment_noise_percent', 'sample_rate_hz', 'output_bitrate_kbps']) {
    assert.match(capability, new RegExp(field), field);
  }
  assert.match(source, /title="高级声音设置"/);
  assert.match(source, /audioCapabilityRows/);
  assert.doesNotMatch(source, /row\.key !== 'spectral_perturbation_percent'/);
  assert.match(source, /<MediaParameterPanels/);
  assert.doesNotMatch(source, /应用声音参数/);
  assert.doesNotMatch(source.slice(source.indexOf('function DesktopApp()')), /实时话术幻化/);
  assert.match(source, /title="声音周期"/);
  assert.match(source, /title="视频周期"/);
  assert.match(cycleCard, /ariaLabel=\{`\$\{title\}最小秒`\}/);
  assert.match(cycleCard, /ariaLabel=\{`\$\{title\}最大秒`\}/);
  assert.doesNotMatch(source, /title="声音处理参数"/);
  assert.doesNotMatch(source, /aria-label="音频 MFCC 维度"/);
  assert.doesNotMatch(source, /aria-label="音频音色库"/);
});

test('声音预设随机化会把完整抽样值写回当前参数并刷新只读展示', async () => {
  const source = await readSource('App.tsx');
  const commitStart = source.indexOf('function commitAudioCycleSample');
  const commit = source.slice(commitStart, source.indexOf('function sampleAndCommitAudioCycle', commitStart));
  const samplerStart = source.indexOf('function sampleAndCommitAudioCycle');
  const sampler = source.slice(samplerStart, source.indexOf('function nextMediaCyclePlanId', samplerStart));
  const rerollStart = source.indexOf('function rerollSubtleAudioParams');
  const reroll = source.slice(rerollStart, source.indexOf('const sourceMediaPool', rerollStart));

  assert.ok(commitStart >= 0 && samplerStart >= 0 && rerollStart >= 0);
  assert.match(sampler, /sampleAudioCycle\(audioValuePresetIdsRef\.current/);
  assert.match(sampler, /commitAudioCycleSample\(/);
  assert.match(commit, /const next = \{[\s\S]*?audio:\s*\{[\s\S]*?\.\.\.sample\.values/);
  assert.match(commit, /mediaEffectParamsRef\.current = next;[\s\S]*setMediaEffectParams\(next\)/);
  assert.match(reroll, /sampleAndCommitAudioCycle\(\)/);
  assert.doesNotMatch(reroll, /prepare_audio_media_candidate|start_media_processing/);
  assert.match(source, /value=\{mediaEffectParams\.audio\}/);
});

test('停止、暂停或真实媒体时钟不健康时停止实时参数调度', async () => {
  const source = await readSource('App.tsx');

  assert.match(source, /const playbackRequested = snapshot\?\.playback_state\?\.toLowerCase\(\) === ['"]playing['"]/);
  assert.match(source, /const playbackActive = playbackRequested[\s\S]*mediaState\.clock_health === ['"]healthy['"][\s\S]*!mediaState\.paused/);
  assert.match(source, /const runtimeActive = MPV_REALTIME_VIDEO_ENABLED[\s\S]*playbackActive[\s\S]*currentMediaIsVideo[\s\S]*videoProcessingEnabled/);
  assert.match(source, /const audioPeriodActive = playbackActive && audioProcessingEnabled/);
  assert.match(source, /if \(!runtimeActive \|\| !runtimeBaseParameters\)/);
  assert.match(source, /getProcessingStatusLabel\(audioProcessingStatus\)/);
});

test('主界面不渲染运行资源区域，但保留自动准备和状态轮询', async () => {
  const source = await readSource('App.tsx');

  assert.doesNotMatch(source, /<Card title="运行资源">/);
  assert.doesNotMatch(source, /选择本地资源目录|清理运行资源/);
  assert.doesNotMatch(source, /cancel_runtime_resource_install|import_runtime_resource_directory|clear_runtime_resources/);
  assert.match(source, /install_runtime_resources/);
  assert.match(source, /get_runtime_resource_status/);
  assert.match(source, /resolvePendingRuntimeAction/);
  assert.match(source, /runtimeResourceEnsureDecision/);
  assert.match(source, /window\.clearTimeout\(timer\)/);
  assert.match(source, /runtimeResourcePollComponent\(runtimeResourceStatus\)/);
  const pollingEffectStart = source.indexOf('if (!runtimeResourceStatus || !shouldPollRuntimeResources(runtimeResourceStatus)) return;');
  const pollingEffect = source.slice(pollingEffectStart, source.indexOf('useLayoutEffect', pollingEffectStart));
  assert.doesNotMatch(pollingEffect, /state: 'failed'/);
  assert.match(pollingEffect, /setRuntimeResourceStatus\(\(current\) => current \? \{ \.\.\.current \} : current\)/);
  assert.match(source, /refreshRuntimeResourceCapabilities/);
  assert.match(source, /runtimeResourceMountedRef\.current[\s\S]*runtimeResourceActionTokenRef\.current === expectedActionToken/);

  const initialRuntimeStatusStart = source.indexOf('runtimeResourceMountedRef.current = true;');
  const initialRuntimeStatus = source.slice(
    initialRuntimeStatusStart,
    source.indexOf('useEffect(() => {', initialRuntimeStatusStart + 1),
  );
  assert.match(initialRuntimeStatus, /applyRuntimeResourceStatus\(status, actionToken\)/);
  assert.doesNotMatch(initialRuntimeStatus, /setRuntimeResourceStatus\(status\)/);

  const startupCapabilities = source.slice(
    source.indexOf('const cancelIdleWork = scheduleAfterInitialPaint'),
    source.indexOf('return () => {', source.indexOf('const cancelIdleWork = scheduleAfterInitialPaint')),
  );
  assert.match(startupCapabilities, /const capabilityToken = runtimeResourceActionTokenRef\.current/);
  assert.match(startupCapabilities, /refreshRuntimeResourceCapabilities\(\['media'\], capabilityToken\)/);
  assert.doesNotMatch(startupCapabilities, /refreshRuntimeResourceCapabilities\(\['media', 'voice'\], capabilityToken\)/);
});

test('启动后的空闲初始化只检查 media 运行资源', async () => {
  const source = await readSource('App.tsx');
  const startupCapabilitiesStart = source.indexOf('const cancelIdleWork = scheduleAfterInitialPaint');
  const startupCapabilities = source.slice(
    startupCapabilitiesStart,
    source.indexOf('return () => {', startupCapabilitiesStart),
  );

  assert.match(startupCapabilities, /refreshRuntimeResourceCapabilities\(\['media'\], capabilityToken\)/);
  assert.doesNotMatch(startupCapabilities, /ensureRuntimeResources\('voice'|voiceClone|voice_clone|XTTS|Demucs|Whisper/);
});

test('已有视频的视频处理先确保 media 并只进入 mpv', async () => {
  const source = await readSource('App.tsx');
  const mediaProcessing = source.slice(
    source.indexOf('async function applyVideoProcessing'),
    source.indexOf('async function cleanupLocalCaches'),
  );

  assert.match(mediaProcessing, /ensureRuntimeResources\('media',[\s\S]*prepare_realtime_video_plan/);
  assert.match(source, /commit_realtime_video_plan/);
  assert.match(source, /stop_realtime_video_renderer/);
  assert.doesNotMatch(source, /prepare_media_video_stream|read_media_video_stream|ack_media_video_stream|commit_media_video_stream|VideoMse|video_stream/);
  assert.doesNotMatch(mediaProcessing, /start_media_processing/);
  assert.doesNotMatch(mediaProcessing, /audio_variants|ambient_sound_path|['"]both['"]/);
});
