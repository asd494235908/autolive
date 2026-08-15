import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { test } from 'node:test';

const currentDir = path.dirname(new URL(import.meta.url).pathname);

test('固定话术面板标题描述实际播放用途', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /<Card title="固定话术播放">/);
  assert.doesNotMatch(appSource, /<Card title="声音克隆替换">/);
  assert.match(appSource, /正在准备当前 MP4 的参考人声（自动去除背景音乐）与话术索引/);
});

test('固定话术面板保留添加文案入口并区分更新状态', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /selectedVoiceClonePreset \? '更新文案' : '添加文案'/);
});

test('固定话术音频播放失败时不上吞错误，并通过失败 IPC 恢复原音轨', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const errorHandler = appSource.slice(
    appSource.indexOf('function handleVoiceCloneAudioError'),
    appSource.indexOf('function handleVoiceCloneAudioElementError'),
  );

  assert.match(appSource, /onError=\{handleVoiceCloneAudioElementError\}/);
  assert.match(appSource, /fail_voice_clone_playback/);
  assert.match(appSource, /operation_id: currentPlayback\.operation_id/);
  assert.match(appSource, /reason:/);
  assert.match(appSource, /setVoiceCloneAudioUrl\(null\)/);
  assert.match(appSource, /恢复原音轨/);
  assert.doesNotMatch(errorHandler, /clear_voice_clone_replacement/);
  assert.match(errorHandler, /原音轨恢复失败/);
  assert.match(errorHandler, /回到主窗口[\s\S]*清空当前替换/);
  assert.doesNotMatch(
    appSource,
    /function playCurrentVoiceCloneText[\s\S]*?voiceCloneAudio\.play\(\)\.catch\(\(\) => undefined\)/,
  );
});

test('点击播放当前文案时在用户手势同步通知播放器恢复 AudioContext', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const playHandler = appSource.slice(
    appSource.indexOf('async function playCurrentVoiceCloneText'),
    appSource.indexOf('async function cancelVoiceCloneOperation'),
  );

  assert.match(playHandler, /type: 'playback-control'/);
  assert.match(playHandler, /action: 'resume'/);
  assert.match(appSource, /if \(event\.data\.action === 'resume'\) resumeAudioDiagnostics\(\);/);
});

test('固定话术使用独立媒体输出并在真正播放后才静音原音轨', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const audioGraphSetup = appSource.slice(
    appSource.indexOf('context = new AudioContext()'),
    appSource.indexOf('return scheduleAudioContextCleanup;'),
  );
  const voiceClonePlaybackEffectStart = appSource.indexOf(
    'voiceCloneAudioUrlRef.current = voiceCloneAudioUrl',
  );
  const voiceClonePlaybackEffect = appSource.slice(
    voiceClonePlaybackEffectStart,
    appSource.indexOf('const interludeAudio = interludeAudioRef.current', voiceClonePlaybackEffectStart),
  );

  assert.doesNotMatch(audioGraphSetup, /createMediaElementSource\(voiceCloneAudioRef\.current\)/);
  assert.match(appSource, /onPlaying=\{handleVoiceCloneAudioPlaying\}/);
  assert.match(appSource, /currentVoiceCloneAudioPlaying/);
  assert.match(voiceClonePlaybackEffect, /if \(isCurrentVoiceClonePlaybackActive\(snapshotRef\.current\)\) \{[\s\S]*playVoiceCloneAudio\(voiceCloneAudio\)/);
  assert.doesNotMatch(voiceClonePlaybackEffect, /if \(!audioDiagnosticsReady\)/);
});

