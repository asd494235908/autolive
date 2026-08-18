export type RuntimeResourceComponent = 'media';

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

export type RuntimeResourceEnsureDecision = 'resume' | 'wait' | 'install' | 'conflict';

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

export function isRuntimeResourceConflict(
  requestedComponent: RuntimeResourceComponent,
  status: Pick<RuntimeResourceStatus, 'state' | 'component'>,
): boolean {
  return isRuntimeResourceBusy(status)
    && status.component !== null
    && status.component !== requestedComponent;
}
