import { fetch } from '@tauri-apps/plugin-http';
import type { components as OpenAPIComponents } from './api/openapi.generated';

type ViteEnvironment = { DEV?: boolean; VITE_CONTROL_PLANE_BASE_URL?: string; VITE_CONTROL_PLANE_ENV?: string };
const viteEnvironment = (import.meta as ImportMeta & { env?: ViteEnvironment }).env;
const configuredControlPlaneBaseUrl = viteEnvironment?.VITE_CONTROL_PLANE_BASE_URL?.trim();
const developmentControlPlaneBaseUrl = 'http://127.0.0.1:18090';
const testControlPlaneBaseUrl = 'http://101.96.208.132:9090';
const testControlPlaneBuild = viteEnvironment?.VITE_CONTROL_PLANE_ENV === 'test';

function resolveControlPlaneBaseUrl() {
  const candidate = configuredControlPlaneBaseUrl || (viteEnvironment?.DEV ? developmentControlPlaneBaseUrl : '');
  if (!candidate) {
    throw new Error('VITE_CONTROL_PLANE_BASE_URL must be set for production desktop builds');
  }
  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    throw new Error('VITE_CONTROL_PLANE_BASE_URL must be an absolute URL');
  }
  if (parsed.username || parsed.password || parsed.search || parsed.hash) {
    throw new Error('VITE_CONTROL_PLANE_BASE_URL must not contain credentials, query, or hash');
  }
  const loopbackHttp = parsed.protocol === 'http:'
    && (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1' || parsed.hostname === '[::1]');
  const normalizedBaseUrl = parsed.toString().replace(/\/+$/, '');
  const fixedTestHttp = parsed.protocol === 'http:'
    && testControlPlaneBuild && normalizedBaseUrl === testControlPlaneBaseUrl;
  if (parsed.protocol !== 'https:' && !(viteEnvironment?.DEV && loopbackHttp) && !fixedTestHttp) {
    throw new Error('VITE_CONTROL_PLANE_BASE_URL must use HTTPS; development HTTP is limited to loopback and test HTTP to the fixed test origin');
  }
  return normalizedBaseUrl;
}

export const CONTROL_PLANE_BASE_URL = resolveControlPlaneBaseUrl();
type OpenAPISchemas = OpenAPIComponents['schemas'];
export type ApiErrorDetailDto = OpenAPISchemas['ErrorDetail'];
export type ErrorResponseDto = OpenAPISchemas['ErrorResponse'];
export type UserSummaryDto = OpenAPISchemas['UserSummary'];
export type DeviceSummaryDto = OpenAPISchemas['DeviceSummary'];
export type DeviceRegistrationDto = OpenAPISchemas['DeviceRegistration'];
export type ActivateDeviceRequestDto = Pick<OpenAPISchemas['ActivateDeviceRequest'], 'device'>;
export type ActivateDeviceResponseDto = OpenAPISchemas['ActivateDeviceResponse'];
export type ClientProfileResponseDto = OpenAPISchemas['ClientProfileResponse'];
export type HeartbeatRequestDto = OpenAPISchemas['HeartbeatRequest'];
export type HeartbeatResponseDto = OpenAPISchemas['HeartbeatResponse'];
export type CreateModelLeaseRequestDto = OpenAPISchemas['CreateModelLeaseRequest'];
export type ModelLeaseDto = OpenAPISchemas['ModelLease'];
export type ModelLeaseResponseDto = OpenAPISchemas['ModelLeaseResponse'];
export type ReleaseModelLeaseResponseDto = OpenAPISchemas['ReleaseModelLeaseResponse'];
export type DirectLLMCallRecordRequestDto = OpenAPISchemas['DirectLLMCallRecordRequest'];
export type DirectLLMCallRecordResponseDto = OpenAPISchemas['DirectLLMCallRecordResponse'];

export class ControlPlaneError extends Error {
  code: string; status: number; requestId?: string; details?: ApiErrorDetailDto[];
  constructor(input: { code: string; message: string; status: number; requestId?: string; details?: ApiErrorDetailDto[] }) {
    super(input.message);
    this.name = 'ControlPlaneError';
    this.code = input.code;
    this.status = input.status;
    this.requestId = input.requestId;
    this.details = input.details;
  }
}
const CONTROL_PLANE_MAX_RESPONSE_BYTES = 1024 * 1024;

function createResponseTooLargeError(requestId: string) {
  return new ControlPlaneError({
    code: 'CONTROL_PLANE_RESPONSE_TOO_LARGE',
    message: '控制面响应超过 1 MiB 限制',
    status: 502,
    requestId,
  });
}

async function readBoundedResponseText(response: Response, requestId: string): Promise<string> {
  const contentLength = response.headers.get('content-length')?.trim();
  if (contentLength && /^\d+$/.test(contentLength) && Number(contentLength) > CONTROL_PLANE_MAX_RESPONSE_BYTES) {
    await response.body?.cancel().catch(() => undefined);
    throw createResponseTooLargeError(requestId);
  }
  if (!response.body) return '';

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = '';
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > CONTROL_PLANE_MAX_RESPONSE_BYTES) {
        await reader.cancel().catch(() => undefined);
        throw createResponseTooLargeError(requestId);
      }
      text += decoder.decode(value, { stream: true });
    }
    return text + decoder.decode();
  } finally {
    reader.releaseLock();
  }
}

