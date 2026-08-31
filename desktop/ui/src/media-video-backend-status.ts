import type {
  AdvancedEffectParams,
  VideoEffectParams,
} from './media-parameter-panels/media-parameter-types';

export type MediaVideoBackend = 'realtime_gpu' | 'cpu4' | 'source';

export type MediaVideoBackendActivation = 'active' | 'configured' | 'available' | 'failed';

export type MediaVideoApplyState =
  | 'idle'
  | 'source_transitioning'
  | 'ready'
  | 'applying'
  | 'result_unknown'
  | 'readback_confirmed'
  | 'presented_confirmed'
  | 'active'
  | 'failed';

export type MediaVideoRendererLifecycle =
  | 'unavailable'
  | 'probing'
  | 'spawned'
  | 'active'
  | 'failed'
  | 'stopped';

export type MediaVideoGraphicsApi = 'd3d11' | 'vulkan' | 'software';

export type MediaVideoCycleSlotStatus =
  | 'active'
  | 'ready'
  | 'preparing'
  | 'planned'
  | 'late'
  | 'failed';

export type MediaVideoSupportCompleteness = 'complete' | 'incomplete' | 'not_applicable';

export type MediaVideoParameterSupport = Readonly<{
  field: string;
  active: boolean;
  supported: boolean;
  mapping: string | null;
  reason: string | null;
}>;

export type MediaVideoParameterSupportReport = Readonly<{
  backend: MediaVideoBackend;
  fullySupported: boolean;
  parameters: readonly MediaVideoParameterSupport[];
}>;

export type MediaVideoCycleSlot = Readonly<{
  sequence: number;
  target_pts_ms: number;
  status: MediaVideoCycleSlotStatus;
}>;

export type MediaVideoBackendDemotion = Readonly<{
  from: MediaVideoBackend;
  to: MediaVideoBackend;
  fromMode: string;
  toMode: string;
  reason: string;
  atUnixMs: number;
}>;

export type MediaVideoEofFact = Readonly<{
  playback_generation: number;
  backend_epoch: number;
  clock_epoch: number;
  loop_index: number;
}>;

export type ActiveMediaVideoCycleSnapshot = Readonly<{
  sequence: number;
  fingerprint: string;
  video: VideoEffectParams;
  advanced: AdvancedEffectParams;
}>;

export type MediaVideoBackendStatus = Readonly<{
  status_revision: number;
  playback_generation: number | null;
  clock_epoch: number | null;
  loop_index: number | null;
  backend_epoch: number;
  fallback_floor_mode:
    | 'gpu_d3d11_zero_copy'
    | 'gpu_d3d11_copy'
    | 'gpu_vulkan_copy'
    | 'gpu_software_decode'
    | 'cpu4'
    | 'original';
  backend: MediaVideoBackend;
  activation: MediaVideoBackendActivation;
  lifecycle?: MediaVideoRendererLifecycle;
  gpu_adapter: string | null;
  graphics_api: MediaVideoGraphicsApi | null;
  decoder: string | null;
  filter: string | null;
  n: MediaVideoCycleSlot | null;
  n1: MediaVideoCycleSlot | null;
  n2: MediaVideoCycleSlot | null;
  apply_state: MediaVideoApplyState;
  active_plan_fingerprint: string | null;
  active_cycle_snapshot: ActiveMediaVideoCycleSnapshot | null;
  pending_plan_fingerprint: string | null;
  actual_source_fps: number | null;
  confirmed_change_count: number;
  cycle_drift_ms: number | null;
  av_sync_drift_ms: number | null;
  audible_audio_pts_ms: number | null;
  audio_epoch: number | null;
  demotion_reason: string | null;
  support_completeness: MediaVideoSupportCompleteness;
  process_id: number | null;
  parameter_support: MediaVideoParameterSupportReport;
  unsupported_parameter_count: number;
  ignored_active_parameter_count: number;
  ignored_active_parameter_examples: readonly string[];
  last_demotion: MediaVideoBackendDemotion | null;
  demotion_history: readonly MediaVideoBackendDemotion[];
  transition_started_at_unix_ms: number | null;
  transition_completed_at_unix_ms: number | null;
  resume_pts_ms: number | null;
  presented_pts_ms: number | null;
  gpu_pass_p99_ms: number | null;
  frame_drop_count: number | null;
  decoder_frame_drop_count: number | null;
  mistimed_frame_count: number | null;
  delayed_frame_count: number | null;
  frame_budget_violation_windows: number;
  physical_paused: boolean | null;
  eof: MediaVideoEofFact | null;
}>;

export type MediaVideoBackendIdentity = Readonly<{
  playback_generation: number;
  backend_epoch: number;
  clock_epoch: number;
  loop_index: number;
}>;

export type ManagedNativeVideoOwnership = Readonly<
  Pick<MediaVideoBackendIdentity, 'playback_generation' | 'clock_epoch' | 'loop_index'>
>;

export type MediaVideoBackendStatusView = Readonly<{
  valid: boolean;
  active: boolean;
  effects_applied: boolean;
  backend: MediaVideoBackend | null;
  label: string;
  detail: string;
}>;

export type MediaVideoBackendDiagnosticCode =
  | 'ipc_invoke_failed'
  | 'status_identity_rejected'
  | 'status_revision_rejected'
  | 'status_not_object'
  | 'status_legacy_contract'
  | 'status_missing_field'
  | 'status_invalid_field'
  | 'status_invariant_violation';

export type MediaVideoBackendDiagnostic = Readonly<{
  code: MediaVideoBackendDiagnosticCode;
  field: string | null;
  detail: string;
}>;

export type MediaVideoBackendStatusParseResult =
  | Readonly<{ ok: true; status: MediaVideoBackendStatus }>
  | Readonly<{ ok: false; error: MediaVideoBackendDiagnostic }>;

export type MediaVideoBackendStatusAcceptanceResult =
  | Readonly<{
    accepted: true;
    status: MediaVideoBackendStatus;
    lastObserved: MediaVideoBackendStatus;
    effective: MediaVideoBackendStatus | null;
  }>
  | Readonly<{
    accepted: false;
    status: MediaVideoBackendStatus | null;
    lastObserved: MediaVideoBackendStatus | null;
    effective: null;
    diagnostic: MediaVideoBackendDiagnostic;
  }>;

