import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./audio-output-recovery.ts', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const {
  classifyAudioOutputSync,
  clearResolvedPortAudioSyncError,
  getPortAudioSourceRetryMode,
  isExpectedAudioOutputSyncCancellation,
  isRetryableAudioOutputSyncCode,
  resolvePortAudioSourcePath,
  shouldKeepPortAudioCycleScheduling,
} = await import(
  `data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`
);

test('successful source sync clears only resolved PortAudio fallback errors', () => {
  assert.equal(
    clearResolvedPortAudioSyncError(
      'PortAudio 音频源不可用，已回退 WebView：候选追赶期间收到更新的停止、暂停或切换请求',
    ),
    null,
  );
  assert.equal(
    clearResolvedPortAudioSyncError(
      'PortAudio PCM 生产恢复失败，已回退 WebView：候选预热超时',
    ),
    null,
  );
  assert.equal(
    clearResolvedPortAudioSyncError('独立播放器加载视频失败，请回到主页重新导入。'),
    '独立播放器加载视频失败，请回到主页重新导入。',
  );
  assert.equal(clearResolvedPortAudioSyncError(null), null);
});

test('only the latest active sync path clears a resolved PortAudio error', () => {
  const activeStart = appSource.indexOf("if (disposition === 'active')");
  const retryableStart = appSource.indexOf("if (disposition === 'retryable')", activeStart);
  const activeBranch = appSource.slice(activeStart, retryableStart);
  assert.ok(activeStart >= 0 && retryableStart > activeStart);
  assert.match(activeBranch, /setPlaybackError\(clearResolvedPortAudioSyncError\)/);

  const cancellationStart = appSource.indexOf('if (isExpectedAudioOutputSyncCancellation');
  const retryableCatchStart = appSource.indexOf('} else if (isRetryableAudioOutputSyncCode', cancellationStart);
  const cancellationBranch = appSource.slice(cancellationStart, retryableCatchStart);
  assert.ok(cancellationStart >= 0 && retryableCatchStart > cancellationStart);
  assert.doesNotMatch(cancellationBranch, /setPlaybackError/);
});

const stalledStatus = {
  available: true,
  preferred_portaudio: true,
  running: false,
  recovery_required: true,
  hardware_state: 'active',
};

test('playing output requests exactly one recovery for a stalled PCM producer', () => {
  assert.equal(getPortAudioSourceRetryMode(stalledStatus, 'playing', false), 'recover');
  assert.equal(getPortAudioSourceRetryMode(stalledStatus, 'playing', true), null);
});

test('paused or healthy output never starts PCM recovery', () => {
  assert.equal(getPortAudioSourceRetryMode(stalledStatus, 'paused', false), null);
  assert.equal(
    getPortAudioSourceRetryMode(
      { ...stalledStatus, running: true, recovery_required: false },
      'playing',
      false,
    ),
    null,
  );
});

test('WebView-only preference never starts PCM recovery', () => {
  assert.equal(
    getPortAudioSourceRetryMode(
      { ...stalledStatus, preferred_portaudio: false },
      'playing',
      false,
    ),
    null,
  );
});

test('selected active hardware without PCM requests a normal source sync', () => {
  assert.equal(
    getPortAudioSourceRetryMode(
      { ...stalledStatus, recovery_required: false },
      'playing',
      false,
    ),
    'sync',
  );
});

test('hardware restart is left to the main window instead of racing source sync', () => {
  assert.equal(
    getPortAudioSourceRetryMode(
      { ...stalledStatus, hardware_state: 'inactive' },
      'playing',
      false,
    ),
    null,
  );
  assert.equal(
    getPortAudioSourceRetryMode(
      { ...stalledStatus, available: false },
      'playing',
      false,
    ),
    null,
  );
});

