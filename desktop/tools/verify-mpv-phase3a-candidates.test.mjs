import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  captureDefaultNeutralityFrames,
  deriveProductionCapabilityEvidence,
  parsePhase3aCliArgs,
  phase3aSampleEvidence,
  phase3aSampleSpec,
  reservePhase3aReport,
} from './verify-mpv-phase3a-candidates.mjs';

test('Phase 3A 实机门禁要求逐次确认与不可覆盖报告路径', () => {
  assert.deepEqual(
    parsePhase3aCliArgs(['--confirm-real-media-gate', '--report', 'v5.json']),
    { report: join(process.cwd(), 'v5.json'), help: false, confirmed: true },
  );
  assert.deepEqual(parsePhase3aCliArgs(['--report', 'v5.json']), {
    report: join(process.cwd(), 'v5.json'), help: false, confirmed: false,
  });
  assert.throws(
    () => parsePhase3aCliArgs(['--confirm-real-media-gate', '--confirm-real-media-gate']),
    /不得重复/,
  );
});

test('Phase 3A 真实门禁独占 1080p 全屏客户区，不复用普通隐藏窗口尺寸', async () => {
  const source = await readFile(new URL('./verify-mpv-phase3a-candidates.mjs', import.meta.url), 'utf8');
  assert.match(source, /hiddenWindow:\s*true,[\s\S]*fullscreenWindow:\s*true,/);
});