const BACKENDS = ['realtime_gpu', 'cpu4', 'source'] as const;
const ACTIVATIONS = ['active', 'configured', 'available', 'failed'] as const;
const APPLY_STATES = [
  'idle',
  'source_transitioning',
  'ready',
  'applying',
  'result_unknown',
  'readback_confirmed',
  'presented_confirmed',
  'active',
  'failed',
] as const;
const LIFECYCLES = ['unavailable', 'probing', 'spawned', 'active', 'failed', 'stopped'] as const;
const GRAPHICS_APIS = ['d3d11', 'vulkan', 'software'] as const;
const DECODERS = ['d3d11va', 'd3d11va-copy', 'software', 'unreported-original'] as const;
const SLOT_STATUSES = ['active', 'ready', 'preparing', 'planned', 'late', 'failed'] as const;
const SUPPORT_COMPLETENESS = ['complete', 'incomplete', 'not_applicable'] as const;
const CPU4_SUPPORTED_FIELDS = new Set([
  'video.brightness_percent',
  'video.contrast_percent',
  'video.saturation_percent',
  'video.hue_rotation_degrees',
]);
const MAX_RUNTIME_LABEL_LENGTH = 256;
const MAX_BACKEND_ERROR_LENGTH = 4_096;
const MAX_DIAGNOSTIC_DETAIL_LENGTH = 240;
const MAX_RUNTIME_PARAMETER_FIELDS = 256;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isEnumValue<T extends string>(values: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && values.includes(value as T);
}

function isRuntimeLabel(value: unknown): value is string | null {
  return value === null
    || (typeof value === 'string'
      && value.trim().length > 0
      && value.length <= MAX_RUNTIME_LABEL_LENGTH);
}

function isBackendError(value: unknown): value is string | null {
  return value === null
    || (typeof value === 'string'
      && value.trim().length > 0
      && value.length <= MAX_BACKEND_ERROR_LENGTH);
}

function isPlanFingerprint(value: unknown): value is string | null {
  return value === null
    || (typeof value === 'string'
      && value.trim().length > 0
      && value.length <= MAX_RUNTIME_LABEL_LENGTH);
}

function parseFailure(
  code: Exclude<MediaVideoBackendDiagnosticCode, 'ipc_invoke_failed'>,
  field: string | null,
  detail: string,
): MediaVideoBackendStatusParseResult {
  return { ok: false, error: { code, field, detail } };
}

function isCycleSlot(value: unknown): value is MediaVideoCycleSlot | null {
  if (value === null) return true;
  if (!isRecord(value)) return false;
  return Number.isSafeInteger(value.sequence)
    && (value.sequence as number) >= 1
    && typeof value.target_pts_ms === 'number'
    && Number.isFinite(value.target_pts_ms)
    && value.target_pts_ms >= 0
    && isEnumValue(SLOT_STATUSES, value.status);
}

function isRuntimeParameterSection(value: unknown): boolean {
  if (!isRecord(value) || Object.keys(value).length > MAX_RUNTIME_PARAMETER_FIELDS) return false;
  return Object.entries(value).every(([key, parameter]) => {
    if (key.length === 0 || key.length > MAX_RUNTIME_LABEL_LENGTH) return false;
    if (parameter === null || typeof parameter === 'boolean') return true;
    if (typeof parameter === 'number') return Number.isFinite(parameter);
    if (typeof parameter === 'string') return parameter.length <= MAX_RUNTIME_LABEL_LENGTH;
    if (!isRecord(parameter) || Object.keys(parameter).length > MAX_RUNTIME_PARAMETER_FIELDS) return false;
    return Object.entries(parameter).every(([nestedKey, nestedValue]) => (
      nestedKey.length > 0
      && nestedKey.length <= MAX_RUNTIME_LABEL_LENGTH
      && typeof nestedValue === 'number'
      && Number.isFinite(nestedValue)
    ));
  });
}

function isActiveCycleSnapshot(value: unknown): value is ActiveMediaVideoCycleSnapshot | null {
  if (value === null) return true;
  return isRecord(value)
    && Number.isSafeInteger(value.sequence)
    && (value.sequence as number) >= 1
    && typeof value.fingerprint === 'string'
    && isPlanFingerprint(value.fingerprint)
    && isRuntimeParameterSection(value.video)
    && isRuntimeParameterSection(value.advanced);
}

function isVideoEofFact(value: unknown): value is MediaVideoEofFact {
  if (!isRecord(value)) return false;
  return Number.isSafeInteger(value.playback_generation)
    && (value.playback_generation as number) >= 1
    && Number.isSafeInteger(value.backend_epoch)
    && (value.backend_epoch as number) >= 1
    && Number.isSafeInteger(value.clock_epoch)
    && (value.clock_epoch as number) >= 1
    && Number.isSafeInteger(value.loop_index)
    && (value.loop_index as number) >= 0;
}

function isBackendDemotion(value: unknown): value is MediaVideoBackendDemotion {
  if (!isRecord(value)) return false;
  return isEnumValue(BACKENDS, value.from)
    && isEnumValue(BACKENDS, value.to)
    && typeof value.fromMode === 'string'
    && isRuntimeLabel(value.fromMode)
    && typeof value.toMode === 'string'
    && isRuntimeLabel(value.toMode)
    && typeof value.reason === 'string'
    && isBackendError(value.reason)
    && Number.isSafeInteger(value.atUnixMs)
    && (value.atUnixMs as number) >= 0;
}

function hasOwn(record: Record<string, unknown>, field: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, field);
}

function isObservedDecoderForGraphicsApi(graphicsApi: unknown, decoder: unknown): boolean {
  return decoder === 'software'
    || (graphicsApi === 'd3d11' && (decoder === 'd3d11va' || decoder === 'd3d11va-copy'))
    || (graphicsApi === 'vulkan' && decoder === 'd3d11va-copy');
}

function isParameterSupportReport(value: unknown): value is MediaVideoParameterSupportReport {
  if (!isRecord(value)
    || !isEnumValue(BACKENDS, value.backend)
    || typeof value.fullySupported !== 'boolean'
    || !Array.isArray(value.parameters)
    || value.parameters.length > 512) return false;
  return value.parameters.every((parameter) => isRecord(parameter)
    && typeof parameter.field === 'string'
    && parameter.field.length > 0
    && parameter.field.length <= MAX_RUNTIME_LABEL_LENGTH
    && typeof parameter.active === 'boolean'
    && typeof parameter.supported === 'boolean'
    && isRuntimeLabel(parameter.mapping)
    && isRuntimeLabel(parameter.reason));
}

