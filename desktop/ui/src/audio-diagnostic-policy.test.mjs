import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as ts from 'typescript';

async function importTypeScriptModule(relativePath) {
  const source = await readFile(new URL(relativePath, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: relativePath,
    reportDiagnostics: true,
  });
  assert.deepEqual(compiled.diagnostics ?? [], []);
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputText).toString('base64')}`);
}

function diagnostic(overrides = {}) {
  return {
    version: 1,
    type: 'diagnostic',
    source: 'portaudio-mixed-pcm',
    sequence: 1,
    has_pcm: true,
    sample_rate_hz: 44_100,
    captured_frame_count: 96,
    line: [0, 0.25, -0.25],
    rms_dbfs: -20,
    peak_dbfs: -10,
    low_band_rms_dbfs: -22,
    cutoff_hz: 200,
    mfcc: [0.1],
    mfcc_available: true,
    noise_floor_dbfs: -60,
    snr_db: 40,
    formants_hz: [500, 1_500, 2_500],
    current_formant_hz: 500,
    sent_at_ms: 10_000,
    error: null,
    ...overrides,
  };
}

test('诊断输入校验保留 IPC 数量与数值边界', async () => {
  const policy = await importTypeScriptModule('./desktop/audio-diagnostic-policy.ts');
  assert.equal(policy.isDiagnosticMessage(diagnostic()), true);
  assert.equal(policy.isDiagnosticMessage(diagnostic({ line: [2] })), false);
  assert.equal(policy.isDiagnosticMessage(diagnostic({ formants_hz: [500] })), false);
  assert.equal(policy.isDiagnosticMessage({ ...diagnostic(), extra: undefined }), true);
});

test('新鲜 PortAudio 诊断压过 Web Audio 回退，过期后允许回退接管', async () => {
  const policy = await importTypeScriptModule('./desktop/audio-diagnostic-policy.ts');
  const portAudio = diagnostic();
  const webAudio = diagnostic({ source: 'web-audio-analyser', sequence: null, has_pcm: false });

  assert.equal(policy.selectDiagnosticMessage(portAudio, webAudio, 10_500), portAudio);
  assert.equal(policy.selectDiagnosticMessage(portAudio, webAudio, 11_500), webAudio);
});

test('诊断状态不再依赖每 250ms 更新时间状态', async () => {
  const policy = await importTypeScriptModule('./desktop/audio-diagnostic-policy.ts');
  assert.equal(policy.DIAGNOSTIC_PUBLISH_INTERVAL_MS, 250);
  assert.equal(policy.DIAGNOSTIC_UI_COMMIT_INTERVAL_MS, 1_000);
  assert.equal(policy.DIAGNOSTIC_STALE_AFTER_MS, 1_500);
  assert.equal(policy.getDiagnosticStatus(null, false), '无数据：尚未收到诊断消息');
  assert.equal(policy.getDiagnosticStatus(diagnostic(), false), '数据已过期');
});
