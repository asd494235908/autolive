import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-video-backend-status.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const backendStatus = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

const realtimeGpu = {
  backend: 'realtime_gpu',
  activation: 'active',
  gpu_adapter: null,
  graphics_api: 'd3d11',
  decoder: 'd3d11va',
  filter: 'gpu-next/libplacebo',
  n: { sequence: 7, target_pts_ms: 48_000, status: 'active' },
  n1: { sequence: 8, target_pts_ms: 56_000, status: 'ready' },
  n2: { sequence: 9, target_pts_ms: 64_000, status: 'planned' },
  cycle_drift_ms: 23,
  demotion_reason: null,
  support_completeness: 'complete',
  parameter_support: {
    backend: 'realtime_gpu',
    fullySupported: true,
    parameters: [{
      field: 'video.brightness_percent',
      active: true,
      supported: true,
      mapping: 'shader_param:al_brightness_percent',
      reason: null,
    }],
  },
};

test('mpv/libplacebo 真实运行状态投影为已生效，并展示周期流水线', () => {
  const parsed = backendStatus.parseMediaVideoBackendStatus(realtimeGpu);
  assert.equal(parsed?.decoder, 'd3d11va');

  const view = backendStatus.projectMediaVideoBackendStatus(realtimeGpu);
  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, true);
  assert.equal(view.backend, 'realtime_gpu');
  assert.match(view.label, /实时 GPU已生效/);
  assert.match(view.detail, /D3D11/);
  assert.match(view.detail, /解码 d3d11va/);
  assert.match(view.detail, /N\+2 9\/planned@64000ms/);
  assert.match(view.detail, /周期漂移 \+23ms/);
  assert.doesNotMatch(view.detail, /编码|FFmpeg/);
});

test('CPU4 回退不伪造 GPU 或编码器，并只使用实时 eq+hue', () => {
  const cpu4 = {
    ...realtimeGpu,
    backend: 'cpu4',
    gpu_adapter: null,
    graphics_api: 'software',
    decoder: 'software',
    filter: 'libavfilter eq+hue',
    demotion_reason: 'GPU83 初始化失败',
    parameter_support: {
      backend: 'cpu4',
      fullySupported: true,
      parameters: [],
    },
  };
  const view = backendStatus.projectMediaVideoBackendStatus(cpu4);

  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, true);
  assert.match(view.label, /CPU4 实时回退已生效/);
  assert.match(view.detail, /SOFTWARE/);
  assert.match(view.detail, /libavfilter eq\+hue/);
  assert.doesNotMatch(view.detail, /GPU 设备|编码/);
});

test('source 兜底只表示源画面输出，不声称视频参数生效', () => {
  const view = backendStatus.projectMediaVideoBackendStatus({
    backend: 'source',
    activation: 'active',
    gpu_adapter: null,
    graphics_api: null,
    decoder: null,
    filter: null,
    n: null,
    n1: null,
    n2: null,
    cycle_drift_ms: null,
    demotion_reason: 'CPU4 启动失败',
    support_completeness: 'not_applicable',
    parameter_support: {
      backend: 'source',
      fullySupported: true,
      parameters: [],
    },
  });

  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, false);
  assert.match(view.label, /源画面兜底输出中/);
  assert.match(view.label, /未应用视频参数/);
});

test('实时 GPU 存在未适配活动参数时不能当作完整快照已生效', () => {
  const input = {
    ...realtimeGpu,
    support_completeness: 'incomplete',
    parameter_support: {
      backend: 'realtime_gpu',
      fullySupported: false,
      parameters: [{
        field: 'advanced.picture_in_picture_enabled',
        active: true,
        supported: false,
        mapping: null,
        reason: '实时 GPU 主链尚未映射该参数',
      }],
    },
  };
  const parsed = backendStatus.parseMediaVideoBackendStatus(input);
  const view = backendStatus.projectMediaVideoBackendStatus(input);

  assert.equal(view.valid, true);
  assert.equal(view.effects_applied, false);
  assert.equal(backendStatus.canUseRealtimeVideoBackend(parsed), false);
  assert.match(view.label, /尚未完整生效/);
  assert.match(view.detail, /advanced\.picture_in_picture_enabled/);
});

test('只有完整支持且 available/active 的实时 GPU 计划可以提交', () => {
  const complete = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    activation: 'available',
    n: null,
  });
  assert.equal(backendStatus.canUseRealtimeVideoBackend(complete), true);
  assert.equal(backendStatus.canUseRealtimeVideoBackend({ ...complete, activation: 'failed' }), false);
  assert.equal(backendStatus.canUseRealtimeVideoBackend({ ...complete, support_completeness: 'incomplete' }), false);
});

test('configured 和 available 不能被投影为 active', () => {
  for (const activation of ['configured', 'available']) {
    const view = backendStatus.projectMediaVideoBackendStatus({
      ...realtimeGpu,
      activation,
      n: null,
      support_completeness: 'incomplete',
      parameter_support: { ...realtimeGpu.parameter_support, fullySupported: false },
    });
    assert.equal(view.valid, true);
    assert.equal(view.active, false);
    assert.equal(view.effects_applied, false);
    assert.match(view.label, /尚未生效/);
  }
});

test('旧 FFmpeg 视频状态、缺字段和自相矛盾数据均 fail closed', () => {
  const cases = [
    { ...realtimeGpu, backend: 'ffmpeg_gpu' },
    { ...realtimeGpu, encoder: 'h264_amf' },
    { ...realtimeGpu, backend: 'amd_magic' },
    { ...realtimeGpu, activation: 'ready' },
    { ...realtimeGpu, graphics_api: 'directx12' },
    { ...realtimeGpu, support_completeness: 'partial' },
    { ...realtimeGpu, support_completeness: 'incomplete' },
    { ...realtimeGpu, parameter_support: { ...realtimeGpu.parameter_support, backend: 'cpu4' } },
    { ...realtimeGpu, n: { ...realtimeGpu.n, status: 'configured' } },
    { ...realtimeGpu, cycle_drift_ms: Number.NaN },
  ];
  const missing = { ...realtimeGpu };
  delete missing.decoder;
  cases.push(missing);

  for (const input of cases) {
    assert.equal(backendStatus.parseMediaVideoBackendStatus(input), null);
    const view = backendStatus.projectMediaVideoBackendStatus(input);
    assert.equal(view.valid, false);
    assert.equal(view.active, false);
    assert.equal(view.effects_applied, false);
  }
});