export function parseMediaVideoBackendStatusResult(
  value: unknown,
): MediaVideoBackendStatusParseResult {
  if (!isRecord(value)) {
    return parseFailure('status_not_object', null, '响应必须是对象');
  }
  if (hasOwn(value, 'encoder')) {
    return parseFailure('status_legacy_contract', 'encoder', '响应仍包含已删除的编码器字段');
  }
  const requiredFields = [
    'status_revision',
    'playback_generation',
    'clock_epoch',
    'loop_index',
    'backend_epoch',
    'fallback_floor_mode',
    'backend',
    'activation',
    'gpu_adapter',
    'graphics_api',
    'decoder',
    'filter',
    'n',
    'n1',
    'n2',
    'apply_state',
    'active_plan_fingerprint',
    'active_cycle_snapshot',
    'pending_plan_fingerprint',
    'actual_source_fps',
    'confirmed_change_count',
    'cycle_drift_ms',
    'av_sync_drift_ms',
    'audible_audio_pts_ms',
    'audio_epoch',
    'demotion_reason',
    'support_completeness',
    'process_id',
    'parameter_support',
    'unsupported_parameter_count',
    'ignored_active_parameter_count',
    'ignored_active_parameter_examples',
    'last_demotion',
    'demotion_history',
    'transition_started_at_unix_ms',
    'transition_completed_at_unix_ms',
    'resume_pts_ms',
    'presented_pts_ms',
    'gpu_pass_p99_ms',
    'frame_drop_count',
    'decoder_frame_drop_count',
    'mistimed_frame_count',
    'delayed_frame_count',
    'frame_budget_violation_windows',
    'physical_paused',
    'eof',
  ];
  const missingField = requiredFields.find((field) => !hasOwn(value, field));
  if (missingField) {
    return parseFailure('status_missing_field', missingField, '响应缺少必填字段');
  }
  if (!isEnumValue(BACKENDS, value.backend)) {
    return parseFailure('status_invalid_field', 'backend', '后端类型不在允许范围内');
  }
  if (!isEnumValue(ACTIVATIONS, value.activation)) {
    return parseFailure('status_invalid_field', 'activation', '激活状态不在允许范围内');
  }
  if (!isEnumValue(APPLY_STATES, value.apply_state)) {
    return parseFailure('status_invalid_field', 'apply_state', '提交确认状态不在允许范围内');
  }
  if (!Number.isSafeInteger(value.status_revision) || (value.status_revision as number) < 1) {
    return parseFailure('status_invalid_field', 'status_revision', '状态修订号必须是正安全整数');
  }
  if (!Number.isSafeInteger(value.confirmed_change_count)
    || (value.confirmed_change_count as number) < 0) {
    return parseFailure('status_invalid_field', 'confirmed_change_count', '已确认变化次数必须是非负安全整数');
  }
  if (!isPlanFingerprint(value.active_plan_fingerprint)
    || !isPlanFingerprint(value.pending_plan_fingerprint)) {
    return parseFailure('status_invalid_field', 'active_plan_fingerprint', '计划指纹为空或过长');
  }
  if (!isActiveCycleSnapshot(value.active_cycle_snapshot)) {
    return parseFailure('status_invalid_field', 'active_cycle_snapshot', '当前周期参数快照结构无效');
  }
  if (value.actual_source_fps !== null
    && (typeof value.actual_source_fps !== 'number'
      || !Number.isFinite(value.actual_source_fps)
      || value.actual_source_fps <= 0)) {
    return parseFailure('status_invalid_field', 'actual_source_fps', '源帧率必须是正有限数值');
  }
  if (hasOwn(value, 'lifecycle') && !isEnumValue(LIFECYCLES, value.lifecycle)) {
    return parseFailure('status_invalid_field', 'lifecycle', '渲染器生命周期不在允许范围内');
  }
  if (!isRuntimeLabel(value.gpu_adapter)) {
    return parseFailure('status_invalid_field', 'gpu_adapter', 'GPU 标识为空或过长');
  }
  if (value.graphics_api !== null && !isEnumValue(GRAPHICS_APIS, value.graphics_api)) {
    return parseFailure('status_invalid_field', 'graphics_api', '图形 API 不在允许范围内');
  }
  if ((value.decoder !== null && !isEnumValue(DECODERS, value.decoder))
    || !isRuntimeLabel(value.filter)) {
    return parseFailure(
      'status_invalid_field',
      value.decoder !== null && !isEnumValue(DECODERS, value.decoder) ? 'decoder' : 'filter',
      '解码器或滤镜标识无效',
    );
  }
  for (const field of ['n', 'n1', 'n2'] as const) {
    if (!isCycleSlot(value[field])) {
      return parseFailure('status_invalid_field', field, '周期槽位结构无效');
    }
  }
  const activeSlot = value.n as MediaVideoCycleSlot | null;
  if (value.cycle_drift_ms !== null
    && (typeof value.cycle_drift_ms !== 'number' || !Number.isFinite(value.cycle_drift_ms))) {
    return parseFailure('status_invalid_field', 'cycle_drift_ms', '周期漂移必须是有限数值');
  }
  if (value.av_sync_drift_ms !== null
    && (!Number.isSafeInteger(value.av_sync_drift_ms))) {
    return parseFailure('status_invalid_field', 'av_sync_drift_ms', '音画漂移必须是安全整数');
  }
  for (const field of ['audible_audio_pts_ms', 'audio_epoch']) {
    const fieldValue = value[field];
    if (fieldValue !== null
      && (!Number.isSafeInteger(fieldValue) || (fieldValue as number) < 0)) {
      return parseFailure('status_invalid_field', field, '音频时钟字段必须是非负安全整数');
    }
  }
  if (value.audio_epoch === 0) {
    return parseFailure('status_invalid_field', 'audio_epoch', 'audio_epoch 必须大于 0');
  }
  const hasAudioClock = value.audible_audio_pts_ms !== null || value.audio_epoch !== null;
  if (hasAudioClock !== (value.audible_audio_pts_ms !== null && value.audio_epoch !== null)
    || (value.av_sync_drift_ms !== null) !== hasAudioClock
    || (hasAudioClock && (value.activation !== 'active' || value.process_id === null))) {
    return parseFailure('status_invariant_violation', 'audio_epoch', '音频时钟字段必须成组出现且只属于活动进程');
  }
  if (!isBackendError(value.demotion_reason)) {
    return parseFailure('status_invalid_field', 'demotion_reason', '降级原因为空或超过安全上限');
  }
  if (!isEnumValue(SUPPORT_COMPLETENESS, value.support_completeness)) {
    return parseFailure('status_invalid_field', 'support_completeness', '支持完整度不在允许范围内');
  }
  if (value.process_id !== null
    && (!Number.isSafeInteger(value.process_id) || (value.process_id as number) <= 0)) {
    return parseFailure('status_invalid_field', 'process_id', '进程号必须是正安全整数');
  }
  if (!isParameterSupportReport(value.parameter_support)) {
    return parseFailure('status_invalid_field', 'parameter_support', '参数支持报告结构无效');
  }
  if (value.last_demotion !== null && !isBackendDemotion(value.last_demotion)) {
    return parseFailure('status_invalid_field', 'last_demotion', '最近降级记录结构无效');
  }
  if (!Array.isArray(value.demotion_history)
    || value.demotion_history.length > 5
    || !value.demotion_history.every(isBackendDemotion)) {
    return parseFailure('status_invalid_field', 'demotion_history', '降级历史结构无效或超过五条');
  }
  const lastHistoryEntry = value.demotion_history.length > 0
    ? value.demotion_history[value.demotion_history.length - 1]
    : null;
  if (JSON.stringify(lastHistoryEntry) !== JSON.stringify(value.last_demotion)) {
    return parseFailure('status_invariant_violation', 'last_demotion', '最近降级记录与历史末项不一致');
  }
  if (value.playback_generation !== null
    && (!Number.isSafeInteger(value.playback_generation) || (value.playback_generation as number) < 1)) {
    return parseFailure('status_invalid_field', 'playback_generation', '播放代次必须是正安全整数');
  }
  if (value.clock_epoch !== null
    && (!Number.isSafeInteger(value.clock_epoch) || (value.clock_epoch as number) < 1)) {
    return parseFailure('status_invalid_field', 'clock_epoch', '时钟 epoch 必须是正安全整数');
  }
  if (value.loop_index !== null
    && (!Number.isSafeInteger(value.loop_index) || (value.loop_index as number) < 0)) {
    return parseFailure('status_invalid_field', 'loop_index', '循环序号必须是非负安全整数');
  }
  if (!Number.isSafeInteger(value.backend_epoch) || (value.backend_epoch as number) < 0) {
    return parseFailure('status_invalid_field', 'backend_epoch', '后端 epoch 必须是非负安全整数');
  }
  if (!isEnumValue([
    'gpu_d3d11_zero_copy',
    'gpu_d3d11_copy',
    'gpu_vulkan_copy',
    'gpu_software_decode',
    'cpu4',
    'original',
  ] as const, value.fallback_floor_mode)) {
    return parseFailure('status_invalid_field', 'fallback_floor_mode', '降级下限模式不在允许范围内');
  }
  const unsupportedCount = value.parameter_support.parameters.filter((parameter) => !parameter.supported).length;
  const ignoredActive = value.parameter_support.parameters
    .filter((parameter) => parameter.active && !parameter.supported);
  if (!Number.isSafeInteger(value.unsupported_parameter_count)
    || value.unsupported_parameter_count !== unsupportedCount
    || !Number.isSafeInteger(value.ignored_active_parameter_count)
    || value.ignored_active_parameter_count !== ignoredActive.length
    || !Array.isArray(value.ignored_active_parameter_examples)
    || value.ignored_active_parameter_examples.length > 3
    || value.ignored_active_parameter_examples.some((field) => typeof field !== 'string')
    || value.ignored_active_parameter_examples.some((field, index) => field !== ignoredActive[index]?.field)) {
    return parseFailure('status_invariant_violation', 'unsupported_parameter_count', '参数支持计数或示例与报告不一致');
  }
  for (const field of [
    'transition_started_at_unix_ms',
    'transition_completed_at_unix_ms',
    'resume_pts_ms',
    'presented_pts_ms',
  ]) {
    const fieldValue = value[field];
    if (fieldValue !== null
      && (!Number.isSafeInteger(fieldValue) || (fieldValue as number) < 0)) {
      return parseFailure('status_invalid_field', field, '转换时间或恢复 PTS 必须是非负安全整数');
    }
  }
  if (value.gpu_pass_p99_ms !== null
    && (typeof value.gpu_pass_p99_ms !== 'number'
      || !Number.isFinite(value.gpu_pass_p99_ms)
      || value.gpu_pass_p99_ms < 0)) {
    return parseFailure('status_invalid_field', 'gpu_pass_p99_ms', 'GPU P99 必须是非负有限数值');
  }
  if (value.presented_pts_ms !== null
    && (value.process_id === null
      || value.playback_generation === null
      || value.clock_epoch === null
      || value.loop_index === null
      || (value.activation !== 'active' && value.activation !== 'available'))) {
    return parseFailure('status_invariant_violation', 'presented_pts_ms', '视频 PTS 只能属于已绑定的可用或活动 mpv 进程');
  }
  for (const field of [
    'frame_drop_count',
    'decoder_frame_drop_count',
    'mistimed_frame_count',
    'delayed_frame_count',
  ]) {
    const fieldValue = value[field];
    if (fieldValue !== null
      && (!Number.isSafeInteger(fieldValue) || (fieldValue as number) < 0)) {
      return parseFailure('status_invalid_field', field, '帧计数必须是非负安全整数');
    }
  }
  if (!Number.isSafeInteger(value.frame_budget_violation_windows)
    || (value.frame_budget_violation_windows as number) < 0
    || (value.frame_budget_violation_windows as number) > 3) {
    return parseFailure('status_invalid_field', 'frame_budget_violation_windows', '帧预算连续异常窗口必须在 0 到 3 之间');
  }
  if (value.physical_paused !== null && typeof value.physical_paused !== 'boolean') {
    return parseFailure('status_invalid_field', 'physical_paused', '物理暂停状态必须是布尔值或 null');
  }
  if ((value.process_id === null) !== (value.physical_paused === null)) {
    return parseFailure('status_invariant_violation', 'physical_paused', '物理暂停状态必须与受管 mpv 进程同时存在');
  }
  if (value.eof !== null && !isVideoEofFact(value.eof)) {
    return parseFailure('status_invalid_field', 'eof', 'EOF 事实结构无效');
  }
  if (value.parameter_support.backend !== value.backend) {
    return parseFailure('status_invariant_violation', 'parameter_support.backend', '参数支持报告后端与运行后端不一致');
  }
  if (value.support_completeness !== 'not_applicable'
    && (value.support_completeness === 'complete') !== value.parameter_support.fullySupported) {
    return parseFailure('status_invariant_violation', 'support_completeness', '支持完整度与参数报告不一致');
  }

  if (value.activation === 'active' && value.backend !== 'source') {
    if (activeSlot?.status !== 'active') {
      return parseFailure('status_invariant_violation', 'n', '活动效果后端必须具有活动 N 槽位');
    }
    if (value.backend === 'realtime_gpu') {
      if (value.support_completeness === 'not_applicable'
        || (value.graphics_api !== 'd3d11' && value.graphics_api !== 'vulkan')
        || !isObservedDecoderForGraphicsApi(value.graphics_api, value.decoder)) {
        return parseFailure('status_invariant_violation', 'graphics_api', 'GPU 活动状态的图形 API 或解码器组合无效');
      }
    }
  }
  if (value.activation !== 'active' && activeSlot !== null) {
    return parseFailure('status_invariant_violation', 'activation', '存在活动 N 槽位时渲染器必须保持 active');
  }
  if ((value.backend === 'realtime_gpu' || value.backend === 'cpu4')
    && (value.activation === 'available' || value.activation === 'active')
    && value.process_id === null) {
    return parseFailure('status_invariant_violation', 'process_id', '可用或活动效果后端必须具有进程号');
  }
  if (value.backend === 'cpu4') {
    const fields = value.parameter_support.parameters.map((parameter) => parameter.field);
    const supportedFields = value.parameter_support.parameters
      .filter((parameter) => parameter.supported)
      .map((parameter) => parameter.field);
    if (fields.length !== 83
      || new Set(fields).size !== 83
      || supportedFields.length !== CPU4_SUPPORTED_FIELDS.size
      || supportedFields.some((field) => !CPU4_SUPPORTED_FIELDS.has(field))
      || value.unsupported_parameter_count !== 79) {
      return parseFailure('status_invariant_violation', 'parameter_support', 'CPU4 必须精确报告 83 项中的 4 项支持');
    }
    if ((value.activation === 'available' || value.activation === 'active')
      && (value.graphics_api !== 'd3d11'
        || value.decoder !== 'software'
        || value.filter !== 'libavfilter eq+hue'
        || value.gpu_adapter !== null)) {
      return parseFailure('status_invariant_violation', 'filter', 'CPU4 活动管线必须是 D3D11 软件解码与 eq+hue');
    }
  }
  if (value.activation === 'active' && value.backend === 'source') {
    const managedOriginal = value.process_id !== null;
    if (value.support_completeness !== 'not_applicable'
      || value.gpu_adapter !== null
      || value.n !== null
      || value.n1 !== null
      || value.n2 !== null) {
      return parseFailure('status_invariant_violation', 'backend', 'Original 不得携带效果周期槽位或 GPU 适配器');
    }
    if (managedOriginal) {
      const gpuNeutralPipelineMatches = value.filter === 'Original（中性 shader）'
        && ((value.fallback_floor_mode === 'gpu_d3d11_zero_copy'
          && value.graphics_api === 'd3d11'
          && value.decoder === 'd3d11va')
          || (value.fallback_floor_mode === 'gpu_d3d11_copy'
            && value.graphics_api === 'd3d11'
            && value.decoder === 'd3d11va-copy')
          || (value.fallback_floor_mode === 'gpu_vulkan_copy'
            && value.graphics_api === 'vulkan'
            && value.decoder === 'd3d11va-copy')
          || (value.fallback_floor_mode === 'gpu_software_decode'
            && value.graphics_api === 'd3d11'
            && value.decoder === 'software'));
      const neutralFilterMatchesLaunchMode = value.filter === 'Original（无 shader）'
        || gpuNeutralPipelineMatches
        || (value.filter === 'Original（CPU4 中性参数）'
          && value.fallback_floor_mode === 'cpu4'
          && value.graphics_api === 'd3d11'
          && value.decoder === 'software');
      if ((value.graphics_api !== 'd3d11' && value.graphics_api !== 'vulkan')
        || (value.decoder !== 'unreported-original'
          && !isObservedDecoderForGraphicsApi(value.graphics_api, value.decoder))
        || !neutralFilterMatchesLaunchMode) {
        return parseFailure('status_invariant_violation', 'filter', '受管 Original 管线信息无效');
      }
    } else if (value.graphics_api !== null || value.decoder !== null || value.filter !== null) {
      return parseFailure('status_invariant_violation', 'process_id', '无进程 Original 不得声明图形管线');
    }
  }
  if (value.eof !== null) {
    if ((value.activation !== 'active' && value.activation !== 'available')
      || value.process_id === null
      || value.physical_paused !== true
      || value.eof.playback_generation !== value.playback_generation
      || value.eof.backend_epoch !== value.backend_epoch
      || value.eof.clock_epoch !== value.clock_epoch
      || value.eof.loop_index !== value.loop_index) {
      return parseFailure('status_invariant_violation', 'eof', 'EOF 事实与当前后端身份不一致');
    }
  }
  const activeCycleSlot = value.n as MediaVideoCycleSlot | null;
  const activeCycleSnapshot = value.active_cycle_snapshot as ActiveMediaVideoCycleSnapshot | null;
  const activeCycleExpected = (value.backend === 'realtime_gpu' || value.backend === 'cpu4')
    && value.activation === 'active'
    && value.apply_state === 'active'
    && activeCycleSlot?.status === 'active'
    && value.active_plan_fingerprint !== null;
  if (activeCycleExpected !== (activeCycleSnapshot !== null)) {
    return parseFailure('status_invariant_violation', 'active_cycle_snapshot', '当前周期参数快照与物理确认状态不一致');
  }
  if (activeCycleSnapshot !== null
    && (activeCycleSnapshot.sequence !== activeCycleSlot?.sequence
      || activeCycleSnapshot.fingerprint !== value.active_plan_fingerprint)) {
    return parseFailure('status_invariant_violation', 'active_cycle_snapshot', '当前周期参数快照与 Active N 身份不一致');
  }

  return { ok: true, status: value as MediaVideoBackendStatus };
}

