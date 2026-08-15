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
type PendingRuntimeActionDescriptor = Pick<RuntimeResourceStatus, 'component'> & { token: number };

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

export function isRuntimeResourceConflict(
  requestedComponent: RuntimeResourceComponent,
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
): boolean {
  return isRuntimeResourceBusy(status)
    && status.component !== null
    && status.component !== requestedComponent;
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
  const component = status.component === 'voice' ? '语音资源' : '媒体资源';
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
