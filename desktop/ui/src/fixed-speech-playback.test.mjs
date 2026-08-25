import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const appUrl = new URL('./App.tsx', import.meta.url);

test('固定话术从主窗口通过 BroadcastChannel 发送给最终效果窗口', async () => {
  const source = await readFile(appUrl, 'utf8');
  const play = source.slice(
    source.indexOf('function playCurrentFixedSpeechText'),
    source.indexOf('function cancelCurrentFixedSpeech'),
  );

  assert.match(source, /<FeatureDrawer[\s\S]*title="固定话术"/);
  assert.match(play, /type: 'fixed-speech-command'/);
  assert.match(play, /action: 'speak'/);
  assert.match(play, /operation_id: operationId/);
  assert.match(play, /text: trimmedFixedSpeechText/);
  assert.match(play, /FIXED_SPEECH_ACK_TIMEOUT_MS/);
});

test('最终效果窗口只选本地系统语音并覆盖完成、失败和启动超时', async () => {
  const source = await readFile(appUrl, 'utf8');
  const start = source.slice(
    source.indexOf('async function startFixedSpeech'),
    source.indexOf('function handleRealtimeAudioPlaying'),
  );

  assert.match(source, /selectLocalSpeechVoice\(synthesis\.getVoices\(\)\)/);
  assert.match(start, /new SpeechSynthesisUtterance/);
  assert.match(start, /utterance\.onstart/);
  assert.match(start, /utterance\.onend/);
  assert.match(start, /utterance\.onerror/);
  assert.match(start, /utterance\.volume = 1/);
  assert.doesNotMatch(start, /utterance\.volume = userMutedRef/);
  assert.match(start, /语音启动超时/);
  assert.match(start, /speechSynthesis\.speak\(utterance\)/);
});

test('固定话术从 starting 起暂停插话，朗读结束后保留并恢复当前插话', async () => {
  const source = await readFile(appUrl, 'utf8');
  const start = source.slice(
    source.indexOf('fixedSpeechOperationRef.current = {'),
    source.indexOf('const voice = await waitForLocalSpeechVoice'),
  );
  assert.match(start, /fixedSpeechActiveRef\.current = true/);
  assert.match(start, /pauseInterludePlayback\(\)/);

  const scheduleStart = source.indexOf('const currentClockMs = performance.now();');
  const scheduleEnd = source.indexOf('  }, [sourceUrl]);', scheduleStart);
  const schedule = source.slice(source.lastIndexOf('useEffect(() => {', scheduleStart), scheduleEnd);
  const pauseBranch = schedule.slice(schedule.indexOf('if (shouldPauseInterlude({'));
  assert.match(pauseBranch, /pauseInterludePlayback\(\)/);
  assert.doesNotMatch(pauseBranch, /clearInterludePlayback\(/);

  const finalize = source.slice(
    source.indexOf('function finalizeFixedSpeech'),
    source.indexOf('async function waitForLocalSpeechVoice'),
  );
  assert.match(finalize, /resumeInterludePlayback\(\)/);
  const resume = source.slice(
    source.indexOf('function resumeInterludePlayback'),
    source.indexOf('function publishFixedSpeechStatus'),
  );
  assert.match(resume, /snapshotRef\.current\?\.playback_state !== 'playing'/);
});

test('朗读期间静音最终效果原声，终态与卸载均恢复', async () => {
  const source = await readFile(appUrl, 'utf8');
  const finalize = source.slice(
    source.indexOf('function finalizeFixedSpeech'),
    source.indexOf('async function waitForLocalSpeechVoice'),
  );

  assert.match(source, /fixedSpeechActiveRef\.current \|\|/);
  assert.match(source, /muted=\{userMuted \|\| fixedSpeechActive\}/);
  assert.match(finalize, /speechSynthesis\?\.cancel\(\)/);
  assert.match(finalize, /fixedSpeechActiveRef\.current = false/);
  assert.match(finalize, /syncUserAudioSettings\(\)/);
  assert.match(source, /cancelFixedSpeech\(operationId\)/);
  assert.match(source, /addEventListener\('pagehide', handlePageHide\)/);
  assert.match(source, /removeEventListener\('pagehide', handlePageHide\)/);
});

test('固定话术 UI 保留预设增删改与 1–500 字输入，不含旧克隆流程', async () => {
  const source = await readFile(appUrl, 'utf8');

  assert.match(source, /selectedFixedSpeechPreset \? '更新文案' : '添加文案'/);
  assert.match(source, /deleteSelectedFixedSpeechPreset/);
  assert.match(source, /aria-label="固定话术文本"/);
  assert.match(source, /aria-label="固定话术文本"[\s\S]*maxLength=\{500\}[\s\S]*showCount/);
  assert.doesNotMatch(source, /voice[_A-Z]?clone/i);
  assert.doesNotMatch(source, /XTTS|Demucs|Whisper|预生成|参考人声/);
});

test('导入视频只探测，不打开窗口不播放也不准备语音资源', async () => {
  const source = await readFile(appUrl, 'utf8');
  const importVideo = source.slice(
    source.indexOf('async function importVideo'),
    source.indexOf('function updateInterludeDraft'),
  );

  assert.match(importVideo, /probe_local_videos[\s\S]*request:\s*\{\s*paths\s*\}/);
  assert.doesNotMatch(importVideo, /openFinalEffectWindowFromHome|start_playback/);
  assert.doesNotMatch(importVideo, /voice|model|worker/i);
});