export function parseMediaVideoBackendStatus(value: unknown): MediaVideoBackendStatus | null {
  const result = parseMediaVideoBackendStatusResult(value);
  return result.ok ? result.status : null;
}

export function canUseRealtimeVideoBackend(
  status: MediaVideoBackendStatus | null,
): boolean {
  return status?.backend === 'realtime_gpu'
    && (status.activation === 'available' || status.activation === 'active')
    && status.support_completeness === 'complete'
    && status.parameter_support.fullySupported;
}

export function canPrepareManagedVideoBackend(
  status: MediaVideoBackendStatus | null,
): boolean {
  return canUseRealtimeVideoBackend(status)
    || (status?.backend === 'cpu4'
      && (status.activation === 'available' || status.activation === 'active')
      && status.process_id !== null);
}

export function isTerminalMediaVideoBackendStatus(
  status: MediaVideoBackendStatus | null,
): boolean {
  return status?.activation === 'failed'
    || status?.lifecycle === 'failed'
    || status?.lifecycle === 'stopped'
    || status?.lifecycle === 'unavailable'
    || (status?.backend === 'source' && status.activation === 'active');
}

export function mediaVideoBackendIdentity(
  status: MediaVideoBackendStatus | null,
): MediaVideoBackendIdentity | null {
  if (status === null
    || status.playback_generation === null
    || status.clock_epoch === null
    || status.loop_index === null) return null;
  return {
    playback_generation: status.playback_generation,
    backend_epoch: status.backend_epoch,
    clock_epoch: status.clock_epoch,
    loop_index: status.loop_index,
  };
}

