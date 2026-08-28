export type MediaVideoBackend = 'realtime_gpu' | 'cpu4' | 'source';

export type MediaVideoBackendActivation = 'active' | 'configured' | 'available' | 'failed';

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

export type MediaVideoBackendStatus = Readonly<{
  backend: MediaVideoBackend;
  activation: MediaVideoBackendActivation;
  gpu_adapter: string | null;
  graphics_api: MediaVideoGraphicsApi | null;
  decoder: string | null;
  filter: string | null;
  n: MediaVideoCycleSlot | null;
  n1: MediaVideoCycleSlot | null;
  n2: MediaVideoCycleSlot | null;
  cycle_drift_ms: number | null;
  demotion_reason: string | null;
  support_completeness: MediaVideoSupportCompleteness;
  parameter_support: MediaVideoParameterSupportReport;
}>;

export type MediaVideoBackendStatusView = Readonly<{
  valid: boolean;
  active: boolean;
  effects_applied: boolean;
  backend: MediaVideoBackend | null;
  label: string;
  detail: string;
}>;

const BACKENDS = ['realtime_gpu', 'cpu4', 'source'] as const;
const ACTIVATIONS = ['active', 'configured', 'available', 'failed'] as const;
const GRAPHICS_APIS = ['d3d11', 'vulkan', 'software'] as const;
const SLOT_STATUSES = ['active', 'ready', 'preparing', 'planned', 'late', 'failed'] as const;
const SUPPORT_COMPLETENESS = ['complete', 'incomplete', 'not_applicable'] as const;
const MAX_RUNTIME_LABEL_LENGTH = 256;

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

function hasOwn(record: Record<string, unknown>, field: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, field);
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

export function parseMediaVideoBackendStatus(value: unknown): MediaVideoBackendStatus | null {
  if (!isRecord(value)) return null;
  if (hasOwn(value, 'encoder')) return null;
  const requiredFields = [
    'backend',
    'activation',
    'gpu_adapter',
    'graphics_api',
    'decoder',
    'filter',
    'n',
    'n1',
    'n2',
    'cycle_drift_ms',
    'demotion_reason',
    'support_completeness',
    'parameter_support',
  ];
  if (!requiredFields.every((field) => hasOwn(value, field))) return null;
  if (!isEnumValue(BACKENDS, value.backend)) return null;
  if (!isEnumValue(ACTIVATIONS, value.activation)) return null;
  if (!isRuntimeLabel(value.gpu_adapter)) return null;
  if (value.graphics_api !== null && !isEnumValue(GRAPHICS_APIS, value.graphics_api)) return null;
  if (!isRuntimeLabel(value.decoder) || !isRuntimeLabel(value.filter)) return null;
  if (!isCycleSlot(value.n) || !isCycleSlot(value.n1) || !isCycleSlot(value.n2)) return null;
  if (value.cycle_drift_ms !== null
    && (typeof value.cycle_drift_ms !== 'number' || !Number.isFinite(value.cycle_drift_ms))) return null;
  if (!isRuntimeLabel(value.demotion_reason)) return null;
  if (!isEnumValue(SUPPORT_COMPLETENESS, value.support_completeness)) return null;
  if (!isParameterSupportReport(value.parameter_support)) return null;
  if (value.parameter_support.backend !== value.backend) return null;
  if (value.support_completeness !== 'not_applicable'
    && (value.support_completeness === 'complete') !== value.parameter_support.fullySupported) return null;

  if (value.activation === 'active' && value.backend !== 'source') {
    if (value.n?.status !== 'active') return null;
    if (value.backend === 'realtime_gpu') {
      if (value.support_completeness === 'not_applicable'
        || (value.graphics_api !== 'd3d11' && value.graphics_api !== 'vulkan')) return null;
    } else if (value.support_completeness !== 'complete') return null;
    if (value.backend === 'cpu4') {
      if (value.graphics_api !== 'software' || value.gpu_adapter !== null) return null;
    }
  }
  if (value.activation === 'active' && value.backend === 'source') {
    if (
      value.support_completeness !== 'not_applicable'
      || value.gpu_adapter !== null
      || value.graphics_api !== null
      || value.decoder !== null
      || value.filter !== null
      || value.n !== null
      || value.n1 !== null
      || value.n2 !== null
    ) return null;
  }

  return value as MediaVideoBackendStatus;
}

export function canUseRealtimeVideoBackend(
  status: MediaVideoBackendStatus | null,
): boolean {
  return status?.backend === 'realtime_gpu'
    && (status.activation === 'available' || status.activation === 'active')
    && status.support_completeness === 'complete'
    && status.parameter_support.fullySupported;
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

export function projectMediaVideoBackendStatus(value: unknown): MediaVideoBackendStatusView {
  const status = parseMediaVideoBackendStatus(value);
  if (status === null) {
    return {
      valid: false,
      active: false,
      effects_applied: false,
      backend: null,
      label: '视频后端状态不可用',
      detail: '运行数据校验失败，不能判定视频参数已生效',
    };
  }

  const active = status.activation === 'active';
  const skippedParameters = status.parameter_support.parameters
    .filter((parameter) => parameter.active && !parameter.supported);
  const effectsApplied = active && (
    (status.backend === 'realtime_gpu'
      && status.support_completeness === 'complete'
      && status.parameter_support.fullySupported
      && status.parameter_support.parameters.some((parameter) => parameter.active && parameter.supported))
    || (status.backend !== 'realtime_gpu'
      && status.backend !== 'source'
      && status.support_completeness === 'complete')
  );
  const activationLabel = {
    active: '已生效',
    configured: '已配置，尚未生效',
    available: '可用，尚未生效',
    failed: '运行失败',
  }[status.activation];
  const pipeline = [
    status.gpu_adapter,
    status.graphics_api?.toUpperCase(),
    status.decoder && `解码 ${status.decoder}`,
    status.filter && `滤镜 ${status.filter}`,
    slotLabel('N', status.n),
    slotLabel('N+1', status.n1),
    slotLabel('N+2', status.n2),
    status.cycle_drift_ms === null ? null : `周期漂移 ${status.cycle_drift_ms >= 0 ? '+' : ''}${status.cycle_drift_ms}ms`,
    status.demotion_reason && `降级原因：${status.demotion_reason}`,
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
      : realtimePartialLabel ?? activationLabel}`,
    detail: pipeline.length > 0 ? pipeline.join(' · ') : '无额外运行信息',
  };
}
