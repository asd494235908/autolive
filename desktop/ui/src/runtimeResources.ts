export type RuntimeResourceComponent = 'media' | 'voice';

export type RuntimeResourceState =
  | 'not-installed'
  | 'checking'
  | 'downloading'
  | 'verifying'
  | 'ready'
  | 'failed'
  | 'cancelled';

export type RuntimeResourceStatus = {
  state: RuntimeResourceState;
  component: RuntimeResourceComponent | null;
  current_file: string | null;
  downloaded_bytes: number;
  total_bytes: number;
  bytes_per_second: number;
  installed_bytes: number;
  resource_root: string;
  error: string | null;
};

type RuntimeResourceStateInput = Pick<RuntimeResourceStatus, 'state'>;
type RuntimeResourceLifecycleInput = Pick<RuntimeResourceStatus, 'state' | 'component'>;
type PendingRuntimeActionDescriptor = Pick<RuntimeResourceStatus, 'component'> & { token: number };

export type RuntimeResourceEnsureDecision = 'resume' | 'wait' | 'install' | 'conflict';

export type RuntimeResourceConsumers = {
  importVideoBusy: boolean;
  mediaProcessingBusy: boolean;
  researchRunning: boolean;
  researchActionBusy: boolean;
  voiceCloneActionBusy: boolean;
  voiceCloneModelLoading: boolean;
  preGenerationGenerating: boolean;
  voiceClonePreparing: boolean;
  voiceCloneGenerating: boolean;
  voiceClonePlaybackPreparing: boolean;
  realtimeWorkerRunning: boolean;
};

export type RuntimeResourceClearTerminalAction =
  | 'none'
  | 'clear-capabilities'
  | 'revalidate-capabilities'
  | 'conflict';

export function canResumeRuntimeAction(status: RuntimeResourceStateInput): boolean {
  return status.state === 'ready';
}

export function isCurrentRuntimeResourceAction(currentToken: number, expectedToken?: number): boolean {
  return expectedToken === undefined || currentToken === expectedToken;
}

export function resolvePendingRuntimeAction<T extends PendingRuntimeActionDescriptor>(
  pending: T | null,
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
  currentToken: number,
  expectedToken?: number,
): { pending: T | null; shouldResume: boolean } {
  const shouldResume = pending !== null
    && pending.component === status.component
    && pending.token === currentToken
    && isCurrentRuntimeResourceAction(currentToken, expectedToken)
    && canResumeRuntimeAction(status);
  return shouldResume
    ? { pending: null, shouldResume: true }
    : { pending, shouldResume: false };
}

export function isRuntimeResourceBusy(status: RuntimeResourceStateInput): boolean {
  return ['checking', 'downloading', 'verifying'].includes(status.state);
}

export const shouldPollRuntimeResources = isRuntimeResourceBusy;

export function runtimeResourceEnsureDecision(
  requestedComponent: RuntimeResourceComponent,
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
): RuntimeResourceEnsureDecision {
  if (canResumeRuntimeAction(status)) {
    return status.component === requestedComponent ? 'resume' : 'conflict';
  }
  if (!isRuntimeResourceBusy(status)) return 'install';
  return status.component === null || isRuntimeResourceConflict(requestedComponent, status) ? 'conflict' : 'wait';
}

export function runtimeResourcePollComponent(
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
): RuntimeResourceComponent | null {
  if (!shouldPollRuntimeResources(status)) return null;
  return status.component === null ? 'media' : status.component;
}

export function resolveRuntimeResourceClearLifecycle(
  clearInFlight: boolean,
  status: RuntimeResourceLifecycleInput,
): { inFlight: boolean; terminalAction: RuntimeResourceClearTerminalAction } {
  const isGlobalClear = clearInFlight || status.component === null;
  if (!isGlobalClear) return { inFlight: false, terminalAction: 'none' };
  if (isRuntimeResourceBusy(status)) {
    return { inFlight: true, terminalAction: 'none' };
  }
  if (status.state === 'not-installed') {
    return { inFlight: false, terminalAction: 'clear-capabilities' };
  }
  if (status.state === 'failed' || status.state === 'cancelled') {
    return { inFlight: false, terminalAction: 'revalidate-capabilities' };
  }
  return { inFlight: false, terminalAction: status.state === 'ready' ? 'conflict' : 'none' };
}

