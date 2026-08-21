import type { ApiErrorDetail, ApiErrorResponse, ApiRequestOptions, HttpMethod } from '../types/api';
import {
  accessToken,
  clearSession,
  readSession,
  updateSessionTokens
} from '../features/auth/session';
import type { RefreshTokenResponse } from '../types/api';

const DEFAULT_TIMEOUT_MS = 10_000;

export class ApiClientError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly requestId?: string;
  readonly details?: ApiErrorDetail[];

  constructor(
    message: string,
    options: {
      status: number;
      code?: string;
      requestId?: string;
      details?: ApiErrorDetail[];
    }
  ) {
    super(message);
    this.name = 'ApiClientError';
    this.status = options.status;
    this.code = options.code;
    this.requestId = options.requestId;
    this.details = options.details;
  }
}

export function createRequestId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }

  return `req_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

function resolveBaseUrl() {
  const rawBaseUrl = import.meta.env.VITE_API_BASE_URL?.trim();
  return rawBaseUrl ? rawBaseUrl.replace(/\/+$/, '') : '';
}

function buildUrl(path: string, query?: ApiRequestOptions['query']) {
  const url = new URL(
    /^https?:\/\//.test(path) ? path : `${resolveBaseUrl()}${path.startsWith('/') ? path : `/${path}`}`,
    window.location.origin
  );

  if (query) {
    Object.entries(query).forEach(([key, value]) => {
      if (value === undefined || value === null || value === '') {
        return;
      }

      url.searchParams.set(key, String(value));
    });
  }

  if (!/^https?:\/\//.test(path) && !resolveBaseUrl()) {
    return `${url.pathname}${url.search}`;
  }

  return url.toString();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function normalizeError(
  payload: unknown,
  fallbackRequestId: string,
  status: number
): ApiClientError {
  if (isRecord(payload) && typeof payload.code === 'string' && typeof payload.message === 'string') {
    const errorPayload = payload as ApiErrorResponse;
    return new ApiClientError(errorPayload.message, {
      status,
      code: errorPayload.code,
      requestId: errorPayload.request_id ?? fallbackRequestId,
      details: errorPayload.details
    });
  }

  return new ApiClientError(`请求失败（HTTP ${status}）`, {
    status,
    requestId: fallbackRequestId
  });
}

async function parseResponseBody(response: Response) {
  const contentType = response.headers.get('content-type') ?? '';
  if (!contentType.includes('application/json')) {
    return null;
  }

  try {
    return await response.json();
  } catch {
    return null;
  }
}

let refreshInFlight: Promise<boolean> | null = null;

function isAuthPath(path: string) {
  return path.includes('/api/v1/auth/');
}

async function refreshAccessToken() {
  if (refreshInFlight) {
    return refreshInFlight;
  }

  const session = readSession();
  if (!session?.tokens.refresh_token) {
    return false;
  }

  refreshInFlight = (async () => {
    try {
      const response = await request<RefreshTokenResponse>(
        '/api/v1/auth/refresh',
        {
          method: 'POST',
          body: { refresh_token: session.tokens.refresh_token }
        },
        false
      );
      const latest = readSession();
      if (latest?.tokens.refresh_token === session.tokens.refresh_token) {
        updateSessionTokens(response.tokens);
      }
      return true;
    } catch {
      clearSession();
      return false;
    }
  })().finally(() => {
    refreshInFlight = null;
  });

  return refreshInFlight;
}

export async function request<T>(
  path: string,
  options: ApiRequestOptions = {},
  allowAuthRefresh = true,
  allowAuditRetry = true
): Promise<T> {
  const requestId = options.requestId ?? createRequestId();
  const controller = new AbortController();
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);

  try {
    const token = accessToken();
    const method = options.method ?? 'GET';
    const headers: Record<string, string> = {
      Accept: 'application/json',
      'X-Request-Id': requestId,
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(options.headers ?? {})
    };

    if (options.body !== undefined) {
      headers['Content-Type'] = 'application/json';
    }

    const response = await fetch(buildUrl(path, options.query), {
      method,
      headers,
      body: options.body !== undefined ? JSON.stringify(options.body) : undefined,
      signal: options.signal ?? controller.signal,
      credentials: 'include'
    });

    const payload = await parseResponseBody(response);
    const responseRequestId =
      response.headers.get('x-request-id') ??
      (isRecord(payload) && typeof payload.request_id === 'string' ? payload.request_id : requestId);

    if (!response.ok) {
      const hasIdempotencyKey = Object.entries(headers).some(
        ([name, value]) => name.toLowerCase() === 'idempotency-key' && value.trim().length > 0
      );
      if (
        response.status === 503 &&
        allowAuditRetry &&
        method !== 'GET' &&
        hasIdempotencyKey &&
        isRecord(payload) &&
        payload.code === 'AUDIT_UNAVAILABLE'
      ) {
        return request<T>(path, options, allowAuthRefresh, false);
      }
      if (
        response.status === 401 &&
        allowAuthRefresh &&
        !isAuthPath(path) &&
        readSession()?.tokens.refresh_token
      ) {
        if (await refreshAccessToken()) {
          return request<T>(path, options, false);
        }
      }
      if (response.status === 401) {
        clearSession();
      }
      throw normalizeError(payload, responseRequestId, response.status);
    }

    return (payload as T) ?? ({} as T);
  } catch (error) {
    if (error instanceof ApiClientError) {
      throw error;
    }

    if (error instanceof DOMException && error.name === 'AbortError') {
      throw new ApiClientError('请求超时或已取消', {
        status: 0,
        code: 'REQUEST_ABORTED',
        requestId
      });
    }

    throw new ApiClientError('网络异常，暂时无法连接到管理接口', {
      status: 0,
      code: 'NETWORK_ERROR',
      requestId
    });
  } finally {
    window.clearTimeout(timeoutId);
  }
}

export const apiClient = {
  get<T>(path: string, options?: Omit<ApiRequestOptions, 'method'>) {
    return request<T>(path, { ...options, method: 'GET' });
  },
  post<T>(path: string, options?: Omit<ApiRequestOptions, 'method'>) {
    return request<T>(path, { ...options, method: 'POST' });
  },
  request
};

export function isMutationMethod(method: HttpMethod) {
  return method !== 'GET';
}