async function parseJsonSafely(response: Response, requestId: string): Promise<unknown> {
  const text = await readBoundedResponseText(response, requestId);
  if (!text) return null;
  try { return JSON.parse(text) as unknown } catch { return text }
}
function createRequestId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  return `req_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}
function isRecord(value: unknown): value is Record<string, unknown> { return typeof value === 'object' && value !== null; }
type SessionRefreshHandler = () => Promise<string | null>;
let sessionRefreshHandler: SessionRefreshHandler | null = null;
let sessionRefreshInFlight: Promise<string | null> | null = null;
export function setSessionRefreshHandler(handler: SessionRefreshHandler | null) { sessionRefreshHandler = handler }
export function clearSessionRefreshHandler(handler: SessionRefreshHandler) { if (sessionRefreshHandler === handler) sessionRefreshHandler = null }
async function refreshExpiredSession(): Promise<string | null> { if (sessionRefreshInFlight) return sessionRefreshInFlight; if (!sessionRefreshHandler) return null; sessionRefreshInFlight = sessionRefreshHandler().finally(() => { sessionRefreshInFlight = null }); return sessionRefreshInFlight }

async function requestJson<T>(path: string, init: { method: 'GET' | 'POST'; body?: unknown; accessToken?: string; idempotencyKey?: string; timeoutMs?: number; requestId?: string }, allowAuthRefresh = true, allowAuditRetry = true): Promise<T> {
  const requestId = init.requestId ?? createRequestId();
  const headers = new Headers({ Accept: 'application/json', 'X-Request-Id': requestId });
  if (init.body !== undefined) headers.set('Content-Type', 'application/json');
  if (init.accessToken) headers.set('Authorization', `Bearer ${init.accessToken}`);
  if (init.idempotencyKey) headers.set('Idempotency-Key', init.idempotencyKey);
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), init.timeoutMs ?? 15_000);
  let response: Response;
  try {
    response = await fetch(`${CONTROL_PLANE_BASE_URL}${path}`, { method: init.method, headers, body: init.body === undefined ? undefined : JSON.stringify(init.body), connectTimeout: 5_000, signal: controller.signal });
  } catch (error) {
    if (controller.signal.aborted) throw new ControlPlaneError({ code: 'CONTROL_PLANE_TIMEOUT', message: '控制面请求超时', status: 408, requestId });
    if (error instanceof ControlPlaneError) throw error;
    throw new ControlPlaneError({ code: 'NETWORK_ERROR', message: '网络异常，暂时无法连接到控制面', status: 0, requestId });
  }
  finally { clearTimeout(timeout) }
  const payload = await parseJsonSafely(response, requestId);
  if (!response.ok) {
    if (response.status === 503 && allowAuditRetry && init.method !== 'GET' && init.idempotencyKey && isRecord(payload) && payload.code === 'AUDIT_UNAVAILABLE') {
      return requestJson<T>(path, init, allowAuthRefresh, false);
    }
    if (response.status === 401 && allowAuthRefresh && init.accessToken) { const refreshed = await refreshExpiredSession(); if (refreshed && refreshed !== init.accessToken) return requestJson<T>(path, { ...init, accessToken: refreshed }, false) }
    if (isRecord(payload) && typeof payload.code === 'string' && typeof payload.message === 'string') { const errorPayload = payload as unknown as ErrorResponseDto; throw new ControlPlaneError({ code: errorPayload.code, message: errorPayload.message, requestId: errorPayload.request_id ?? requestId, details: errorPayload.details, status: response.status }) }
    throw new ControlPlaneError({ code: 'HTTP_ERROR', message: `${response.status} ${response.statusText}`.trim(), status: response.status, requestId })
  }
  return payload as T;
}
export function buildIdempotencyKey(): string { return createRequestId() }
export function activateDeviceControlPlane(accessToken: string, request: ActivateDeviceRequestDto) { return requestJson<ActivateDeviceResponseDto>('/api/v1/client/activate', { method: 'POST', body: request, accessToken, idempotencyKey: buildIdempotencyKey() }) }
export function getClientProfileControlPlane(accessToken: string) { return requestJson<ClientProfileResponseDto>('/api/v1/client/profile', { method: 'GET', accessToken }) }
export function sendHeartbeatControlPlane(accessToken: string, request: HeartbeatRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<HeartbeatResponseDto>('/api/v1/client/heartbeat', { method: 'POST', body: request, accessToken, idempotencyKey }) }
export function requestModelLeaseControlPlane(accessToken: string, request: CreateModelLeaseRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<ModelLeaseResponseDto>('/api/v1/client/model-leases', { method: 'POST', body: request, accessToken, idempotencyKey }) }
export function renewModelLeaseControlPlane(accessToken: string, leaseId: string, extendSeconds: number, idempotencyKey = buildIdempotencyKey()) { return requestJson<ModelLeaseResponseDto>(`/api/v1/client/model-leases/${leaseId}/renew`, { method: 'POST', body: { extend_seconds: extendSeconds }, accessToken, idempotencyKey }) }
export function releaseModelLeaseControlPlane(accessToken: string, leaseId: string, idempotencyKey = buildIdempotencyKey()) { return requestJson<ReleaseModelLeaseResponseDto>(`/api/v1/client/model-leases/${leaseId}/release`, { method: 'POST', accessToken, idempotencyKey }) }
export function recordDirectLLMCallControlPlane(accessToken: string, request: DirectLLMCallRecordRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<DirectLLMCallRecordResponseDto>('/api/v1/client/llm/call-records', { method: 'POST', body: request, accessToken, idempotencyKey }) }
