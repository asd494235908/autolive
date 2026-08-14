import { fetch } from '@tauri-apps/plugin-http';

const DEFAULT_CONTROL_PLANE_BASE_URL = 'http://192.168.100.213:18090';
const configuredControlPlaneBaseUrl = (
  import.meta as ImportMeta & { env?: { VITE_CONTROL_PLANE_BASE_URL?: string } }
).env?.VITE_CONTROL_PLANE_BASE_URL?.trim();

export const CONTROL_PLANE_BASE_URL = (
  configuredControlPlaneBaseUrl || DEFAULT_CONTROL_PLANE_BASE_URL
).replace(/\/+$/, '');
export interface ErrorResponseDto { code: string; message: string; request_id: string }
export interface UserSummaryDto { id: string; username: string; role: 'admin' | 'user'; status: 'active' | 'disabled'; created_at: string }
export interface DeviceSummaryDto {
  id: string; user_id: string; device_name: string; platform: string; app_version: string;
  status: 'pending_activation' | 'active' | 'disabled' | 'revoked'; disk_free_bytes?: number;
  memory_total_bytes?: number; memory_available_bytes?: number; cpu_logical_cores?: number;
  runtime_os_name?: string; runtime_os_version?: string; kernel_version?: string; last_seen_at: string;
}
export interface LoginRequestDto { username: string; password: string }
export interface LoginResponseDto { request_id: string; tokens: { access_token: string; refresh_token: string; expires_at: string }; user: UserSummaryDto }
export interface RefreshTokenResponseDto { request_id: string; tokens: LoginResponseDto['tokens'] }
export interface LogoutResponseDto { request_id: string; success: true }
export interface DeviceRegistrationDto { device_id: string; device_name: string; platform: string; app_version: string; os_version?: string }
export interface ActivateDeviceRequestDto { activation_code: string; device: DeviceRegistrationDto }
export interface ActivateDeviceResponseDto { request_id: string; device: DeviceSummaryDto }
export interface ClientProfileResponseDto { request_id: string; user: UserSummaryDto; device: DeviceSummaryDto; permissions: string[] }
export interface HeartbeatRequestDto {
  device_id: string; sent_at: string;
  status: { disk_free_bytes: number; memory_total_bytes?: number; memory_available_bytes?: number; cpu_logical_cores?: number; os_name?: string; os_version?: string; kernel_version?: string; current_media_name?: string; playback_state?: 'idle' | 'playing' | 'paused' | 'error' }
}
export interface HeartbeatResponseDto { request_id: string; accepted_at: string; device_status: DeviceSummaryDto['status'] }
export interface CreateModelLeaseRequestDto { provider: string; model: string; purpose: string; max_duration_seconds: number }
export interface ModelLeaseDto { id: string; provider: string; model: string; status: 'active' | 'released' | 'expired'; expires_at: string; proxy_mode: 'direct_lease'; direct_base_url?: string; concurrency_limit: number }
export interface ModelLeaseResponseDto { request_id: string; lease: ModelLeaseDto }
export interface ReleaseModelLeaseResponseDto { request_id: string; lease_id: string; released: boolean }
export interface DirectLLMCallRecordRequestDto { client_call_id: string; lease_id: string; provider: string; model: string; input_tokens: number; output_tokens: number; total_tokens: number; latency_ms: number; status: 'succeeded' | 'failed' | 'timeout' | 'cancelled' | 'unknown'; usage_source: 'client_reported'; error_code?: string }
export interface DirectLLMCallRecordResponseDto { request_id: string; recorded: boolean }

export class ControlPlaneError extends Error {
  code: string; status: number; requestId?: string;
  constructor(input: { code: string; message: string; status: number; requestId?: string }) { super(input.message); this.name = 'ControlPlaneError'; this.code = input.code; this.status = input.status; this.requestId = input.requestId }
}
async function parseJsonSafely(response: Response): Promise<unknown> { const text = await response.text(); if (!text) return null; try { return JSON.parse(text) as unknown } catch { return text } }
type SessionRefreshHandler = () => Promise<string | null>;
let sessionRefreshHandler: SessionRefreshHandler | null = null;
let sessionRefreshInFlight: Promise<string | null> | null = null;
export function setSessionRefreshHandler(handler: SessionRefreshHandler | null) { sessionRefreshHandler = handler }
async function refreshExpiredSession(): Promise<string | null> { if (sessionRefreshInFlight) return sessionRefreshInFlight; if (!sessionRefreshHandler) return null; sessionRefreshInFlight = sessionRefreshHandler().finally(() => { sessionRefreshInFlight = null }); return sessionRefreshInFlight }

