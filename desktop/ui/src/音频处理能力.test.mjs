import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./音频处理能力.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const capabilityModule = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

const audio = {
  input_gain_db: 1,
  output_gain_db: 2,
  loudness_adjustment_db: -0.5,
  low_eq_db: 0.1,
  mid_eq_db: -0.2,
  high_eq_db: 0.3,
  pitch_shift_semitones: 0.05,
  playback_speed: 1,
  fade_in_ms: 50,
  fade_out_ms: 100,
  reverb_wet_percent: 10,
  sample_rate_hz: 48_000,
  output_bitrate_kbps: 192,
};

test('基础参数全部展示，并区分 FFmpeg 已生效和待应用参数', () => {
  const rows = capabilityModule.buildAudioCapabilityRows(audio, true, true);
  assert.equal(rows.length, 22);
  assert.equal(rows.find((row) => row.key === 'dry_wet_percent').status, 'configured');
  assert.equal(rows.find((row) => row.key === 'total_gain_db').value, 2.5);
  for (const key of ['total_gain_db', 'low_eq_db', 'mid_eq_db', 'high_eq_db']) {
    assert.equal(rows.find((row) => row.key === key).status, 'ready', key);
  }
  for (const key of ['pitch_shift_semitones', 'playback_speed', 'sample_rate_hz', 'output_bitrate_kbps']) {
    assert.equal(rows.find((row) => row.key === key).status, 'configured', key);
  }
  for (const key of ['input_gain_db', 'output_gain_db', 'loudness_adjustment_db', 'fade_in_ms', 'fade_out_ms', 'reverb_wet_percent']) {
    assert.equal(rows.find((row) => row.key === key).status, 'ready', key);
  }
});

test('未启用声音处理时基础参数显示为未启用', () => {
  const rows = capabilityModule.buildAudioCapabilityRows(audio, false, false);
  assert.ok(rows.every((row) => row.status === 'disabled'));
});

test('无效或自动采样率参数不会产生 NaN', () => {
  const rows = capabilityModule.buildAudioCapabilityRows({ sample_rate_hz: null, input_gain_db: Number.NaN }, true, false);
  assert.equal(rows.find((row) => row.key === 'sample_rate_hz').value, null);
  assert.equal(rows.find((row) => row.key === 'input_gain_db').value, null);
  assert.equal(rows.find((row) => row.key === 'total_gain_db').value, null);
});