export function canUseManagedNativeVideo(
  status: MediaVideoBackendStatus | null,
  expected: Pick<MediaVideoBackendIdentity, 'playback_generation' | 'clock_epoch' | 'loop_index'>,
): boolean {
  return (status?.activation === 'active' || status?.activation === 'available')
    && status.process_id !== null
    && status.presented_pts_ms !== null
    && status.playback_generation === expected.playback_generation
    && status.clock_epoch === expected.clock_epoch
    && status.loop_index === expected.loop_index;
}

export function shouldEnsureOriginalVideoRenderer(
  videoProcessingEnabled: boolean,
  status: MediaVideoBackendStatus | null,
  playbackGeneration: number,
): boolean {
  if (!videoProcessingEnabled) return true;
  return !(
    status?.playback_generation === playbackGeneration
    && (status.activation === 'active' || status.activation === 'available')
    && status.lifecycle !== 'failed'
    && status.lifecycle !== 'stopped'
    && status.apply_state !== 'failed'
    && status.process_id !== null
    // 首周期确认前逻辑 backend 仍是中性 Source；物理启动模式才决定
    // 该会话是否已经由 Rust GPU/CPU4 周期链承载。
    && status.fallback_floor_mode !== 'original'
  );
}

export function canKeepManagedNativeVideoDuringLoopTransition(
  status: MediaVideoBackendStatus | null,
  expected: Pick<MediaVideoBackendIdentity, 'playback_generation' | 'clock_epoch' | 'loop_index'>,
): boolean {
  const eof = status?.eof;
  if (!eof) return false;
  return status?.activation === 'active'
    && status.process_id !== null
    && status.playback_generation === expected.playback_generation
    && status.clock_epoch === expected.clock_epoch
    && status.loop_index === eof.loop_index
    && eof.playback_generation === expected.playback_generation
    && eof.backend_epoch === status.backend_epoch
    && eof.clock_epoch === expected.clock_epoch
    && eof.loop_index < Number.MAX_SAFE_INTEGER
    && expected.loop_index === eof.loop_index + 1;
}

