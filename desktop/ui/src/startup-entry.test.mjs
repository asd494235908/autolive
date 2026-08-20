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

  assert.match(source, /id="startup-splash"/);
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
  const source = await readFile(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8');
  const config = JSON.parse(source);
  const mainWindow = config.app.windows.find((window) => window.label === 'main');

  assert.ok(mainWindow);
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
  assert.doesNotMatch(source, /from ['"]antd['"]/);
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
  const app = await readSource('App.tsx');
  const nonce = await readSource('cspNonce.ts');

  assert.match(app, /<ConfigProvider\s+csp=\{\{\s*nonce:\s*getCspNonce\(\)\s*\}\}/);
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

test('Ant Design 根外壳由延迟加载的 App 自己拥有', async () => {
  const source = await readSource('App.tsx');

  assert.match(source, /App as AntApp/);
  assert.match(source, /ConfigProvider/);
  assert.match(source, /<ConfigProvider(?:\s[^>]*)?>/);
  assert.match(source, /<AntApp>/);
});

test('导入视频使用单一同步 guard 覆盖选择和探测，不打开窗口不自动播放', async () => {
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
    /try \{[\s\S]*await open\([\s\S]*probe_local_video[\s\S]*get_snapshot/,
  );
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.match(
    importVideo,
    /catch \(cause\) \{[\s\S]*导入视频失败[\s\S]*\} finally \{[\s\S]*importVideoInFlightRef\.current = false;[\s\S]*setImportVideoBusy\(false\);[\s\S]*\}/,
  );
  assert.doesNotMatch(importVideo, /setTimeout|waitForAbortableDelay/);
});

test('导入按钮在完整导入链路中显示 loading 并禁止重复点击', async () => {
  const source = await readSource('App.tsx');
  const onClick = source.indexOf('onClick={() => void importVideo()}');
  assert.notEqual(onClick, -1);
  const buttonStart = source.lastIndexOf('<Button', onClick);
  const importButton = source.slice(buttonStart, source.indexOf('</Button>', onClick));

  assert.match(importButton, /loading=\{importVideoBusy\}/);
  assert.match(importButton, /disabled=\{importVideoBusy \|\| runtimeResourceBusy\}/);
  assert.match(importButton, /导入视频/);
  assert.doesNotMatch(importButton, /导入视频并播放/);
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

test('启动不恢复上次导入的视频，必须由用户手动选择文件', async () => {
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
  assert.match(importVideo, /probe_local_video[\s\S]*get_snapshot/);
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

test('导入视频先确保 media，资源就绪后再探测，不自动播放', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(importVideo, /ensureRuntimeResources\('media',[\s\S]*probe_local_video[\s\S]*get_snapshot/);
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.match(source, /pendingRuntimeActionRef/);
  assert.match(source, /token/);
  assert.match(source, /RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}position:\s*['"]fixed['"]/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}overflow:\s*['"]hidden['"]/);
});

test('导入文件选择器只开放确认的视频扩展名', async () => {
  const source = await readSource('App.tsx');
  const declaration = source.match(/const SUPPORTED_VIDEO_EXTENSIONS = \[([\s\S]*?)\] as const;/);

  assert.ok(declaration);
  assert.deepEqual(
    [...declaration[1].matchAll(/'([^']+)'/g)].map((match) => match[1]),
    ['mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'ts', 'm2ts', 'flv', 'wmv', '3gp'],
  );
  assert.match(source, /filters: \[\{ name: '视频文件', extensions: \[\.\.\.SUPPORTED_VIDEO_EXTENSIONS\] \}\]/);
});

test('高级声音抽屉展示 FFmpeg 参数且主窗口不暴露实时话术', async () => {
  const source = await readSource('App.tsx');
  const capability = await readSource('audio-processing-capabilities.ts');
  for (const field of ['input_gain_db', 'output_gain_db', 'loudness_adjustment_db', 'low_eq_db', 'mid_eq_db', 'high_eq_db', 'noise_reduction_percent', 'phase_perturbation_percent', 'vibrato_frequency_hz', 'environment_noise_percent', 'sample_rate_hz', 'output_bitrate_kbps']) {
    assert.match(capability, new RegExp(field), field);
  }
  assert.match(source, /title="高级声音设置"/);
  assert.match(source, /audioCapabilityRows/);
  assert.match(source, /row\.key !== 'spectral_perturbation_percent'/);
  assert.match(source, /应用声音参数/);
  assert.doesNotMatch(source.slice(source.indexOf('function DesktopApp()')), /实时话术幻化/);
  assert.match(source, /ariaLabel="声音周期最小秒"/);
  assert.match(source, /ariaLabel="声音周期最大秒"/);
  assert.match(source, /ariaLabel="视频周期最小秒"/);
  assert.match(source, /ariaLabel="视频周期最大秒"/);
  assert.doesNotMatch(source, /title="声音处理参数"/);
  assert.doesNotMatch(source, /aria-label="音频 MFCC 维度"/);
  assert.doesNotMatch(source, /aria-label="音频音色库"/);
});

test('停止或暂停播放时停止实时参数调度', async () => {
  const source = await readSource('App.tsx');

  assert.match(source, /const playbackActive = snapshot\?\.playback_state\?\.toLowerCase\(\) === ['"]playing['"]/);
  assert.match(source, /const runtimeActive = playbackActive && videoProcessingEnabled/);
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

test('已有视频的媒体处理先确保 media 并只恢复一次', async () => {
  const source = await readSource('App.tsx');
  const mediaProcessing = source.slice(
    source.indexOf('async function applyMediaProcessing'),
    source.indexOf('async function startResearchAnalysis'),
  );

  assert.match(mediaProcessing, /ensureRuntimeResources\('media',[\s\S]*start_media_processing/);
});
