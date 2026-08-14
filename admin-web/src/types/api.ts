export type Id = string;
export type Timestamp = string;
export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export type ApiErrorDetail = {
  field: string;
  reason: string;
};

export type ApiErrorResponse = {
  code: string;
  message: string;
  request_id: string;
  details?: ApiErrorDetail[];
};

export type ApiRequestOptions = {
  method?: HttpMethod;
  body?: unknown;
  headers?: Record<string, string>;
  query?: Record<string, string | number | boolean | null | undefined>;
  requestId?: string;
  timeoutMs?: number;
  signal?: AbortSignal;
};

export type ActorRole = 'admin' | 'user';
export type UserStatus = 'active' | 'disabled';
export type DeviceStatus = 'pending_activation' | 'active' | 'disabled' | 'revoked';
export type ActivationCodeStatus = 'active' | 'used' | 'expired' | 'revoked';
export type ModelAccountStatus = 'active' | 'cooldown' | 'exhausted' | 'disabled';

export type Pagination = {
  page: number;
  page_size: number;
  total: number;
};

export type SessionTokens = {
  access_token: string;
  refresh_token: string;
  expires_at: Timestamp;
};

export type UserSummary = {
  id: Id;
  username: string;
  role: ActorRole;
  status: UserStatus;
  created_at: Timestamp;
};

export type DeviceSummary = {
  id: Id;
  user_id: Id;
  device_name: string;
  platform: string;
  app_version: string;
  status: DeviceStatus;
  disk_free_bytes?: number;
  memory_total_bytes?: number;
  memory_available_bytes?: number;
  cpu_logical_cores?: number;
  runtime_os_name?: string;
  runtime_os_version?: string;
  kernel_version?: string;
  last_seen_at: Timestamp;
};

export type ActivationCode = {
  id: Id;
  status: ActivationCodeStatus;
  expires_at: Timestamp;
  max_devices: number;
  plain_code?: string | null;
};


export type ModelPoolAccountSummary = {
  id: Id;
  provider: string;
  model: string;
  base_url?: string;
  status: ModelAccountStatus;
  priority?: number;
  daily_limit?: number;
  concurrency_limit?: number;
  secret_configured: boolean;
  active_leases: number;
  daily_used_tokens: number;
  last_test_status?: 'succeeded' | 'failed' | 'timeout';
  last_tested_at?: Timestamp;
};

export type AuditLog = {
  id: Id;
  actor_user_id?: string | null;
  device_id?: string | null;
  action: string;
  target_type: string;
  target_id?: string | null;
  request_id?: string | null;
  created_at: Timestamp;
};

export type LoginRequest = {
  username: string;
  password: string;
};

export type LoginResponse = {
  request_id: string;
  tokens: SessionTokens;
  user: UserSummary;
};

export type RefreshTokenResponse = {
  request_id: string;
  tokens: SessionTokens;
};

export type LogoutResponse = {
  request_id: string;
  success: true;
};

export type HealthResponse = {
  request_id: string;
  status: 'ok' | 'degraded';
  service: string;
  version: string;
  now: Timestamp;
};

export type UserListResponse = {
  request_id: string;
  items: UserSummary[];
  pagination: Pagination;
};

export type UserEnvelope = {
  request_id: string;
  user: UserSummary;
};

export type CreateUserRequest = {
  username: string;
  password: string;
  role: ActorRole;
};

export type DeviceListResponse = {
  request_id: string;
  items: DeviceSummary[];
  pagination: Pagination;
};

export type DeviceEnvelope = {
  request_id: string;
  device: DeviceSummary;
};

export type ActivationCodeListResponse = {
  request_id: string;
  items: ActivationCode[];
  pagination: Pagination;
};

export type CreateActivationCodeRequest = {
  expires_at: Timestamp;
  max_devices: 1;
};

export type ActivationCodeEnvelope = {
  request_id: string;
  activation_code: ActivationCode;
};

export type ModelPoolResponse = {
  request_id: string;
  accounts: ModelPoolAccountSummary[];
};

export type CreateModelPoolAccountRequest = {
  provider: string;
  model: string;
  base_url: string;
  api_key: string;
  priority: number;
  daily_limit: number;
  concurrency_limit: number;
};

export type UpdateModelPoolAccountRequest = {
  base_url?: string;
  priority?: number;
  daily_limit?: number;
  concurrency_limit?: number;
  status?: ModelAccountStatus;
};

export type ModelPoolAccountEnvelope = {
  request_id: string;
  account: ModelPoolAccountSummary;
};

export type ModelPoolConnectivityTestResponse = {
  request_id: string;
  account_id: Id;
  provider: string;
  model: string;
  status: 'succeeded' | 'failed' | 'timeout';
  tested_at: Timestamp;
  latency_ms: number;
  http_status?: number;
  response_summary?: string;
  error_code?: string;
};

export type ModelUsageRecord = {
  id: Id;
  lease_id: Id;
  request_id: Id;
  client_call_id: Id;
  provider: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  latency_ms: number;
  status: 'succeeded' | 'failed' | 'timeout' | 'cancelled' | 'unknown';
  usage_source: 'client_reported';
  error_code?: string;
  created_at: Timestamp;
};

export type ModelUsageListResponse = {
  request_id: string;
  items: ModelUsageRecord[];
  pagination: Pagination;
};

export type AuditLogListResponse = {
  request_id: string;
  items: AuditLog[];
  pagination: Pagination;
};