function mediaVideoBackendMatchesExpected(
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership,
): boolean {
  return status?.playback_generation === expected.playback_generation
    && status.clock_epoch === expected.clock_epoch
    && status.loop_index === expected.loop_index;
}

function explicitlyReleasedManagedNativeVideo(
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership,
): boolean {
  if (status === null || !mediaVideoBackendMatchesExpected(status, expected)) return false;
  return status.activation === 'failed'
    || status.lifecycle === 'failed'
    || status.lifecycle === 'stopped'
    || status.lifecycle === 'unavailable'
    || (status.backend === 'source'
      && status.activation === 'active'
      && status.process_id === null);
}

function explicitlyReleasedCurrentOrPreviousManagedNativeVideo(
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership,
): boolean {
  if (status === null) return false;
  const statusMatchesExpectedOrPrevious = status.playback_generation === expected.playback_generation
    && status.clock_epoch === expected.clock_epoch
    && (status.loop_index === expected.loop_index
      || (status.loop_index !== null
        && status.loop_index < Number.MAX_SAFE_INTEGER
        && status.loop_index + 1 === expected.loop_index));
  return statusMatchesExpectedOrPrevious
    && explicitlyReleasedManagedNativeVideo(status, {
      playback_generation: status.playback_generation,
      clock_epoch: status.clock_epoch,
      loop_index: status.loop_index,
    });
}

export function resolveManagedNativeVideoOwnership(
  current: ManagedNativeVideoOwnership | null,
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership | null,
  playbackCanOwnVideo: boolean,
): ManagedNativeVideoOwnership | null {
  if (!playbackCanOwnVideo || expected === null) return null;
  if (explicitlyReleasedCurrentOrPreviousManagedNativeVideo(status, expected)
    || (current !== null && explicitlyReleasedManagedNativeVideo(status, current))) return null;
  if (canUseManagedNativeVideo(status, expected)
    || canKeepManagedNativeVideoDuringLoopTransition(status, expected)) {
    return { ...expected };
  }
  if (current === null) return null;
  if (
    current.playback_generation === expected.playback_generation
    && current.clock_epoch === expected.clock_epoch
    && (expected.loop_index === current.loop_index
      || current.loop_index < Number.MAX_SAFE_INTEGER
        && expected.loop_index === current.loop_index + 1)
  ) return { ...expected };
  const eof = status?.eof;
  if (
    eof !== null
    && eof !== undefined
    && eof.playback_generation === current.playback_generation
    && eof.clock_epoch === current.clock_epoch
    && eof.loop_index === current.loop_index
    && expected.playback_generation === current.playback_generation + 1
  ) return { ...expected };
  return null;
}