export function isRuntimeResourceConflict(
  requestedComponent: RuntimeResourceComponent,
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
): boolean {
  return isRuntimeResourceBusy(status)
    && status.component !== null
    && status.component !== requestedComponent;
}

export function runtimeResourceConsumerBusyReason(consumers: RuntimeResourceConsumers): string | null {
  return Object.values(consumers).some(Boolean)
    ? '本地媒体或语音任务正在使用运行资源，请先完成或取消后再清理。'
    : null;
}

export function isRuntimeResourceRealtimeConsumerBusy(workerStatus: string | null | undefined): boolean {
  return workerStatus === 'running';
}

export function runtimeResourceComponentLabel(component: RuntimeResourceComponent | null): string {
  if (component === 'voice') return '语音资源';
  if (component === 'media') return '媒体资源';
  return '全部运行资源';
}

export function runtimeResourceComponentDescription(component: RuntimeResourceComponent | null): string {
  if (component === 'voice') return 'voice 包含媒体、固定话术运行环境和模型。';
  if (component === 'media') return 'media 包含 FFmpeg 和 FFprobe。';
  return '全部运行资源包含 FFmpeg、固定话术 Worker 和语音模型。';
}

export function isVoiceRuntimeResourceReady(
  status: Pick<RuntimeResourceStatus, 'state' | 'component'> | null,
  workerAvailable: boolean,
  previouslyReady: boolean,
): boolean {
  return previouslyReady
    || workerAvailable
    || (status?.state === 'ready' && status.component === 'voice');
}

export function runtimeResourcePercent(
  status: Pick<RuntimeResourceStatus, 'state' | 'downloaded_bytes' | 'total_bytes'>,
): number {
  if (status.state === 'ready') return 100;
  if (!Number.isFinite(status.total_bytes) || status.total_bytes <= 0) return 0;
  const downloadedBytes = Number.isFinite(status.downloaded_bytes) ? status.downloaded_bytes : 0;
  return Math.round(Math.min(1, Math.max(0, downloadedBytes / status.total_bytes)) * 100);
}

export function formatRuntimeResourceBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / (1024 ** unitIndex);
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

export function runtimeResourceProgressDetails(
  status: Pick<RuntimeResourceStatus, 'downloaded_bytes' | 'total_bytes' | 'bytes_per_second'>,
): string {
  const downloaded = Math.max(0, status.downloaded_bytes);
  const total = Math.max(0, status.total_bytes);
  const remaining = Math.max(0, total - downloaded);
  return `已下载 ${formatRuntimeResourceBytes(downloaded)} / ${formatRuntimeResourceBytes(total)} · 速度 ${formatRuntimeResourceBytes(status.bytes_per_second)}/s · 剩余 ${formatRuntimeResourceBytes(remaining)}`;
}

export function runtimeResourceMessage(
  status: Pick<RuntimeResourceStatus, 'state' | 'component' | 'current_file' | 'resource_root' | 'installed_bytes' | 'error'>,
): string {
  const component = runtimeResourceComponentLabel(status.component);
  switch (status.state) {
    case 'not-installed':
      return `${component}尚未安装，首次使用时会自动下载。`;
    case 'checking':
      return `正在检查${component}和可用磁盘空间。`;
    case 'downloading':
      return `正在下载${component}${status.current_file ? `：${status.current_file}` : ''}`;
    case 'verifying':
      return `正在校验${component}${status.current_file ? `：${status.current_file}` : ''}`;
    case 'ready':
      return `${component}已就绪：${status.resource_root || '应用数据目录'}（${formatRuntimeResourceBytes(status.installed_bytes)}）`;
    case 'failed':
      return `${component}安装失败：${status.error || '未知错误'}`;
    case 'cancelled':
      return `${component}安装已取消，可重试或选择本地资源目录。`;
  }
}