async function requestJson<T>(path: string, init: { method: 'GET' | 'POST'; body?: unknown; accessToken?: string; idempotencyKey?: string; timeoutMs?: number }, allowAuthRefresh = true): Promise<T> {
  const headers = new Headers({ Accept: 'application/json' });
  if (init.body !== undefined) headers.set('Content-Type', 'application/json');
  if (init.accessToken) headers.set('Authorization', `Bearer ${init.accessToken}`);
  if (init.idempotencyKey) headers.set('Idempotency-Key', init.idempotencyKey);
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), init.timeoutMs ?? 15_000);
  let response: Response;
  try { response = await fetch(`${CONTROL_PLANE_BASE_URL}${path}`, { method: init.method, headers, body: init.body === undefined ? undefined : JSON.stringify(init.body), connectTimeout: 5_000, signal: controller.signal }) }
  catch (error) { if (controller.signal.aborted) throw new ControlPlaneError({ code: 'CONTROL_PLANE_TIMEOUT', message: '控制面请求超时', status: 408 }); throw error }
  finally { clearTimeout(timeout) }
  const payload = await parseJsonSafely(response);
  if (!response.ok) {
    if (response.status === 401 && allowAuthRefresh && init.accessToken && !path.startsWith('/api/v1/auth/')) { const refreshed = await refreshExpiredSession(); if (refreshed && refreshed !== init.accessToken) return requestJson<T>(path, { ...init, accessToken: refreshed }, false) }
    if (payload && typeof payload === 'object' && 'code' in payload && 'message' in payload) { const errorPayload = payload as ErrorResponseDto; throw new ControlPlaneError({ code: errorPayload.code, message: errorPayload.message, requestId: errorPayload.request_id, status: response.status }) }
    throw new ControlPlaneError({ code: 'http_error', message: `${response.status} ${response.statusText}`.trim(), status: response.status })
  }
  return payload as T;
}
export function buildIdempotencyKey(): string { return crypto.randomUUID() }
export function loginControlPlane(request: LoginRequestDto) { return requestJson<LoginResponseDto>('/api/v1/auth/login', { method: 'POST', body: request }) }
export function refreshControlPlane(refreshToken: string) { return requestJson<RefreshTokenResponseDto>('/api/v1/auth/refresh', { method: 'POST', body: { refresh_token: refreshToken } }, false) }
export function logoutControlPlane(accessToken: string, refreshToken?: string) { return requestJson<LogoutResponseDto>('/api/v1/auth/logout', { method: 'POST', body: refreshToken ? { refresh_token: refreshToken } : undefined, accessToken }, false) }
export function activateDeviceControlPlane(accessToken: string, request: ActivateDeviceRequestDto) { return requestJson<ActivateDeviceResponseDto>('/api/v1/client/activate', { method: 'POST', body: request, accessToken, idempotencyKey: buildIdempotencyKey() }) }
export function getClientProfileControlPlane(accessToken: string) { return requestJson<ClientProfileResponseDto>('/api/v1/client/profile', { method: 'GET', accessToken }) }
export function sendHeartbeatControlPlane(accessToken: string, request: HeartbeatRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<HeartbeatResponseDto>('/api/v1/client/heartbeat', { method: 'POST', body: request, accessToken, idempotencyKey }) }
export function requestModelLeaseControlPlane(accessToken: string, request: CreateModelLeaseRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<ModelLeaseResponseDto>('/api/v1/client/model-leases', { method: 'POST', body: request, accessToken, idempotencyKey }) }
export function renewModelLeaseControlPlane(accessToken: string, leaseId: string, extendSeconds: number, idempotencyKey = buildIdempotencyKey()) { return requestJson<ModelLeaseResponseDto>(`/api/v1/client/model-leases/${leaseId}/renew`, { method: 'POST', body: { extend_seconds: extendSeconds }, accessToken, idempotencyKey }) }
export function releaseModelLeaseControlPlane(accessToken: string, leaseId: string, idempotencyKey = buildIdempotencyKey()) { return requestJson<ReleaseModelLeaseResponseDto>(`/api/v1/client/model-leases/${leaseId}/release`, { method: 'POST', accessToken, idempotencyKey }) }
export function recordDirectLLMCallControlPlane(accessToken: string, request: DirectLLMCallRecordRequestDto, idempotencyKey = buildIdempotencyKey()) { return requestJson<DirectLLMCallRecordResponseDto>('/api/v1/client/llm/call-records', { method: 'POST', body: request, accessToken, idempotencyKey }) }