export function isCurrentMediaVideoCyclePresented(
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership,
  sequence?: number,
): boolean {
  return mediaVideoBackendMatchesExpected(status, expected)
    && status?.activation === 'active'
    && status.apply_state === 'active'
    && status.active_plan_fingerprint !== null
    && (status.backend === 'realtime_gpu' || status.backend === 'cpu4')
    && status.n?.status === 'active'
    && (sequence === undefined || status.n.sequence === sequence);
}

export function isCurrentMediaVideoCycleFailed(
  status: MediaVideoBackendStatus | null,
  expected: ManagedNativeVideoOwnership,
): boolean {
  return mediaVideoBackendMatchesExpected(status, expected)
    && status?.activation === 'failed';
}

function formatBackendIdentity(identity: MediaVideoBackendIdentity): string {
  return `${identity.playback_generation}/${identity.backend_epoch}/${identity.clock_epoch}/${identity.loop_index}`;
}

function rejectMediaVideoBackendIdentity(
  current: MediaVideoBackendStatus | null,
  incoming: MediaVideoBackendStatus | null,
  detail: string,
): MediaVideoBackendStatusAcceptanceResult {
  return {
    accepted: false,
    status: current,
    lastObserved: incoming,
    effective: null,
    diagnostic: {
      code: 'status_identity_rejected',
      field: 'identity',
      detail,
    },
  };
}

export function acceptMediaVideoBackendStatusResult(
  current: MediaVideoBackendStatus | null,
  incoming: MediaVideoBackendStatus | null,
  expectedIdentity: MediaVideoBackendIdentity | null,
): MediaVideoBackendStatusAcceptanceResult {
  if (incoming === null) {
    return rejectMediaVideoBackendIdentity(current, incoming, '后端状态为空，已保留最后一次有效身份');
  }
  const incomingIdentity = mediaVideoBackendIdentity(incoming);
  if (incomingIdentity === null) {
    return rejectMediaVideoBackendIdentity(current, incoming, '后端状态未携带完整播放身份，已保留最后一次有效身份');
  }
  // EOF 推进由 Rust 独占。前端本地 clock/loop 只能用于显示，不能屏蔽后端 EOF 事实。
  if (expectedIdentity !== null
    && incomingIdentity.playback_generation !== expectedIdentity.playback_generation) {
    return rejectMediaVideoBackendIdentity(
      current,
      incoming,
      `状态身份 ${formatBackendIdentity(incomingIdentity)} 不属于当前播放代次 ${expectedIdentity.playback_generation}`,
    );
  }
  if (current !== null && incoming.status_revision < current.status_revision) {
    return {
      accepted: false,
      status: current,
      lastObserved: incoming,
      effective: null,
      diagnostic: {
        code: 'status_revision_rejected',
        field: 'status_revision',
        detail: `状态修订 ${incoming.status_revision} 落后于已接收修订 ${current.status_revision}`,
      },
    };
  }
  if (current !== null && incoming.status_revision === current.status_revision) {
    if (JSON.stringify(incoming) !== JSON.stringify(current)) {
      return {
        accepted: false,
        status: current,
        lastObserved: incoming,
        effective: null,
        diagnostic: {
          code: 'status_revision_rejected',
          field: 'status_revision',
          detail: `状态修订 ${incoming.status_revision} 内容冲突，已拒绝覆盖同修订状态`,
        },
      };
    }
    return {
      accepted: true,
      status: current,
      lastObserved: current,
      effective: effectiveMediaVideoBackendStatus(
        current,
        expectedIdentity?.playback_generation ?? null,
      ),
    };
  }
  return {
    accepted: true,
    status: incoming,
    lastObserved: incoming,
    effective: effectiveMediaVideoBackendStatus(incoming, expectedIdentity?.playback_generation ?? null),
  };
}

export function effectiveMediaVideoBackendStatus(
  status: MediaVideoBackendStatus | null,
  expectedPlaybackGeneration: number | null = null,
): MediaVideoBackendStatus | null {
  if (status === null
    || (expectedPlaybackGeneration !== null
      && status.playback_generation !== expectedPlaybackGeneration)
    || status.activation !== 'active'
    || (status.apply_state !== 'idle' && status.apply_state !== 'active')) return null;
  if (status.backend === 'source') return status;
  return status.apply_state === 'active'
    && status.active_plan_fingerprint !== null
    && status.n?.status === 'active'
    ? status
    : null;
}

export function mediaVideoPresentationPtsMs(
  status: MediaVideoBackendStatus | null,
  sourceDurationMs: number | null | undefined,
): number | null {
  if (status?.presented_pts_ms === null
    || status?.presented_pts_ms === undefined
    || status.loop_index === null
    || !Number.isSafeInteger(sourceDurationMs)
    || (sourceDurationMs as number) <= 0) return null;
  const presentationPtsMs = status.loop_index * (sourceDurationMs as number)
    + status.presented_pts_ms;
  return Number.isSafeInteger(presentationPtsMs) ? presentationPtsMs : null;
}

function backendLabel(backend: MediaVideoBackend): string {
  switch (backend) {
    case 'realtime_gpu': return '实时 GPU';
    case 'cpu4': return 'CPU4 实时回退';
    case 'source': return '源画面兜底';
  }
}

function slotLabel(name: string, slot: MediaVideoCycleSlot | null): string | null {
  return slot === null ? null : `${name} ${slot.sequence}/${slot.status}@${slot.target_pts_ms}ms`;
}