test('Phase 3A 报告路径在媒体启动前原子占用', async () => {
  const root = await mkdtemp(join(tmpdir(), 'autolive-phase3a-report-'));
  const reportPath = join(root, 'v5.json');
  try {
    const handle = await reservePhase3aReport(reportPath);
    await handle.writeFile('reserved', 'utf8');
    await handle.close();
    await assert.rejects(reservePhase3aReport(reportPath), { code: 'EEXIST' });

    const source = await readFile(new URL('./verify-mpv-phase3a-candidates.mjs', import.meta.url), 'utf8');
    assert.ok(
      source.indexOf('await reservePhase3aReport(options.report)') <
      source.indexOf('await runPhase3aCandidateGate()'),
      '报告路径必须在真实媒体门禁前占用',
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('Phase 3A 专用样本覆盖完整更新窗口且不依赖循环重置', () => {
  const sample = phase3aSampleSpec(120);
  assert.deepEqual(sample, {
    name: '1080p-60fps-phase3a',
    width: 1920,
    height: 1080,
    fps: 60,
    duration: 15,
    gateRole: 'candidate_full_resolution_render',
  });
  assert.equal(sample.duration * sample.fps, 900);
  assert.throws(() => phase3aSampleSpec(99), /不得少于 100/);
});

test('Phase 3A 报告显式记录更新窗口与不跨循环余量', () => {
  assert.deepEqual(phase3aSampleEvidence(phase3aSampleSpec(120), 120), {
    durationSeconds: 15,
    frameCount: 900,
    updateCount: 120,
    minimumSpareDurationSeconds: 2,
    remainingWindowEvidence: 'runtime_pts_and_monotonic_clock',
  });
  assert.throws(
    () => phase3aSampleEvidence({ ...phase3aSampleSpec(120), duration: 2 }, 120),
    /不足以保留两秒实测余量/,
  );
});

test('生产能力从 Rust 映射与已审计 shader 交叉派生并在漂移时 fail-closed', async () => {
  const [rustSource, shaderSource, gateSource] = await Promise.all([
    readFile(new URL('../src-tauri/src/media_video_gpu_effects.rs', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/resources/shaders/gpu83.hook', import.meta.url), 'utf8'),
    readFile(new URL('./verify-mpv-phase3a-candidates.mjs', import.meta.url), 'utf8'),
  ]);
  const evidence = deriveProductionCapabilityEvidence(rustSource, shaderSource);
  assert.equal(evidence.status, 'matched');
  assert.equal(evidence.declaredParameterCount, 83);
  assert.equal(evidence.mappingParameterCount, 83);
  assert.equal(evidence.availableParameterCount, 61);
  assert.equal(evidence.unavailableParameterCount, 22);
  assert.equal(evidence.productionShaderDynamicParameterCount, 61);
  assert.equal(evidence.availableShaderOptions.length, 61);
  assert.equal(evidence.label, '61/83');

  const demotedRustSource = rustSource.replace(/\r?\n(\s*)AVAILABLE\r?\n/, '\n$1UNVERIFIED\n');
  assert.throws(
    () => deriveProductionCapabilityEvidence(demotedRustSource, shaderSource),
    /shader.*capability 身份不一致/,
  );
  const swappedRustSource = rustSource
    .replace(/("video\.brightness_percent"[\s\S]*?)AVAILABLE/, '$1UNVERIFIED')
    .replace(/("video\.color_space_conversion_enabled"[\s\S]*?)UNVERIFIED/, '$1AVAILABLE');
  assert.throws(
    () => deriveProductionCapabilityEvidence(swappedRustSource, shaderSource),
    /shader.*capability 身份不一致/,
  );
  assert.throws(
    () => deriveProductionCapabilityEvidence(rustSource, shaderSource.replace('al_brightness_percent', 'al_brightness_percent_x')),
    /已审计契约不匹配/,
  );
  assert.doesNotMatch(gateSource, /productionCapabilityAfterGate:\s*['"]20\/83['"]/);
  assert.match(gateSource, /productionCapabilityAfterGate:\s*productionCapability\.label/);
});

function fakeNeutralityIpc({ drifting = false, ptsValues = null } = {}) {
  const commands = [];
  const shaders = [];
  let ptsReads = 0;
  return {
    commands,
    async send(command) {
      commands.push(command);
      if (command[0] === 'change-list') {
        shaders.push(command[3]);
        return { data: null };
      }
      if (command[0] !== 'get_property') return { data: null };
      if (command[1] === 'pause') return { data: false };
      if (command[1] === 'glsl-shaders') return { data: [...shaders] };
      if (command[1] === 'time-pos/full') {
        ptsReads += 1;
        if (ptsValues !== null) return { data: ptsValues[ptsReads - 1] ?? ptsValues.at(-1) };
        return { data: drifting && ptsReads > 1 ? 12.04 : 12 };
      }
      return { data: null };
    },
  };
}

test('默认中性证据在同一暂停 PTS 内先取无 shader 帧，再按固定顺序动态追加候选', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'autolive-neutrality-test-'));
  await rm(directory, { recursive: true, force: true });
  const ipc = fakeNeutralityIpc();
  const shaderPaths = ['E:/trusted/a.hook', 'E:/trusted/b.hook'];
  try {
    const evidence = await captureDefaultNeutralityFrames({
      ipc,
      directory,
      shaderPaths,
      defaultOptions: { al_a: 0, al_b: 1 },
      warmupOptions: { al_a: 1, al_b: 0 },
    });
    assert.equal(evidence.loadedShaderCount, 2);
    assert.equal(evidence.anchorPtsSeconds, 12);
    assert.equal(evidence.setupScreenshotCount, 3);
    assert.deepEqual(evidence.warmup, {
      completed: true,
      path: join(directory, 'setup-warmup.png'),
      mediaPtsSeconds: 12,
    });
    assert.deepEqual(evidence.captures.map(({ role, mediaPtsSeconds }) => ({ role, mediaPtsSeconds })), [
      { role: 'no_shader', mediaPtsSeconds: 12 },
      { role: 'candidate_default', mediaPtsSeconds: 12 },
    ]);
    assert.deepEqual(evidence.captures.map(({ path }) => path), [
      join(directory, 'frame-0000.png'),
      join(directory, 'frame-0001.png'),
    ]);
    assert.deepEqual(
      ipc.commands.filter(([name]) => name === 'change-list'),
      shaderPaths.map((path) => ['change-list', 'glsl-shaders', 'append', path]),
    );
    const noShaderCaptureIndex = ipc.commands.findIndex(([name]) => name === 'screenshot-to-file');
    const firstAppendIndex = ipc.commands.findIndex(([name]) => name === 'change-list');
    const warmupSnapshotIndex = ipc.commands.findIndex(
      (command) => command[0] === 'set_property' && command[1] === 'glsl-shader-opts',
    );
    const warmupCaptureIndex = ipc.commands.findIndex(
      (command) => command[0] === 'screenshot-to-file' && command[1].endsWith('setup-warmup.png'),
    );
    const defaultSnapshotIndex = ipc.commands.findLastIndex(
      (command) => command[0] === 'set_property' && command[1] === 'glsl-shader-opts',
    );
    const candidateCaptureIndex = ipc.commands.findLastIndex(([name]) => name === 'screenshot-to-file');
    assert.ok(noShaderCaptureIndex < firstAppendIndex);
    assert.ok(firstAppendIndex < warmupSnapshotIndex);
    assert.ok(warmupSnapshotIndex < warmupCaptureIndex);
    assert.ok(warmupCaptureIndex < defaultSnapshotIndex);
    assert.ok(defaultSnapshotIndex < candidateCaptureIndex);
    assert.deepEqual(ipc.commands[warmupSnapshotIndex], [
      'set_property',
      'glsl-shader-opts',
      'al_a=1,al_b=0',
    ]);
    assert.deepEqual(ipc.commands[defaultSnapshotIndex], [
      'set_property',
      'glsl-shader-opts',
      'al_a=0,al_b=1',
    ]);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('默认中性证据拒绝跨帧对照', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'autolive-neutrality-drift-'));
  await rm(directory, { recursive: true, force: true });
  try {
    await assert.rejects(
      captureDefaultNeutralityFrames({
        ipc: fakeNeutralityIpc({ drifting: true }),
        directory,
        shaderPaths: ['E:/trusted/a.hook'],
        defaultOptions: { al_a: 0 },
        warmupOptions: { al_a: 1 },
      }),
      /跨帧/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('默认中性证据在恢复 defaults 前拒绝 warmup 跨帧', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'autolive-neutrality-warmup-drift-'));
  await rm(directory, { recursive: true, force: true });
  const ipc = fakeNeutralityIpc({ ptsValues: [12, 12, 12.04] });
  try {
    await assert.rejects(
      captureDefaultNeutralityFrames({
        ipc,
        directory,
        shaderPaths: ['E:/trusted/a.hook'],
        defaultOptions: { al_a: 0 },
        warmupOptions: { al_a: 1 },
      }),
      /候选全激活预热帧跨帧/,
    );
    assert.equal(
      ipc.commands.filter(
        (command) => command[0] === 'set_property' && command[1] === 'glsl-shader-opts',
      ).length,
      1,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('默认中性 setup 拒绝缺少 warmupOptions', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'autolive-neutrality-required-'));
  await rm(directory, { recursive: true, force: true });
  try {
    await assert.rejects(
      captureDefaultNeutralityFrames({
        ipc: fakeNeutralityIpc(),
        directory,
        shaderPaths: ['E:/trusted/a.hook'],
        defaultOptions: { al_a: 0 },
      }),
      /warmupOptions 必填/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('候选总门禁固定为零 shader 启动、setup 动态加载和默认/全激活 P99 交替快照', async () => {
  const source = await readFile(new URL('./verify-mpv-phase3a-candidates.mjs', import.meta.url), 'utf8');
  assert.match(source, /shaderPaths:\s*\[\]/);
  assert.match(source, /if \(!options\.confirmed \|\| options\.report === null\)/);
  assert.match(source, /拒绝启动.*--confirm-real-media-gate.*--report/s);
  assert.match(source, /generateSamples\([^;]+\[phase3aSampleSpec\(updateCount\)\]/s);
  assert.match(source, /\.\.\.phase3aSampleEvidence\(sample, updateCount\)/);
  assert.match(source, /allowNoShaderStart:\s*true/);
  assert.match(source, /sessionSetup:\s*async/);
  assert.match(source, /warmupOptions:\s*contract\.combinedActiveOptions/);
  assert.match(source, /warmupCompleted:\s*neutrality\.warmup\.completed/);
  assert.match(source, /setupScreenshotCount:\s*neutrality\.setupScreenshotCount/);
  assert.match(source, /neutralityAnalysisCaptureCount:\s*neutrality\.captures\.length/);
  assert.match(source, /serializeShaderOptions\(contract\.defaults\)/);
  assert.match(source, /serializeShaderOptions\(contract\.combinedActiveOptions\)/);
  assert.match(source, /requireUpdatePlaybackContinuity:\s*true/);
  assert.match(source, /monotonicUpdatePtsWithoutLoop:/);
  assert.match(source, /maximumValidatedResolution:\s*fullyValidated1080p\s*\?\s*'1920x1080'\s*:\s*null/);
});
