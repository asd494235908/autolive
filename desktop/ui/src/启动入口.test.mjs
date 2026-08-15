import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

async function readSource(fileName) {
  return readFile(new URL(`./${fileName}`, import.meta.url), 'utf8');
}

test('HTML 入口在 React 执行前提供静态启动反馈', async () => {
  const source = await readSource('../index.html');

  assert.match(source, /id="startup-splash"/);
  assert.match(source, /正在启动桌面端/);
  assert.match(source, /startup-splash-spinner/);
  assert.match(source, /animation/);
});

test('启动壳使用 Ant Design 默认浅色主题色', async () => {
  const html = await readSource('../index.html');
  const loading = await readSource('启动加载.tsx');

  assert.match(html, /color-scheme:\s*light/);
  assert.match(html, /background:\s*#fff/);
  assert.match(html, /#1677ff/);
  assert.match(html, /rgba\(0, 0, 0, 0\.88\)/);
  assert.match(loading, /background: '#fff'/);
  assert.match(loading, /color: 'rgba\(0, 0, 0, 0\.88\)'/);
  assert.match(loading, /borderTopColor: '#1677ff'/);
});

test('主窗口允许页面纵向滚动，滚动锁仅由最终效果窗口使用', async () => {
  const html = await readSource('../index.html');

  assert.match(html, /body\s*\{[\s\S]*overflow-y:\s*auto;/);
  assert.doesNotMatch(html, /body\s*\{[\s\S]*overflow:\s*hidden;/);
});

test('React 入口懒加载 App 并提供 Suspense 与错误恢复', async () => {
  const source = await readSource('main.tsx');

  assert.match(source, /lazy\(\(\) => import\(['"]\.\/App['"]\)\)/);
  assert.match(source, /<Suspense\b/);
  assert.match(source, /<StartupErrorBoundary\b/);
  assert.doesNotMatch(source, /from ['"]antd['"]/);
  assert.doesNotMatch(source, /import App from ['"]\.\/App['"]/);
});

test('Ant Design 根外壳由延迟加载的 App 自己拥有', async () => {
  const source = await readSource('App.tsx');

  assert.match(source, /App as AntApp/);
  assert.match(source, /ConfigProvider/);
  assert.match(source, /<ConfigProvider>/);
  assert.match(source, /<AntApp>/);
});

test('导入视频使用单一同步 guard 覆盖选择、探测、播放和打开窗口', async () => {
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
    /try \{[\s\S]*await open\([\s\S]*probe_local_mp4[\s\S]*start_playback[\s\S]*await openFinalEffectWindowFromHome\(\);/,
  );
  assert.match(
    importVideo,
    /catch \(cause\) \{[\s\S]*导入视频失败[\s\S]*\} finally \{[\s\S]*importVideoInFlightRef\.current = false;[\s\S]*setImportVideoBusy\(false\);[\s\S]*\}/,
  );
  assert.doesNotMatch(importVideo, /setTimeout|waitForAbortableDelay/);
});

test('导入按钮在完整导入链路中显示 loading 并禁止重复点击', async () => {
  const source = await readSource('App.tsx');
  const buttonLabel = source.indexOf('导入视频并播放');
  const buttonStart = source.lastIndexOf('<Button', buttonLabel);
  const importButton = source.slice(buttonStart, source.indexOf('</Button>', buttonStart));

  assert.match(importButton, /loading=\{importVideoBusy\}/);
  assert.match(importButton, /disabled=\{importVideoBusy \|\| runtimeResourceBusy\}/);
  assert.match(importButton, /onClick=\{\(\) => void importVideo\(\)\}/);
});

test('后台重启后自动恢复上次导入的视频并继续准备人声', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(source, /const sourceRestoreAttemptedRef = useRef\(false\);/);
  assert.match(source, /window\.localStorage\.getItem\('autolive\.source\.path'\)/);
  assert.match(
    source,
    /if \(!nextSnapshot\.source_media && !sourceRestoreAttemptedRef\.current\) \{[\s\S]*void importVideo\(storedSourcePath\)/,
  );
  assert.match(importVideo, /async function importVideo\(selectedSourcePath\?: string\)/);
  assert.match(importVideo, /selectedSourcePath \?\?[\s\S]*await open\(/);
  assert.match(importVideo, /probe_local_mp4[\s\S]*start_playback[\s\S]*openFinalEffectWindowFromHome[\s\S]*prepareVoiceCloneAfterImport/);
});

test('打开文件选择器或恢复旧路径前先取消旧视频的自动人声准备', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );
  const abortIndex = importVideo.indexOf('voiceCloneAutoPrepareControllerRef.current?.abort()');
  const selectedIndex = importVideo.indexOf('const selection =');
  const chooserIndex = importVideo.indexOf('await open(');

  assert.ok(abortIndex > -1);
  assert.ok(abortIndex < selectedIndex);
  assert.ok(abortIndex < chooserIndex);
  assert.match(importVideo, /const restoreAutoPrepareGeneration = selectedSourcePath === undefined/);
  assert.match(importVideo, /finally \{[\s\S]*shouldRestoreVoiceCloneAutoPrepareAfterPicker\([\s\S]*prepareVoiceCloneAfterImport\(restoreAutoPrepareGeneration\)/);
});

test('导入视频先确保 media，资源就绪后再探测并播放一次', async () => {
  const source = await readSource('App.tsx');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(importVideo, /ensureRuntimeResources\('media',[\s\S]*probe_local_mp4[\s\S]*start_playback/);
  assert.match(source, /pendingRuntimeActionRef/);
  assert.match(source, /token/);
  assert.match(source, /RUNTIME_RESOURCE_POLL_INTERVAL_MS = 500/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}position:\s*['"]fixed['"]/);
  assert.doesNotMatch(source, /runtime-resource[\s\S]{0,400}overflow:\s*['"]hidden['"]/);
});

test('资源面板提供重试、取消、本地导入和显式清理', async () => {
  const source = await readSource('App.tsx');

  assert.match(source, /install_runtime_resources/);
  assert.match(source, /cancel_runtime_resource_install/);
  assert.match(source, /import_runtime_resource_directory/);
  assert.match(source, /clear_runtime_resources/);
  assert.match(source, /Modal\.confirm/);
  assert.match(source, /选择本地资源目录/);
  assert.match(source, /清理运行资源/);
  assert.match(source, /runtimeResourceProgressDetails\(runtimeResourceStatus\)/);
  assert.match(source, /resolvePendingRuntimeAction/);
  assert.match(source, /runtimeResourceEnsureDecision/);
  assert.match(source, /if \(runtimeResourceClearInFlightRef\.current\) \{[\s\S]*RuntimeResourceConflictError/);
  assert.match(source, /window\.clearTimeout\(timer\)/);
  assert.match(source, /runtimeResourcePollError/);
  assert.match(source, /runtimeResourcePollComponent\(runtimeResourceStatus\)/);
  assert.match(source, /resolveRuntimeResourceClearLifecycle/);
  assert.match(source, /clearLifecycle\.terminalAction === 'revalidate-capabilities'/);
  assert.match(source, /resourceConsumersBusy/);
  assert.match(source, /resourceConsumersBusyReasonRef\.current/);
  assert.match(source, /title=\{runtimeResourceClearDisabledReason/);
  const pollingEffectStart = source.indexOf('if (!runtimeResourceStatus || !shouldPollRuntimeResources(runtimeResourceStatus)) return;');
  const pollingEffect = source.slice(pollingEffectStart, source.indexOf('useLayoutEffect', pollingEffectStart));
  assert.doesNotMatch(pollingEffect, /state: 'failed'/);
  assert.match(pollingEffect, /setRuntimeResourceStatus\(\(current\) => current \? \{ \.\.\.current \} : current\)/);

  const cancelResources = source.slice(
    source.indexOf('async function cancelRuntimeResources'),
    source.indexOf('async function chooseRuntimeResourceDirectory'),
  );
  assert.match(cancelResources, /catch \(cause\)[\s\S]*setRuntimeResourceStatus\(\(current\) => current \? \{ \.\.\.current \} : current\)/);
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
  assert.match(startupCapabilities, /refreshRuntimeResourceCapabilities\(\['media', 'voice'\], capabilityToken\)/);

  const chooseDirectory = source.slice(
    source.indexOf('async function chooseRuntimeResourceDirectory'),
    source.indexOf('function confirmClearRuntimeResources'),
  );
  assert.match(chooseDirectory, /try \{[\s\S]*await open\(\{ directory: true, multiple: false \}\)[\s\S]*catch \(cause\)/);
  assert.match(chooseDirectory, /if \(!pending\) runtimeResourceActionTokenRef\.current \+= 1;[\s\S]*applyRuntimeResourceStatus\(status, token\)/);

  const retryResources = source.slice(
    source.indexOf('async function retryRuntimeResources'),
    source.indexOf('async function cancelRuntimeResources'),
  );
  assert.match(retryResources, /if \(!pending\) runtimeResourceActionTokenRef\.current \+= 1;[\s\S]*applyRuntimeResourceStatus\(status, token\)/);

  const clearResources = source.slice(
    source.indexOf('function confirmClearRuntimeResources'),
    source.indexOf('function updateInterludeDraft'),
  );
  assert.match(clearResources, /resourceConsumersBusyReasonRef\.current/);
  assert.match(clearResources, /runtimeResourceBusyRef\.current/);
  assert.match(clearResources, /voiceClonePreGenerationInFlightRef\.current/);
  assert.match(clearResources, /applyRuntimeResourceStatus/);
  assert.match(clearResources, /runtimeResourceBusyRef\.current = true;[\s\S]*state: 'checking',[\s\S]*component: null,[\s\S]*clear_runtime_resources/);
  assert.match(clearResources, /catch \(cause\) \{[\s\S]*refreshRuntimeResourceCapabilities\(\['media', 'voice'\], token\)/);
  assert.doesNotMatch(clearResources, /setMediaEngineCapabilities\(null\)[\s\S]*setVoiceCloneWorkerCapabilities\(null\)/);
  assert.match(source, /terminalAction === 'conflict'[\s\S]*refreshRuntimeResourceCapabilities\(\['media', 'voice'\], capabilityToken\)/);

  const consumerBusyState = source.slice(
    source.indexOf('const resourceConsumersBusyReason ='),
    source.indexOf('const voiceClonePositionMs'),
  );
  assert.match(consumerBusyState, /snapshot\?\.video_processing_status === 'processing'/);
  assert.match(consumerBusyState, /snapshot\?\.audio_processing_status === 'processing'/);
  assert.match(consumerBusyState, /realtimeWorkerRunning:\s*isRuntimeResourceRealtimeConsumerBusy\(snapshot\?\.worker_status\)/);
  assert.match(source, /const realtimeAudioBusy =[\s\S]*current_audio_source === 'realtime_variant'/);
});

test('已有视频的媒体处理先确保 media 并只恢复一次', async () => {
  const source = await readSource('App.tsx');
  const mediaProcessing = source.slice(
    source.indexOf('async function applyMediaProcessing'),
    source.indexOf('async function startResearchAnalysis'),
  );

  assert.match(mediaProcessing, /ensureRuntimeResources\('media',[\s\S]*start_media_processing/);
});