test('导入后仅在最终效果窗口打开成功后延迟四秒自动准备人声且不等待 MP4 SHA-256', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const openWindowFunction = appSource.slice(
    appSource.indexOf('async function openFinalEffectWindowFromHome'),
    appSource.indexOf('function postPlaybackMediaControl'),
  );
  const importFunction = appSource.slice(
    appSource.indexOf('async function importVideo'),
    appSource.indexOf('function updateInterludeDraft'),
  );
  const prepareFunction = appSource.slice(
    appSource.indexOf('async function prepareVoiceCloneAfterImport'),
    appSource.indexOf('async function importVideo'),
  );

  assert.match(appSource, /const VOICE_CLONE_AUTO_PREPARE_DELAY_MS = 4_000;/);
  assert.match(appSource, /import \{ scheduleAfterInitialPaint, waitForAbortableDelay \} from '\.\/启动调度';/);
  assert.match(prepareFunction, /await waitForAbortableDelay\(VOICE_CLONE_AUTO_PREPARE_DELAY_MS, controller\.signal\)/);
  assert.doesNotMatch(prepareFunction, /mp4_sha256|mp4_hash_status|SHA-256/);
  assert.doesNotMatch(prepareFunction, /setVoiceCloneAutoPreparePhase\('hash'\)/);
  assert.doesNotMatch(prepareFunction, /get_voice_clone_worker_capabilities/);
  assert.doesNotMatch(appSource, /function waitForVoiceCloneAutoPrepareDelay/);
  assert.match(openWindowFunction, /Promise<boolean>/);
  assert.match(openWindowFunction, /return true;/);
  assert.match(openWindowFunction, /return false;/);
  assert.match(importFunction, /const finalEffectWindowOpened = await openFinalEffectWindowFromHome\(\);/);
  assert.match(
    importFunction,
    /if \(finalEffectWindowOpened\) \{[\s\S]*void prepareVoiceCloneAfterImport\(startedSnapshot\.playback_generation\);[\s\S]*\}/,
  );
  assert.doesNotMatch(importFunction, /await openFinalEffectWindowFromHome\(\);\s*void prepareVoiceCloneAfterImport/);
  assert.match(appSource, /voiceCloneAutoPreparePhase !== null/);
  assert.match(
    appSource,
    /async function cancelVoiceCloneOperation[\s\S]*voiceCloneAutoPrepareControllerRef\.current\?\.abort\(\)/,
  );
  assert.match(appSource, /最终效果窗口已打开，4 秒后开始自动准备人声/);
  assert.doesNotMatch(appSource, /正在等待 MP4 哈希完成/);
});