function sanitizeDiagnosticDetail(value: string): string {
  const redacted = value
    .replace(/\b[A-Za-z]:\\[^\s"']+/g, '[本地路径]')
    .replace(/\b(?:authorization|api[_ -]?key|token|secret|password)\s*[:=]\s*[^\s,;]+/gi, '[敏感信息]')
    .replace(/https?:\/\/[^\s/@]+@/gi, 'https://[凭据已隐藏]@')
    .replace(/[\r\n\t]+/g, ' ')
    .trim();
  return redacted.length <= MAX_DIAGNOSTIC_DETAIL_LENGTH
    ? redacted
    : `${redacted.slice(0, MAX_DIAGNOSTIC_DETAIL_LENGTH - 1)}…`;
}

export function createMediaVideoBackendIpcDiagnostic(detail: string): MediaVideoBackendDiagnostic {
  return { code: 'ipc_invoke_failed', field: null, detail };
}

export function formatMediaVideoBackendDiagnostic(
  diagnostic: MediaVideoBackendDiagnostic,
): string {
  const category = diagnostic.code === 'ipc_invoke_failed'
    ? 'IPC 调用失败'
    : diagnostic.code === 'status_identity_rejected'
      ? '状态身份拒绝'
      : diagnostic.code === 'status_revision_rejected'
        ? '状态修订拒绝'
      : 'DTO 校验失败';
  const field = diagnostic.field === null ? '' : `（${diagnostic.field}）`;
  return `${category}${field}：${sanitizeDiagnosticDetail(diagnostic.detail) || '未提供详细原因'}`;
}

export function projectMediaVideoBackendStatus(
  value: unknown,
  diagnostic?: MediaVideoBackendDiagnostic | null,
): MediaVideoBackendStatusView {
  if (value === null && !diagnostic) {
    return {
      valid: true,
      active: false,
      effects_applied: false,
      backend: null,
      label: '视频后端状态读取中',
      detail: '等待 Rust 运行时上报视频后端状态',
    };
  }
  if (diagnostic) {
    return {
      valid: false,
      active: false,
      effects_applied: false,
      backend: null,
      label: diagnostic.code === 'status_identity_rejected'
        || diagnostic.code === 'status_revision_rejected'
        ? '视频后端状态待确认'
        : '视频后端状态不可用',
      detail: formatMediaVideoBackendDiagnostic(diagnostic),
    };
  }
  const parsed = parseMediaVideoBackendStatusResult(value);
  if (!parsed.ok) {
    return {
      valid: false,
      active: false,
      effects_applied: false,
      backend: null,
      label: '视频后端状态不可用',
      detail: formatMediaVideoBackendDiagnostic(parsed.error),
    };
  }
  const status = parsed.status;
  const effective = effectiveMediaVideoBackendStatus(status);
  const active = effective !== null;
  const skippedParameters = status.parameter_support.parameters
    .filter((parameter) => parameter.active && !parameter.supported);
  const effectsApplied = effective !== null && (
    (status.backend === 'realtime_gpu'
      && status.support_completeness === 'complete'
      && status.parameter_support.fullySupported
      && status.parameter_support.parameters.some((parameter) => parameter.active && parameter.supported))
    || (status.backend !== 'realtime_gpu'
      && status.backend !== 'source'
      && status.parameter_support.parameters.some((parameter) => parameter.active && parameter.supported))
  );
  const activationLabel = {
    active: '已生效',
    configured: '已配置，尚未生效',
    available: '可用，尚未生效',
    failed: '运行失败',
  }[status.activation];
  const applyStateLabel = {
    idle: '尚未提交',
    source_transitioning: '换源中，等待新物理时钟',
    ready: '计划就绪，待提交',
    applying: '正在提交，待确认',
    result_unknown: '提交结果未知，待读回确认',
    readback_confirmed: '参数已读回，待画面呈现确认',
    presented_confirmed: '画面已呈现，待状态晋级',
    active: '提交已确认',
    failed: '提交失败',
  }[status.apply_state];
  const pipeline = [
    status.gpu_adapter,
    status.graphics_api?.toUpperCase(),
    status.decoder && `解码 ${status.decoder === 'unreported-original' ? '未报告（Original）' : status.decoder}`,
    status.filter && `滤镜 ${status.filter}`,
    `提交状态 ${applyStateLabel}`,
    `状态修订 ${status.status_revision}`,
    `已确认变化 ${status.confirmed_change_count} 次`,
    status.actual_source_fps === null ? null : `源帧率 ${status.actual_source_fps}fps`,
    slotLabel('N', status.n),
    slotLabel('N+1', status.n1),
    slotLabel('N+2', status.n2),
    status.cycle_drift_ms === null ? null : `周期漂移 ${status.cycle_drift_ms >= 0 ? '+' : ''}${status.cycle_drift_ms}ms`,
    status.av_sync_drift_ms === null
      ? null
      : `音画漂移 ${status.av_sync_drift_ms >= 0 ? '+' : ''}${status.av_sync_drift_ms}ms`,
    status.gpu_pass_p99_ms === null ? null : `GPU pass P99 ${status.gpu_pass_p99_ms}ms`,
    status.frame_drop_count === null ? null : `VO 丢帧 ${status.frame_drop_count}`,
    status.decoder_frame_drop_count === null ? null : `解码丢帧 ${status.decoder_frame_drop_count}`,
    status.mistimed_frame_count === null ? null : `错时帧 ${status.mistimed_frame_count}`,
    status.delayed_frame_count === null ? null : `延迟帧 ${status.delayed_frame_count}`,
    status.physical_paused === null ? null : `物理状态 ${status.physical_paused ? '暂停' : '播放中'}`,
    status.eof === null ? null : 'EOF 待后端原子推进',
    status.frame_budget_violation_windows === 0
      ? null
      : `帧预算连续异常 ${status.frame_budget_violation_windows}/3`,
    status.resume_pts_ms === null ? null : `恢复 PTS ${status.resume_pts_ms}ms`,
    status.transition_started_at_unix_ms === null
      ? null
      : `降级开始 ${status.transition_started_at_unix_ms}`,
    status.transition_completed_at_unix_ms === null
      ? null
      : `降级完成 ${status.transition_completed_at_unix_ms}`,
    status.demotion_reason && `降级原因：${sanitizeDiagnosticDetail(status.demotion_reason)}`,
    status.backend === 'cpu4'
      ? `CPU4 固定未执行 ${status.unsupported_parameter_count} 项，其中当前活动 ${status.ignored_active_parameter_count} 项`
      : null,
    skippedParameters.length === 0
      ? null
      : `本周期未参与参数 ${skippedParameters.length} 个（${skippedParameters
        .slice(0, 3)
        .map((parameter) => parameter.field)
        .join('、')}${skippedParameters.length > 3 ? `，另有 ${skippedParameters.length - 3} 个` : ''}）`,
  ].filter((part): part is string => part !== null);

  const realtimePartialLabel = active
    && status.backend === 'realtime_gpu'
    && skippedParameters.length > 0
    ? '尚未完整生效（正在转入下一级回退）'
    : null;

  return {
    valid: true,
    active,
    effects_applied: effectsApplied,
    backend: status.backend,
    label: `${backendLabel(status.backend)}${status.backend === 'source' && active
      ? '输出中（未应用视频参数）'
      : status.activation === 'active' && !active
        ? '状态待确认'
        : realtimePartialLabel ?? activationLabel}`,
    detail: pipeline.length > 0 ? pipeline.join(' · ') : '无额外运行信息',
  };
}