test('PortAudio recovery helper remains isolated while ordinary cycles use media candidates', () => {
  assert.equal(
    shouldKeepPortAudioCycleScheduling({
      ...stalledStatus,
      running: false,
      recovery_required: true,
    }),
    true,
  );
  assert.equal(
    shouldKeepPortAudioCycleScheduling({
      ...stalledStatus,
      hardware_state: 'inactive',
    }),
    false,
  );
  assert.equal(
    shouldKeepPortAudioCycleScheduling({
      ...stalledStatus,
      preferred_portaudio: false,
    }),
    false,
  );
  assert.match(
    appSource,
    /prepare_audio_media_candidate/,
  );
  assert.doesNotMatch(appSource, /getAudioCycleCoordinatorAction\(/);
});

test('processed video cache changes do not change the PortAudio input source', () => {
  assert.equal(
    resolvePortAudioSourcePath({
      audio_processing_enabled: true,
      current_audio_source: 'original',
      current_audio_reference: null,
      current_video_reference: 'C:\\cache\\processed.mp4',
      source_media: { source_path: 'C:\\media\\source.mp4' },
    }),
    'C:\\media\\source.mp4',
  );
});

test('explicit replacement audio remains the PortAudio input source', () => {
  assert.equal(
    resolvePortAudioSourcePath({
      audio_processing_enabled: true,
      current_audio_source: 'realtime_variant',
      current_audio_reference: 'C:\\audio\\replacement.wav',
      current_video_reference: 'C:\\cache\\processed.mp4',
      source_media: { source_path: 'C:\\media\\source.mp4' },
    }),
    'C:\\audio\\replacement.wav',
  );
});

test('source supersession and incomplete coverage remain retryable', () => {
  assert.equal(isRetryableAudioOutputSyncCode('audio_mixer_candidate_superseded'), true);
  assert.equal(isRetryableAudioOutputSyncCode('audio_mixer_candidate_not_caught_up'), true);
  assert.equal(isRetryableAudioOutputSyncCode('audio_mixer_candidate_ffmpeg_failed'), false);
});

test('pause or stop suppresses only the stale sync it intentionally cancelled', () => {
  assert.equal(
    isExpectedAudioOutputSyncCancellation('audio_mixer_candidate_stale', 'paused'),
    true,
  );
  assert.equal(
    isExpectedAudioOutputSyncCancellation('audio_mixer_start_stale', 'stopped'),
    true,
  );
  assert.equal(
    isExpectedAudioOutputSyncCancellation('audio_mixer_candidate_stale', 'playing'),
    false,
  );
  assert.equal(
    isExpectedAudioOutputSyncCancellation('audio_mixer_candidate_stale', undefined),
    false,
  );
  assert.equal(
    isExpectedAudioOutputSyncCancellation('audio_mixer_candidate_ffmpeg_failed', 'paused'),
    false,
  );
});

test('sync status separates active output, retryable recovery, and hard fallback', () => {
  assert.equal(
    classifyAudioOutputSync({
      preferred_portaudio: true,
      selected_backend: 'portaudio',
      running: true,
      retryable: false,
      reason_code: null,
    }),
    'active',
  );
  assert.equal(
    classifyAudioOutputSync({
      preferred_portaudio: true,
      selected_backend: 'webview',
      running: false,
      retryable: true,
      reason_code: 'audio_mixer_candidate_superseded',
    }),
    'retryable',
  );
  assert.equal(
    classifyAudioOutputSync({
      preferred_portaudio: true,
      selected_backend: 'portaudio',
      running: true,
      retryable: true,
      reason_code: 'audio_mixer_candidate_superseded',
    }),
    'retryable',
  );
  assert.equal(
    classifyAudioOutputSync({
      preferred_portaudio: false,
      selected_backend: 'webview',
      running: false,
      retryable: false,
      reason_code: 'audio_mixer_candidate_ffmpeg_failed',
    }),
    'fallback',
  );
});

test('播放期间 Web Audio 图保持运行，PortAudio 只静音末端输出，卸载时关闭', () => {
  const finalEffectWindow = appSource.slice(
    appSource.indexOf('function FinalEffectWindow()'),
    appSource.indexOf('function DesktopApp()'),
  );
  const contextSync = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function syncWebAudioContextState()'),
    finalEffectWindow.indexOf('function setPortAudioHardwareActive('),
  );
  const userAudioSync = finalEffectWindow.slice(
    finalEffectWindow.indexOf('function syncUserAudioSettings()'),
    finalEffectWindow.indexOf('function syncWebAudioContextState()'),
  );

  assert.match(finalEffectWindow, /audioContextTransitionRef/);
  assert.doesNotMatch(contextSync, /portAudioHardwareRef\.current/);
  assert.match(contextSync, /snapshotRef\.current\?\.playback_state === 'playing'/);
  assert.match(contextSync, /context\.resume\(\)/);
  assert.match(contextSync, /context\.suspend\(\)/);
  assert.match(userAudioSync, /mainMediaVolumeGainRef\.current\.gain\.value = hardwareOut \|\| muted \? 0 : clampVolume\(volume\)/);
  assert.match(userAudioSync, /speakerMuteGainRef\.current\.gain\.value = hardwareOut \? 0 : 1/);
  assert.match(finalEffectWindow, /function setPortAudioHardwareActive[\s\S]*syncWebAudioContextState\(\)/);
  assert.match(finalEffectWindow, /context\?\.close\(\)/);
});