test('自动与手动准备人声都先确保 voice 资源', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const prepareFunction = appSource.slice(
    appSource.indexOf('async function prepareVoiceCloneSource'),
    appSource.indexOf('async function playCurrentVoiceCloneText'),
  );
  const autoPrepareFunction = appSource.slice(
    appSource.indexOf('async function prepareVoiceCloneAfterImport'),
    appSource.indexOf('async function importVideo'),
  );

  assert.match(prepareFunction, /ensureRuntimeResources\('voice'/);
  assert.match(autoPrepareFunction, /prepareVoiceCloneSource\(\{ automatic: true \}\)/);
  assert.match(appSource, /voice 包含媒体、固定话术运行环境和模型/);
  assert.match(prepareFunction, /try \{[\s\S]*ensureRuntimeResources\('voice'[\s\S]*catch \(cause\)/);
  assert.match(appSource, /runtimeResourceStatus\?\.component === 'voice'[\s\S]*runtimeResourceStatus\.state === 'ready'/);
});

test('固定话术预生成只批量准备文案，不影响实际播放或原音轨', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const preGenerationEffect = appSource.slice(
    appSource.indexOf("start_voice_clone_pre_generation"),
    appSource.indexOf("start_voice_clone_pre_generation") + 1_500,
  );

  assert.match(appSource, /start_voice_clone_pre_generation/);
  assert.match(
    appSource,
    /voiceClonePresets\.map\(\(\{ id, text \}\) => \(\{ preset_id: id, text \}\)\)/,
  );
  assert.doesNotMatch(preGenerationEffect, /start_voice_clone_playback/);
  assert.doesNotMatch(preGenerationEffect, /action:\s*'resume'/);
  assert.doesNotMatch(preGenerationEffect, /setVoiceCloneAudioUrl/);
});

test('固定话术预生成在 StrictMode 重挂载后仍接收批次快照', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(
    appSource,
    /useEffect\(\(\) => \{\s*voiceClonePreGenerationMountedRef\.current = true;[\s\S]*?return \(\) => \{\s*voiceClonePreGenerationMountedRef\.current = false;/,
  );
});

test('旧预生成请求结束后重新评估最新来源且拒绝旧快照', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const effectStart = appSource.indexOf("invoke<PlaybackSnapshot>('start_voice_clone_pre_generation'");
  const effectEnd = appSource.indexOf('function applyVoiceClonePresetSelection', effectStart);
  const preGenerationEffect = appSource.slice(effectStart, effectEnd);

  assert.match(preGenerationEffect, /shouldAcceptVoiceClonePreGenerationResult\(\{/);
  assert.match(
    preGenerationEffect,
    /currentGeneration:\s*voiceClonePreGenerationCurrentGenerationRef\.current/,
  );
  assert.match(
    appSource,
    /useLayoutEffect\(\(\) => \{\s*voiceClonePreGenerationCurrentGenerationRef\.current = snapshot\?\.playback_generation \?\? null;\s*\}, \[snapshot\?\.playback_generation\]\);/,
  );
  assert.match(
    preGenerationEffect,
    /\.finally\(\(\) => \{[\s\S]*?voiceClonePreGenerationInFlightRef\.current = false;[\s\S]*?setVoiceClonePreGenerationCompletionVersion\(\(value\) => value \+ 1\)/,
  );
  assert.match(preGenerationEffect, /voiceClonePreGenerationCompletionVersion,[\s\S]*?\]\);/);
});

test('固定话术预生成在提交 IPC 前同步占用 trigger key', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const effectStart = appSource.indexOf('const key = getVoiceClonePreGenerationTriggerKey');
  const effectEnd = appSource.indexOf('function applyVoiceClonePresetSelection', effectStart);
  const preGenerationEffect = appSource.slice(effectStart, effectEnd);
  const startedKeyWrite = 'voiceClonePreGenerationStartedKeyRef.current = key;';
  const startedKeyIndex = preGenerationEffect.indexOf(startedKeyWrite);
  const invokeIndex = preGenerationEffect.indexOf("invoke<PlaybackSnapshot>('start_voice_clone_pre_generation'");
  const thenBody = preGenerationEffect.slice(
    preGenerationEffect.indexOf('.then((nextSnapshot)'),
    preGenerationEffect.indexOf('.catch((cause)'),
  );

  assert.notEqual(startedKeyIndex, -1);
  assert.ok(startedKeyIndex < invokeIndex);
  assert.doesNotMatch(thenBody, /voiceClonePreGenerationStartedKeyRef\.current = key/);
});

test('预生成 IPC 失败仅对当前 trigger key 自动重试一次', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const effectStart = appSource.indexOf('const key = getVoiceClonePreGenerationTriggerKey');
  const effectEnd = appSource.indexOf('function applyVoiceClonePresetSelection', effectStart);
  const preGenerationEffect = appSource.slice(effectStart, effectEnd);
  const thenBody = preGenerationEffect.slice(
    preGenerationEffect.indexOf('.then((nextSnapshot)'),
    preGenerationEffect.indexOf('.catch((cause)'),
  );

  assert.match(appSource, /const voiceClonePreGenerationRetriedKeyRef = useRef<string \| null>\(null\)/);
  assert.match(appSource, /const voiceClonePreGenerationCurrentKeyRef = useRef<string \| null>\(null\)/);
  assert.match(
    preGenerationEffect,
    /shouldRetryVoiceClonePreGeneration\(\{[\s\S]*?failedKey:\s*key,[\s\S]*?currentKey:\s*voiceClonePreGenerationCurrentKeyRef\.current,[\s\S]*?lastRetriedKey:\s*voiceClonePreGenerationRetriedKeyRef\.current/,
  );
  assert.match(
    preGenerationEffect,
    /voiceClonePreGenerationRetriedKeyRef\.current = key;[\s\S]*?voiceClonePreGenerationStartedKeyRef\.current = null;[\s\S]*?retryAfterFailure = true/,
  );
  assert.match(
    preGenerationEffect,
    /setVoiceCloneFormError\([\s\S]*?if \(!shouldRetryVoiceClonePreGeneration\(/,
  );
  assert.match(thenBody, /setVoiceCloneFormError\(null\)/);
  assert.match(
    preGenerationEffect,
    /\(cancelled \|\| retryAfterFailure\) && voiceClonePreGenerationMountedRef\.current/,
  );
  assert.doesNotMatch(thenBody, /voiceClonePreGenerationStartedKeyRef\.current = null/);
});

test('批量预生成占用 Worker 时允许取消并禁用准备和播放', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const disabledReasons = appSource.slice(
    appSource.indexOf('const voiceClonePreGenerationBusyReason'),
    appSource.indexOf('const voiceCloneSaveDisabledReason'),
  );
  const canCancel = appSource.slice(
    appSource.indexOf('const voiceCloneCanCancel'),
    appSource.indexOf('const voiceCloneCanClear'),
  );

  assert.match(disabledReasons, /getVoiceClonePreGenerationBusyReason\(preGeneration\.status\)/);
  assert.match(canCancel, /preGeneration\.status === 'generating'/);
});
