import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const helperPath = path.join(currentDir, '插话播放器.ts');
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
    duckingAttackMs: { min: 5, max: 1_000 },
    duckingReleaseMs: { min: 10, max: 3_000 },
  });
});

test('接入 Web Audio 的本地媒体会在 src 前启用匿名 CORS，避免跨源音轨静音', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const finalEffectMedia = appSource.slice(
    appSource.indexOf('<video', appSource.indexOf('function FinalEffectWindow')),
    appSource.indexOf('</>', appSource.indexOf('<video', appSource.indexOf('function FinalEffectWindow'))),
  );

  assert.match(appSource, /audioContextCleanupTimerRef/);
  assert.match(
    finalEffectMedia,
    /<video\s+ref=\{videoRef\}\s+crossOrigin="anonymous"\s+src=\{sourceUrl\}/,
  );
  assert.match(
    finalEffectMedia,
    /<audio\s+ref=\{audioRef\}\s+crossOrigin="anonymous"\s+src=\{audioUrl \?\? undefined\}/,
  );
  assert.match(
    finalEffectMedia,
    /<audio\s+ref=\{interludeAudioRef\}\s+crossOrigin="anonymous"\s+src=\{interludeAudioUrl \?\? undefined\}/,
  );
  assert.match(appSource, /if \(audioContextRef\.current\) return scheduleAudioContextCleanup;/);
  assert.match(appSource, /window\.setTimeout\(\(\) => \{/);
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
  assert.match(appSource, /audioSource\.connect\(analyser\)/);
  assert.doesNotMatch(appSource, /createBiquadFilter\(\)/);
  assert.doesNotMatch(appSource, /createDelay\(\)/);
  assert.match(appSource, /<audio[\s\S]*?preload="auto"[\s\S]*?onPlaying=\{handleRealtimeAudioPlaying\}/);
});

test('插话音频预加载，正常播放到结束后才切换，并显示加载失败原因', async () => {
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');

  assert.match(appSource, /preload="auto"/);
  assert.match(appSource, /if \(interludeActiveRef\.current\) return;/);
  assert.match(appSource, /onEnded=\{handleInterludeEnded\}/);
  assert.match(appSource, /onError=\{handleInterludeError\}/);
  assert.match(appSource, /插话音频播放失败/);
});

test('视频进入下一轮时不重置插话，插话调度使用独立时钟', async () => {
  const { buildInterludeScheduleKey } = await loadInterludeModule();
  const appSource = await readFile(path.join(currentDir, 'App.tsx'), 'utf8');
  const restartSource = appSource.slice(
    appSource.indexOf('function restartToNextLoop'),
    appSource.indexOf('function restartAtBoundary'),
  );

  assert.equal(buildInterludeScheduleKey(7, '/tmp/video.mp4'), '7:/tmp/video.mp4');
  assert.match(appSource, /buildInterludeScheduleKey\(currentSnapshot\.playback_generation, sourceKey\)/);
  assert.match(appSource, /const currentClockMs = performance\.now\(\);/);
  assert.doesNotMatch(restartSource, /clearInterludePlayback/);
});
