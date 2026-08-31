import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as ts from 'typescript';

const source = await readFile(new URL('./media-video-backend-status.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2021 },
}).outputText;
const backendStatus = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(compiled)}`);

test('跨 loop 的源内 mpv PTS 投影为周期使用的绝对呈现 PTS', () => {
  assert.equal(backendStatus.mediaVideoPresentationPtsMs({
    ...realtimeGpu,
    loop_index: 2,
    presented_pts_ms: 1_500,
  }, 60_000), 121_500);
  assert.equal(backendStatus.mediaVideoPresentationPtsMs({
    ...realtimeGpu,
    loop_index: null,
  }, 60_000), null);
  assert.equal(backendStatus.mediaVideoPresentationPtsMs(realtimeGpu, 0), null);
});

const realtimeGpu = {
  status_revision: 10,
  playback_generation: 7,
  clock_epoch: 1,
  loop_index: 0,
  backend_epoch: 1,
  fallback_floor_mode: 'gpu_d3d11_zero_copy',
  backend: 'realtime_gpu',
  activation: 'active',
  gpu_adapter: null,
  graphics_api: 'd3d11',
  decoder: 'd3d11va',
  filter: 'gpu-next/libplacebo',
  n: { sequence: 7, target_pts_ms: 48_000, status: 'active' },
  n1: { sequence: 8, target_pts_ms: 56_000, status: 'ready' },
  n2: { sequence: 9, target_pts_ms: 64_000, status: 'planned' },
  apply_state: 'active',
  active_plan_fingerprint: 'plan-7',
  active_cycle_snapshot: {
    sequence: 7,
    fingerprint: 'plan-7',
    video: { brightness_percent: 12.5, horizontal_flip_enabled: false },
    advanced: { wave_intensity: 0.75, band_weights: { 65: 0.2 } },
  },
  pending_plan_fingerprint: 'plan-8',
  actual_source_fps: 30,
  confirmed_change_count: 7,
  cycle_drift_ms: 23,
  av_sync_drift_ms: -12,
  audible_audio_pts_ms: 12_345,
  audio_epoch: 4,
  demotion_reason: null,
  support_completeness: 'complete',
  process_id: 1234,
  transition_started_at_unix_ms: null,
  transition_completed_at_unix_ms: null,
  resume_pts_ms: null,
  presented_pts_ms: 12_345,
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
  unsupported_parameter_count: 0,
  ignored_active_parameter_count: 0,
  ignored_active_parameter_examples: [],
  last_demotion: null,
  demotion_history: [],
  gpu_pass_p99_ms: null,
  frame_drop_count: null,
  decoder_frame_drop_count: null,
  mistimed_frame_count: null,
  delayed_frame_count: null,
  frame_budget_violation_windows: 0,
  physical_paused: false,
  eof: null,
};

const cpu4Parameters = [
  ...[
    ['video.brightness_percent', 'libavfilter:eq:brightness'],
    ['video.contrast_percent', 'libavfilter:eq:contrast'],
    ['video.saturation_percent', 'libavfilter:eq:saturation'],
    ['video.hue_rotation_degrees', 'libavfilter:hue:h'],
  ].map(([field, mapping]) => ({ field, active: true, supported: true, mapping, reason: null })),
  ...Array.from({ length: 79 }, (_, index) => ({
    field: `advanced.cpu4_discarded_${index}`,
    active: index === 0,
    supported: false,
    mapping: null,
    reason: 'CPU4 仅执行四参数',
  })),
];

const cpu4 = {
  ...realtimeGpu,
  backend: 'cpu4',
  fallback_floor_mode: 'cpu4',
  gpu_adapter: null,
  graphics_api: 'd3d11',
  decoder: 'software',
  filter: 'libavfilter eq+hue',
  demotion_reason: 'GPU83 初始化失败',
  parameter_support: {
    backend: 'cpu4',
    fullySupported: false,
    parameters: cpu4Parameters,
  },
  support_completeness: 'incomplete',
  transition_started_at_unix_ms: 100,
  transition_completed_at_unix_ms: 120,
  resume_pts_ms: 8_000,
  unsupported_parameter_count: 79,
  ignored_active_parameter_count: 1,
  ignored_active_parameter_examples: ['advanced.cpu4_discarded_0'],
  last_demotion: {
    from: 'realtime_gpu',
    to: 'cpu4',
    fromMode: 'gpu_vulkan',
    toMode: 'cpu4',
    reason: 'GPU83 初始化失败',
    atUnixMs: 100,
  },
  demotion_history: [{
    from: 'realtime_gpu',
    to: 'cpu4',
    fromMode: 'gpu_vulkan',
    toMode: 'cpu4',
    reason: 'GPU83 初始化失败',
    atUnixMs: 100,
  }],
};

test('解析失败返回稳定 code、field 和 detail，旧 nullable 入口保持兼容', () => {
  const missing = { ...realtimeGpu };
  delete missing.decoder;
  const missingResult = backendStatus.parseMediaVideoBackendStatusResult(missing);
  assert.deepEqual(missingResult, {
    ok: false,
    error: {
      code: 'status_missing_field',
      field: 'decoder',
      detail: '响应缺少必填字段',
    },
  });
  assert.equal(backendStatus.parseMediaVideoBackendStatus(missing), null);

  const invalidResult = backendStatus.parseMediaVideoBackendStatusResult({
    ...realtimeGpu,
    graphics_api: 'directx12',
  });
  assert.equal(invalidResult.ok, false);
  assert.equal(invalidResult.error.code, 'status_invalid_field');
  assert.equal(invalidResult.error.field, 'graphics_api');
});

test('当前周期参数快照必须与 Active N 和已确认指纹原子一致', () => {
  const parsed = backendStatus.parseMediaVideoBackendStatus(realtimeGpu);
  assert.equal(parsed?.active_cycle_snapshot?.video.brightness_percent, 12.5);
  assert.equal(parsed?.active_cycle_snapshot?.advanced.wave_intensity, 0.75);

  for (const [field, active_cycle_snapshot] of [
    ['sequence', { ...realtimeGpu.active_cycle_snapshot, sequence: 8 }],
    ['fingerprint', { ...realtimeGpu.active_cycle_snapshot, fingerprint: 'other-plan' }],
    ['video', { ...realtimeGpu.active_cycle_snapshot, video: [] }],
  ]) {
    const result = backendStatus.parseMediaVideoBackendStatusResult({
      ...realtimeGpu,
      active_cycle_snapshot,
    });
    assert.equal(result.ok, false, field);
    assert.equal(result.error.field, 'active_cycle_snapshot', field);
  }

  const uncertain = backendStatus.parseMediaVideoBackendStatusResult({
    ...realtimeGpu,
    apply_state: 'result_unknown',
  });
  assert.equal(uncertain.ok, false);
  assert.equal(uncertain.error.field, 'active_cycle_snapshot');

  const waiting = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    apply_state: 'result_unknown',
    active_cycle_snapshot: null,
  });
  assert.equal(waiting?.active_cycle_snapshot, null);
});

test('Rust lifecycle 作为兼容字段被保留并严格校验', () => {
  const parsed = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    lifecycle: 'active',
  });
  assert.equal(parsed?.lifecycle, 'active');

  const invalid = backendStatus.parseMediaVideoBackendStatusResult({
    ...realtimeGpu,
    lifecycle: 'ready',
  });
  assert.equal(invalid.ok, false);
  assert.equal(invalid.error.field, 'lifecycle');
});

test('不可用状态 detail 区分 IPC 与 DTO，并对长错误脱敏限长', () => {
  const diagnostic = backendStatus.createMediaVideoBackendIpcDiagnostic(
    `token=super-secret C:\\private\\movie.mp4 ${'x'.repeat(500)}`,
  );
  const view = backendStatus.projectMediaVideoBackendStatus(null, diagnostic);
  assert.equal(view.valid, false);
  assert.match(view.detail, /IPC 调用失败/);
  assert.match(view.detail, /敏感信息/);
  assert.doesNotMatch(view.detail, /super-secret|private|movie\.mp4/);
  assert.ok(view.detail.length < 280);

  const dtoView = backendStatus.projectMediaVideoBackendStatus({ ...realtimeGpu, decoder: 'bad' });
  assert.match(dtoView.detail, /DTO 校验失败（decoder）/);
});

test('尚未取得后端状态不是非法 IPC 响应', () => {
  const view = backendStatus.projectMediaVideoBackendStatus(null, null);
  assert.equal(view.valid, true);
  assert.equal(view.active, false);
  assert.equal(view.effects_applied, false);
  assert.match(view.label, /状态读取中/);
  assert.doesNotMatch(view.detail, /DTO 校验失败|响应必须是对象/);
});

test('长降级原因不再使整份运行状态失效，展示时仍会脱敏截断', () => {
  const longReason = `token=runtime-secret C:\\private\\source.mp4 ${'原因'.repeat(400)}`;
  const input = { ...realtimeGpu, demotion_reason: longReason };
  assert.notEqual(backendStatus.parseMediaVideoBackendStatus(input), null);

  const view = backendStatus.projectMediaVideoBackendStatus(input);
  assert.equal(view.valid, true);
  assert.match(view.detail, /降级原因：\[敏感信息\]/);
  assert.doesNotMatch(view.detail, /runtime-secret|private|source\.mp4/);
  assert.ok(view.detail.length < 600);
});

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
  assert.match(view.detail, /音画漂移 -12ms/);
  assert.doesNotMatch(view.detail, /编码|FFmpeg/);
});

test('视频状态展示已观测的 GPU pass P99 和帧健康计数', () => {
  const view = backendStatus.projectMediaVideoBackendStatus({
    ...realtimeGpu,
    gpu_pass_p99_ms: 9.75,
    frame_drop_count: 2,
    decoder_frame_drop_count: 1,
    mistimed_frame_count: 3,
    delayed_frame_count: 4,
    frame_budget_violation_windows: 2,
  });

  assert.equal(view.valid, true);
  assert.match(view.detail, /GPU pass P99 9.75ms/);
  assert.match(view.detail, /VO 丢帧 2/);
  assert.match(view.detail, /解码丢帧 1/);
  assert.match(view.detail, /错时帧 3/);
  assert.match(view.detail, /延迟帧 4/);
  assert.match(view.detail, /帧预算连续异常 2\/3/);
});

test('GPU 接受实际软件解码，受管 Original 可明确标记解码器未报告', () => {
  const gpu = backendStatus.projectMediaVideoBackendStatus({
    ...realtimeGpu,
    decoder: 'software',
  });
  assert.equal(gpu.valid, true);
  assert.match(gpu.detail, /解码 software/);

  const original = {
    ...realtimeGpu,
    backend: 'source',
    n: null,
    n1: null,
    n2: null,
    active_plan_fingerprint: null,
    active_cycle_snapshot: null,
    decoder: 'unreported-original',
    filter: 'Original（无 shader）',
    support_completeness: 'not_applicable',
    parameter_support: {
      backend: 'source',
      fullySupported: true,
      parameters: [],
    },
  };
  const originalView = backendStatus.projectMediaVideoBackendStatus(original);
  assert.equal(originalView.valid, true);
  assert.match(originalView.detail, /解码 未报告（Original）/);
});

test('CPU4 回退不伪造 GPU 或编码器，并只使用实时 eq+hue', () => {
  const view = backendStatus.projectMediaVideoBackendStatus(cpu4);

  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, true);
  assert.equal(backendStatus.canPrepareManagedVideoBackend(
    backendStatus.parseMediaVideoBackendStatus(cpu4),
  ), true);
  assert.match(view.label, /CPU4 实时回退已生效/);
  assert.match(view.detail, /D3D11/);
  assert.match(view.detail, /恢复 PTS 8000ms/);
  assert.match(view.detail, /本周期未参与参数 1 个/);
  assert.match(view.detail, /CPU4 固定未执行 79 项/);
  assert.match(view.detail, /libavfilter eq\+hue/);
  assert.doesNotMatch(view.detail, /GPU 设备|编码/);
});

test('CPU4 每一种 activation 都必须携带精确 83 项、4 支持、79 不支持', () => {
  for (const activation of ['active', 'available', 'configured', 'failed']) {
    const valid = {
      ...cpu4,
      activation,
      n: activation === 'active' ? cpu4.n : null,
      active_cycle_snapshot: activation === 'active' ? cpu4.active_cycle_snapshot : null,
      process_id: ['active', 'available'].includes(activation) ? cpu4.process_id : null,
      physical_paused: ['active', 'available'].includes(activation) ? false : null,
      presented_pts_ms: ['active', 'available'].includes(activation) ? cpu4.presented_pts_ms : null,
      av_sync_drift_ms: activation === 'active' ? cpu4.av_sync_drift_ms : null,
      audible_audio_pts_ms: activation === 'active' ? cpu4.audible_audio_pts_ms : null,
      audio_epoch: activation === 'active' ? cpu4.audio_epoch : null,
    };
    assert.notEqual(backendStatus.parseMediaVideoBackendStatus(valid), null);
    assert.equal(backendStatus.parseMediaVideoBackendStatus({
      ...valid,
      parameter_support: { ...valid.parameter_support, parameters: [] },
      unsupported_parameter_count: 0,
      ignored_active_parameter_count: 0,
      ignored_active_parameter_examples: [],
    }), null);
  }
});

test('source 兜底只表示源画面输出，不声称视频参数生效', () => {
  const view = backendStatus.projectMediaVideoBackendStatus({
    status_revision: 1,
    playback_generation: 7,
    clock_epoch: 1,
    loop_index: 0,
    backend_epoch: 3,
    fallback_floor_mode: 'original',
    backend: 'source',
    activation: 'active',
    gpu_adapter: null,
    graphics_api: null,
    decoder: null,
    filter: null,
    n: null,
    n1: null,
    n2: null,
    apply_state: 'idle',
    active_plan_fingerprint: null,
    active_cycle_snapshot: null,
    pending_plan_fingerprint: null,
    actual_source_fps: null,
    confirmed_change_count: 0,
    cycle_drift_ms: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
    demotion_reason: 'CPU4 启动失败',
    support_completeness: 'not_applicable',
    process_id: null,
    transition_started_at_unix_ms: 100,
    transition_completed_at_unix_ms: 120,
    resume_pts_ms: 8_000,
    presented_pts_ms: null,
    parameter_support: {
      backend: 'source',
      fullySupported: true,
      parameters: [],
    },
    unsupported_parameter_count: 0,
    ignored_active_parameter_count: 0,
    ignored_active_parameter_examples: [],
    last_demotion: null,
    demotion_history: [],
    gpu_pass_p99_ms: null,
    frame_drop_count: null,
    decoder_frame_drop_count: null,
    mistimed_frame_count: null,
    delayed_frame_count: null,
    frame_budget_violation_windows: 0,
    physical_paused: null,
    eof: null,
  });

  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, false);
  assert.match(view.label, /源画面兜底输出中/);
  assert.match(view.label, /未应用视频参数/);
});

test('受管 Original 保留原生输出信息和进程号，但不声称参数生效', () => {
  const input = {
    status_revision: 1,
    playback_generation: 7,
    clock_epoch: 1,
    loop_index: 0,
    backend_epoch: 2,
    fallback_floor_mode: 'gpu_d3d11_zero_copy',
    backend: 'source',
    activation: 'active',
    gpu_adapter: null,
    graphics_api: 'd3d11',
    decoder: 'd3d11va',
    filter: 'Original（无 shader）',
    n: null,
    n1: null,
    n2: null,
    apply_state: 'idle',
    active_plan_fingerprint: null,
    active_cycle_snapshot: null,
    pending_plan_fingerprint: null,
    actual_source_fps: 30,
    confirmed_change_count: 0,
    cycle_drift_ms: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
    demotion_reason: null,
    support_completeness: 'not_applicable',
    process_id: 4321,
    transition_started_at_unix_ms: null,
    transition_completed_at_unix_ms: null,
    resume_pts_ms: null,
    presented_pts_ms: 8_000,
    parameter_support: {
      backend: 'source',
      fullySupported: true,
      parameters: [],
    },
    unsupported_parameter_count: 0,
    ignored_active_parameter_count: 0,
    ignored_active_parameter_examples: [],
    last_demotion: null,
    demotion_history: [],
    gpu_pass_p99_ms: null,
    frame_drop_count: null,
    decoder_frame_drop_count: null,
    mistimed_frame_count: null,
    delayed_frame_count: null,
    frame_budget_violation_windows: 0,
    physical_paused: false,
    eof: null,
  };

  const parsed = backendStatus.parseMediaVideoBackendStatus(input);
  const view = backendStatus.projectMediaVideoBackendStatus(input);
  assert.equal(parsed?.process_id, 4321);
  assert.equal(view.valid, true);
  assert.equal(view.active, true);
  assert.equal(view.effects_applied, false);
  assert.match(view.detail, /D3D11/);
  assert.match(view.detail, /Original（无 shader）/);

  for (const apply_state of ['source_transitioning', 'applying', 'result_unknown']) {
    const transitioning = backendStatus.projectMediaVideoBackendStatus({ ...input, apply_state });
    assert.equal(transitioning.valid, true);
    assert.equal(transitioning.active, false);
    assert.equal(transitioning.effects_applied, false);
    assert.match(transitioning.label, /状态待确认/);
  }

  for (const [filter, overrides] of [
    ['Original（中性 shader）', {
      fallback_floor_mode: 'gpu_d3d11_zero_copy',
      graphics_api: 'd3d11',
      decoder: 'd3d11va',
    }],
    ['Original（CPU4 中性参数）', {
      fallback_floor_mode: 'cpu4',
      graphics_api: 'd3d11',
      decoder: 'software',
    }],
  ]) {
    const neutralInput = { ...input, ...overrides, filter };
    const neutralParsed = backendStatus.parseMediaVideoBackendStatus(neutralInput);
    const neutralView = backendStatus.projectMediaVideoBackendStatus(neutralInput);
    assert.equal(neutralParsed?.filter, filter);
    assert.equal(neutralView.valid, true);
    assert.equal(neutralView.active, true);
    assert.equal(neutralView.effects_applied, false);
    assert.match(neutralView.detail, new RegExp(filter));
  }

  for (const invalid of [
    { ...input, filter: 'Original（中性 shader）', fallback_floor_mode: 'cpu4', decoder: 'software' },
    { ...input, filter: 'Original（中性 shader）', fallback_floor_mode: 'gpu_vulkan_copy', graphics_api: 'd3d11' },
    { ...input, filter: 'Original（CPU4 中性参数）', fallback_floor_mode: 'gpu_d3d11_zero_copy' },
    { ...input, filter: 'Original（未知旁路）' },
  ]) {
    assert.equal(backendStatus.parseMediaVideoBackendStatus(invalid), null);
  }
});

test('EOF fact is strict and bound to generation/backend/clock/loop identity', () => {
  const eof = {
    playback_generation: 7,
    backend_epoch: 1,
    clock_epoch: 1,
    loop_index: 0,
  };
  const parsed = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    physical_paused: true,
    eof,
  });
  assert.deepEqual(parsed?.eof, eof);
  for (const invalid of [true, false, 1, 'true', {}, { ...eof, backend_epoch: -1 }]) {
    assert.equal(backendStatus.parseMediaVideoBackendStatus({
      ...realtimeGpu,
      physical_paused: true,
      eof: invalid,
    }), null);
  }
});

test('物理暂停是受管 mpv 必填事实，EOF 只能与真实暂停同时出现', () => {
  const missing = { ...realtimeGpu };
  delete missing.physical_paused;
  assert.equal(backendStatus.parseMediaVideoBackendStatus(missing), null);
  assert.equal(backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    physical_paused: 'false',
  }), null);
  assert.equal(backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    physical_paused: false,
    eof: {
      playback_generation: 7,
      backend_epoch: 1,
      clock_epoch: 1,
      loop_index: 0,
    },
  }), null);
});

test('process_id 与 physical_paused 必须双向配对', () => {
  for (const [process_id, physical_paused] of [
    [null, false],
    [null, true],
    [1234, null],
  ]) {
    const result = backendStatus.parseMediaVideoBackendStatusResult({
      ...realtimeGpu,
      process_id,
      physical_paused,
      presented_pts_ms: process_id === null ? null : realtimeGpu.presented_pts_ms,
      av_sync_drift_ms: process_id === null ? null : realtimeGpu.av_sync_drift_ms,
      audible_audio_pts_ms: process_id === null ? null : realtimeGpu.audible_audio_pts_ms,
      audio_epoch: process_id === null ? null : realtimeGpu.audio_epoch,
    });
    assert.equal(result.ok, false);
    assert.equal(result.error.field, 'physical_paused');
  }

  assert.notEqual(backendStatus.parseMediaVideoBackendStatus(realtimeGpu), null);
  assert.notEqual(backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    activation: 'available',
    lifecycle: 'spawned',
    n: null,
    active_cycle_snapshot: null,
    cycle_drift_ms: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
    physical_paused: true,
  }), null);
});

test('无进程状态不得残留 presented_pts_ms 或 eof', () => {
  const noProcess = {
    ...cpu4,
    activation: 'failed',
    n: null,
    active_cycle_snapshot: null,
    process_id: null,
    physical_paused: null,
    presented_pts_ms: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
  };
  assert.notEqual(backendStatus.parseMediaVideoBackendStatus(noProcess), null);

  for (const [field, value] of [
    ['presented_pts_ms', 12_345],
    ['eof', {
      playback_generation: noProcess.playback_generation,
      backend_epoch: noProcess.backend_epoch,
      clock_epoch: noProcess.clock_epoch,
      loop_index: noProcess.loop_index,
    }],
  ]) {
    const result = backendStatus.parseMediaVideoBackendStatusResult({
      ...noProcess,
      [field]: value,
    });
    assert.equal(result.ok, false);
    assert.equal(result.error.field, field);
  }
});

test('首个周期提交前的 available 原生进程可以上报 EOF 以推进多文件轮播', () => {
  const expected = {
    playback_generation: 7,
    backend_epoch: 1,
    clock_epoch: 1,
    loop_index: 0,
  };
  const eof = { ...expected };
  const parsed = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    activation: 'available',
    lifecycle: 'spawned',
    physical_paused: true,
    n: null,
    active_cycle_snapshot: null,
    cycle_drift_ms: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
    eof,
  });
  assert.deepEqual(parsed?.eof, eof);
});

test('native surface accepts active or pre-commit available mpv with the exact playback identity and PTS', () => {
  const expected = { playback_generation: 7, clock_epoch: 1, loop_index: 0 };
  assert.equal(backendStatus.canUseManagedNativeVideo(realtimeGpu, expected), true);
  assert.equal(backendStatus.canUseManagedNativeVideo({ ...realtimeGpu, activation: 'available', n: null }, expected), true);
  assert.equal(backendStatus.canUseManagedNativeVideo({ ...realtimeGpu, process_id: null, presented_pts_ms: null }, expected), false);
  assert.equal(backendStatus.canUseManagedNativeVideo({ ...realtimeGpu, presented_pts_ms: null }, expected), false);
  for (const field of ['playback_generation', 'clock_epoch', 'loop_index']) {
    assert.equal(backendStatus.canUseManagedNativeVideo(realtimeGpu, {
      ...expected,
      [field]: expected[field] + 1,
    }), false);
  }
});

test('managed mpv EOF ownership stays sticky across transient status gaps and one-loop handoff', () => {
  const loop0 = { playback_generation: 7, clock_epoch: 1, loop_index: 0 };
  const loop1 = { ...loop0, loop_index: 1 };
  const claimed = backendStatus.resolveManagedNativeVideoOwnership(
    null,
    realtimeGpu,
    loop0,
    true,
  );

  assert.deepEqual(claimed, loop0);
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(null, {
      ...realtimeGpu,
      presented_pts_ms: null,
    }, loop0, true),
    null,
  );
  assert.deepEqual(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, null, loop1, true),
    loop1,
  );
  assert.deepEqual(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, {
      ...realtimeGpu,
      loop_index: 0,
      eof: null,
    }, loop1, true),
    loop1,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, realtimeGpu, loop1, false),
    null,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, realtimeGpu, {
      playback_generation: 8,
      clock_epoch: 2,
      loop_index: 0,
    }, true),
    null,
  );
});

test('current-identity stop or processless Source fallback explicitly releases managed EOF ownership', () => {
  const expected = { playback_generation: 7, clock_epoch: 1, loop_index: 3 };
  const claimed = { ...expected };
  const sourceFallback = {
    ...realtimeGpu,
    loop_index: 3,
    backend: 'source',
    activation: 'active',
    lifecycle: 'active',
    process_id: null,
    presented_pts_ms: null,
  };
  const stopped = {
    ...sourceFallback,
    activation: 'failed',
    lifecycle: 'stopped',
  };

  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, sourceFallback, expected, true),
    null,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(claimed, stopped, expected, true),
    null,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(
      { ...expected, loop_index: 2 },
      { ...stopped, loop_index: 2 },
      expected,
      true,
    ),
    null,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(
      claimed,
      { ...stopped, loop_index: 2 },
      expected,
      true,
    ),
    null,
  );
  assert.equal(
    backendStatus.resolveManagedNativeVideoOwnership(
      { ...expected, loop_index: 1 },
      null,
      expected,
      true,
    ),
    null,
  );
});

test('only an active GPU or CPU4 N slot with the exact playback identity is presented', () => {
  const expected = { playback_generation: 7, clock_epoch: 1, loop_index: 0 };
  assert.equal(backendStatus.isCurrentMediaVideoCyclePresented(realtimeGpu, expected), true);
  assert.equal(backendStatus.isCurrentMediaVideoCyclePresented({
    ...realtimeGpu,
    loop_index: 1,
  }, expected), false);
  assert.equal(backendStatus.isCurrentMediaVideoCyclePresented({
    ...realtimeGpu,
    clock_epoch: 2,
  }, expected), false);
  assert.equal(backendStatus.isCurrentMediaVideoCyclePresented({
    ...realtimeGpu,
    n: null,
  }, expected), false);
  assert.equal(backendStatus.isCurrentMediaVideoCyclePresented({
    ...realtimeGpu,
    n: { ...realtimeGpu.n, status: 'ready' },
  }, expected), false);
  assert.equal(backendStatus.isCurrentMediaVideoCycleFailed({
    ...realtimeGpu,
    activation: 'failed',
  }, expected), true);
  assert.equal(backendStatus.isCurrentMediaVideoCycleFailed({
    ...realtimeGpu,
    activation: 'failed',
    loop_index: expected.loop_index + 1,
  }, expected), false);
});

test('实时 GPU 存在未适配活动参数时不能当作完整快照已生效', () => {
  const input = {
    ...realtimeGpu,
    support_completeness: 'incomplete',
    unsupported_parameter_count: 1,
    ignored_active_parameter_count: 1,
    ignored_active_parameter_examples: ['advanced.picture_in_picture_enabled'],
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
    active_cycle_snapshot: null,
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
  });
  assert.equal(backendStatus.canUseRealtimeVideoBackend(complete), true);
  assert.equal(backendStatus.canUseRealtimeVideoBackend({ ...complete, activation: 'failed' }), false);
  assert.equal(backendStatus.canUseRealtimeVideoBackend({ ...complete, support_completeness: 'incomplete' }), false);
});

test('已有活动 N 槽位时不得把渲染器降写为 available', () => {
  const result = backendStatus.parseMediaVideoBackendStatusResult({
    ...realtimeGpu,
    activation: 'available',
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
  });

  assert.equal(result.ok, false);
  assert.equal(result.error.field, 'activation');
});

test('跨厂商四档 GPU 状态接受 D3D11 copy、软件解码与五步降级历史', () => {
  const modes = [
    'gpu_d3d11_copy',
    'gpu_vulkan_copy',
    'gpu_software_decode',
    'cpu4',
    'original',
  ];
  const demotionHistory = modes.map((toMode, index) => ({
    from: index < 4 ? 'realtime_gpu' : 'cpu4',
    to: index < 3 ? 'realtime_gpu' : index === 3 ? 'cpu4' : 'source',
    fromMode: index === 0 ? 'gpu_d3d11_zero_copy' : modes[index - 1],
    toMode,
    reason: `probe ${index + 1} failed`,
    atUnixMs: index + 1,
  }));
  const d3d11Copy = {
    ...realtimeGpu,
    fallback_floor_mode: 'gpu_d3d11_copy',
    decoder: 'd3d11va-copy',
    demotion_reason: demotionHistory[0].reason,
    last_demotion: demotionHistory[0],
    demotion_history: demotionHistory.slice(0, 1),
  };
  assert.notEqual(backendStatus.parseMediaVideoBackendStatus(d3d11Copy), null);

  const terminal = {
    ...d3d11Copy,
    fallback_floor_mode: 'original',
    backend: 'source',
    activation: 'failed',
    lifecycle: 'failed',
    graphics_api: null,
    decoder: null,
    filter: null,
    n: null,
    active_cycle_snapshot: null,
    process_id: null,
    physical_paused: null,
    parameter_support: { backend: 'source', fullySupported: true, parameters: [] },
    support_completeness: 'not_applicable',
    demotion_reason: demotionHistory[4].reason,
    last_demotion: demotionHistory[4],
    demotion_history: demotionHistory,
    unsupported_parameter_count: 0,
    ignored_active_parameter_count: 0,
    ignored_active_parameter_examples: [],
    av_sync_drift_ms: null,
    audible_audio_pts_ms: null,
    audio_epoch: null,
    presented_pts_ms: null,
  };
  assert.notEqual(backendStatus.parseMediaVideoBackendStatus(terminal), null);
});

test('单项循环提交后等待 Rust 新 loop 状态期间继续保持原生表面', () => {
  const previousLoop = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    loop_index: 8,
    physical_paused: true,
    eof: {
      playback_generation: 7,
      backend_epoch: realtimeGpu.backend_epoch,
      clock_epoch: 1,
      loop_index: 8,
    },
  });
  assert.equal(backendStatus.canKeepManagedNativeVideoDuringLoopTransition(previousLoop, {
    playback_generation: 7,
    clock_epoch: 1,
    loop_index: 9,
  }), true);
  assert.equal(backendStatus.canKeepManagedNativeVideoDuringLoopTransition(previousLoop, {
    playback_generation: 8,
    clock_epoch: 1,
    loop_index: 0,
  }), false);
  assert.equal(backendStatus.canKeepManagedNativeVideoDuringLoopTransition({
    ...previousLoop,
    activation: 'failed',
  }, {
    playback_generation: 7,
    clock_epoch: 1,
    loop_index: 9,
  }), false);
  assert.equal(backendStatus.canKeepManagedNativeVideoDuringLoopTransition({
    ...previousLoop,
    loop_index: 7,
  }, {
    playback_generation: 7,
    clock_epoch: 1,
    loop_index: 9,
  }), false);
});

test('视频处理开启且同代 GPU/CPU4 进程健康时禁止 Original ensure', () => {
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, realtimeGpu, 7), false);
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...cpu4,
    playback_generation: 7,
    activation: 'available',
  }, 7), false);
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...realtimeGpu,
    playback_generation: 6,
  }, 7), true);
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...realtimeGpu,
    activation: 'failed',
    lifecycle: 'failed',
  }, 7), true);
});

test('视频处理开启且中性 Source 仍由 GPU/CPU4 物理会话承载时禁止 Original ensure', () => {
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...realtimeGpu,
    backend: 'source',
    activation: 'available',
    lifecycle: 'spawned',
    apply_state: 'source_transitioning',
    n: null,
    n1: null,
    n2: null,
  }, 7), false);
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...cpu4,
    backend: 'source',
    activation: 'available',
    lifecycle: 'spawned',
    apply_state: 'source_transitioning',
    n: null,
    n1: null,
    n2: null,
  }, 7), false);
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(true, {
    ...realtimeGpu,
    backend: 'source',
    activation: 'active',
    fallback_floor_mode: 'original',
  }, 7), true);
});

test('视频处理关闭时允许 Original ensure', () => {
  assert.equal(backendStatus.shouldEnsureOriginalVideoRenderer(false, realtimeGpu, 7), true);
});

test('状态按 Rust status_revision 单调接收，前端 clock/loop 不再决定新旧', () => {
  const current = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 8,
  });
  const oldBackend = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 9,
    backend_epoch: 3,
    clock_epoch: 99,
    loop_index: 99,
  });
  const oldClock = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 9,
    backend_epoch: 4,
    clock_epoch: 5,
    loop_index: 99,
  });
  const oldLoop = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 9,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 7,
  });
  const oldGeneration = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    playback_generation: 6,
    backend_epoch: 9,
    clock_epoch: 99,
    loop_index: 99,
  });
  const newerLoop = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 11,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 9,
  });
  const newerBackendWithOldLoop = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 11,
    backend_epoch: 5,
    clock_epoch: 6,
    loop_index: 7,
  });
  const nextGeneration = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: 12,
    playback_generation: 8,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  });
  const expected = {
    playback_generation: 7,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 8,
  };
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, oldBackend, expected).status, current);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, oldClock, expected).status, current);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, oldLoop, expected).status, current);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, newerBackendWithOldLoop, expected).status, newerBackendWithOldLoop);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, oldGeneration, expected).status, current);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, newerLoop, expected).status, newerLoop);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, nextGeneration, {
    playback_generation: 8,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  }).status, nextGeneration);
  assert.equal(backendStatus.acceptMediaVideoBackendStatusResult(current, null, expected).status, current);
});

test('状态身份拒绝返回可展示原因，EOF 不再被前端本地 clock epoch 拦截', () => {
  const current = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 8,
  });
  const older = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    playback_generation: 6,
  });
  const rejected = backendStatus.acceptMediaVideoBackendStatusResult(current, older, {
    playback_generation: 7,
    backend_epoch: 4,
    clock_epoch: 6,
    loop_index: 8,
  });
  assert.equal(rejected.accepted, false);
  assert.equal(rejected.status, current);
  assert.equal(rejected.diagnostic.code, 'status_identity_rejected');
  assert.match(backendStatus.formatMediaVideoBackendDiagnostic(rejected.diagnostic), /状态身份拒绝/);

  const eof = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    physical_paused: true,
    eof: {
      playback_generation: 7,
      backend_epoch: 1,
      clock_epoch: 1,
      loop_index: 0,
    },
  });
  const acceptedEof = backendStatus.acceptMediaVideoBackendStatusResult(null, eof, {
    playback_generation: 7,
    backend_epoch: 1,
    clock_epoch: 2,
    loop_index: 1,
  });
  assert.equal(acceptedEof.accepted, true);
  assert.equal(acceptedEof.status, eof);
  const staleGenerationEof = backendStatus.acceptMediaVideoBackendStatusResult(null, eof, {
    playback_generation: 8,
    backend_epoch: 1,
    clock_epoch: 1,
    loop_index: 0,
  });
  assert.equal(staleGenerationEof.accepted, false);
  assert.match(staleGenerationEof.diagnostic.detail, /不属于当前播放代次/);
});

test('保留的旧状态遇到身份拒绝或 IPC 诊断时必须 fail closed，诊断清除后恢复', () => {
  const current = backendStatus.parseMediaVideoBackendStatus(realtimeGpu);
  const rejected = backendStatus.acceptMediaVideoBackendStatusResult(current, null, {
    playback_generation: 7,
    backend_epoch: 1,
    clock_epoch: 1,
    loop_index: 0,
  });

  const rejectedView = backendStatus.projectMediaVideoBackendStatus(
    rejected.status,
    rejected.diagnostic,
  );
  assert.equal(rejectedView.valid, false);
  assert.equal(rejectedView.active, false);
  assert.equal(rejectedView.effects_applied, false);
  assert.equal(rejectedView.backend, null);
  assert.match(rejectedView.label, /状态待确认/);
  assert.match(rejectedView.detail, /状态身份拒绝/);

  const ipcView = backendStatus.projectMediaVideoBackendStatus(
    current,
    backendStatus.createMediaVideoBackendIpcDiagnostic('读取超时'),
  );
  assert.equal(ipcView.valid, false);
  assert.equal(ipcView.active, false);
  assert.equal(ipcView.effects_applied, false);
  assert.equal(ipcView.backend, null);
  assert.match(ipcView.label, /状态不可用/);
  assert.match(ipcView.detail, /IPC 调用失败/);

  const recoveredView = backendStatus.projectMediaVideoBackendStatus(current, null);
  assert.equal(recoveredView.valid, true);
  assert.equal(recoveredView.active, true);
  assert.equal(recoveredView.effects_applied, true);
  assert.match(recoveredView.label, /已生效/);
});

test('状态修订回退与未完成物理确认均 fail closed，确认后恢复 effective', () => {
  const current = backendStatus.parseMediaVideoBackendStatus(realtimeGpu);
  const rollback = backendStatus.parseMediaVideoBackendStatus({
    ...realtimeGpu,
    status_revision: realtimeGpu.status_revision - 1,
  });
  const rejected = backendStatus.acceptMediaVideoBackendStatusResult(current, rollback, {
    playback_generation: 7,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  });
  assert.equal(rejected.accepted, false);
  assert.equal(rejected.status, current);
  assert.equal(rejected.lastObserved, rollback);
  assert.equal(rejected.effective, null);
  assert.equal(rejected.diagnostic.code, 'status_revision_rejected');

  const repeated = backendStatus.acceptMediaVideoBackendStatusResult(current, current, {
    playback_generation: 7,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  });
  assert.equal(repeated.accepted, true);
  assert.equal(repeated.status, current);
  assert.equal(repeated.effective, current);

  const conflict = backendStatus.acceptMediaVideoBackendStatusResult(current, {
    ...current,
    confirmed_change_count: current.confirmed_change_count + 1,
  }, {
    playback_generation: 7,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  });
  assert.equal(conflict.accepted, false);
  assert.equal(conflict.effective, null);
  assert.equal(conflict.diagnostic.code, 'status_revision_rejected');

  for (const apply_state of [
    'source_transitioning',
    'ready',
    'applying',
    'result_unknown',
    'readback_confirmed',
    'presented_confirmed',
  ]) {
    const transitional = {
      ...realtimeGpu,
      status_revision: realtimeGpu.status_revision + 1,
      apply_state,
      active_cycle_snapshot: null,
    };
    const accepted = backendStatus.acceptMediaVideoBackendStatusResult(current, transitional, {
      playback_generation: 7,
      backend_epoch: 0,
      clock_epoch: 1,
      loop_index: 0,
    });
    assert.equal(accepted.accepted, true);
    assert.equal(accepted.effective, null);
    const view = backendStatus.projectMediaVideoBackendStatus(transitional);
    assert.equal(view.valid, true);
    assert.equal(view.active, false);
    assert.equal(view.effects_applied, false);
    assert.match(view.label, /状态待确认/);
  }

  const confirmed = {
    ...realtimeGpu,
    status_revision: realtimeGpu.status_revision + 2,
    apply_state: 'active',
    active_plan_fingerprint: 'confirmed-plan',
    active_cycle_snapshot: {
      ...realtimeGpu.active_cycle_snapshot,
      fingerprint: 'confirmed-plan',
    },
  };
  const recovered = backendStatus.acceptMediaVideoBackendStatusResult(current, confirmed, {
    playback_generation: 7,
    backend_epoch: 0,
    clock_epoch: 1,
    loop_index: 0,
  });
  assert.equal(recovered.accepted, true);
  assert.equal(recovered.effective, recovered.status);
  assert.equal(backendStatus.projectMediaVideoBackendStatus(confirmed).effects_applied, true);
});

test('configured 和 available 不能被投影为 active', () => {
  for (const activation of ['configured', 'available']) {
    const view = backendStatus.projectMediaVideoBackendStatus({
      ...realtimeGpu,
      activation,
      n: null,
      active_cycle_snapshot: null,
      av_sync_drift_ms: null,
      audible_audio_pts_ms: null,
      audio_epoch: null,
      presented_pts_ms: activation === 'available' ? realtimeGpu.presented_pts_ms : null,
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
    { ...realtimeGpu, decoder: null },
    { ...realtimeGpu, decoder: 'unreported-original' },
    { ...realtimeGpu, graphics_api: 'vulkan', decoder: 'd3d11va' },
    { ...realtimeGpu, decoder: 'fake-decoder' },
    { ...realtimeGpu, cycle_drift_ms: Number.NaN },
    { ...realtimeGpu, av_sync_drift_ms: Number.NaN },
    { ...realtimeGpu, audible_audio_pts_ms: null },
    { ...realtimeGpu, audio_epoch: null },
    { ...realtimeGpu, audio_epoch: 0 },
    { ...realtimeGpu, activation: 'configured' },
    { ...realtimeGpu, gpu_pass_p99_ms: Number.NaN },
    { ...realtimeGpu, frame_budget_violation_windows: 4 },
    { ...realtimeGpu, demotion_history: [{ from: 'realtime_gpu', to: 'cpu4', fromMode: null, toMode: 'cpu4', reason: 'x', atUnixMs: 1 }] },
    { ...realtimeGpu, last_demotion: cpu4.last_demotion, demotion_history: [] },
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
