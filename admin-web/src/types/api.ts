import type { components as OpenAPIComponents } from '../api/openapi.generated';

type OpenAPISchemas = OpenAPIComponents['schemas'];

export type Id = OpenAPISchemas['Id'];
export type Timestamp = OpenAPISchemas['Timestamp'];
export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export type ApiErrorDetail = OpenAPISchemas['ErrorDetail'];
export type ApiErrorResponse = OpenAPISchemas['ErrorResponse'];

export type ApiRequestOptions = {
  method?: HttpMethod;
  body?: unknown;
  headers?: Record<string, string>;
  query?: Record<string, string | number | boolean | null | undefined>;
  requestId?: string;
  timeoutMs?: number;
  signal?: AbortSignal;
};

export type ActorRole = OpenAPISchemas['ActorRole'];
export type UserStatus = OpenAPISchemas['UserStatus'];
export type DeviceStatus = OpenAPISchemas['DeviceStatus'];
export type ActivationCodeStatus = OpenAPISchemas['ActivationCodeStatus'];
export type ModelAccountStatus = OpenAPISchemas['ModelAccountStatus'];
export type ModelLeaseStatus = OpenAPISchemas['ModelLeaseStatus'];

export type SessionTokens = OpenAPISchemas['SessionTokens'];
export type UserSummary = OpenAPISchemas['UserSummary'];
export type UserAuthorizationSummary = OpenAPISchemas['UserAuthorizationSummary'];
export type UserAuthorizationPolicy = OpenAPISchemas['UserAuthorizationPolicy'];
export type DeviceSummary = OpenAPISchemas['DeviceSummary'];
export type ActivationCode = OpenAPISchemas['ActivationCode'];
export type ModelPoolAccountSummary = OpenAPISchemas['ModelPoolAccountSummary'];
export type AuditLog = OpenAPISchemas['AuditLog'];

export type Pagination = OpenAPISchemas['Pagination'];
export type UserListResponse = OpenAPISchemas['UserListResponse'];
export type UserEnvelope = OpenAPISchemas['UserEnvelope'];
export type UserAuthorizationSummaryResponse = OpenAPISchemas['UserAuthorizationSummaryResponse'];
export type UserAuthorizationPolicyResponse = OpenAPISchemas['UserAuthorizationPolicyResponse'];
export type DeviceListResponse = OpenAPISchemas['DeviceListResponse'];
export type DeviceEnvelope = OpenAPISchemas['DeviceEnvelope'];
export type UnbindDeviceResponse = OpenAPISchemas['UnbindDeviceResponse'];
export type ActivationCodeListResponse = OpenAPISchemas['ActivationCodeListResponse'];
export type ActivationCodeEnvelope = OpenAPISchemas['ActivationCodeEnvelope'];
export type ModelPoolResponse = OpenAPISchemas['ModelPoolResponse'];
export type ModelPoolAccountEnvelope = OpenAPISchemas['ModelPoolAccountEnvelope'];
export type ModelUsageRecord = OpenAPISchemas['ModelUsageRecord'];
export type ModelUsageListResponse = OpenAPISchemas['ModelUsageListResponse'];
export type AuditLogListResponse = OpenAPISchemas['AuditLogListResponse'];
export type HealthResponse = OpenAPISchemas['HealthResponse'];

export type LoginRequest = OpenAPISchemas['LoginRequest'];
export type LoginResponse = OpenAPISchemas['LoginResponse'];
export type RefreshTokenResponse = OpenAPISchemas['RefreshTokenResponse'];
export type LogoutResponse = OpenAPISchemas['LogoutResponse'];
export type CreateUserRequest = OpenAPISchemas['CreateUserRequest'];
export type UpdateUserRequest = OpenAPISchemas['UpdateUserRequest'];
export type ResetUserPasswordRequest = OpenAPISchemas['ResetUserPasswordRequest'];
export type ChangeLocalAdminPasswordRequest = OpenAPISchemas['ChangeLocalAdminPasswordRequest'];
export type UpdateUserAuthorizationRequest = OpenAPISchemas['UpdateUserAuthorizationRequest'];
export type CreateActivationCodeRequest = OpenAPISchemas['CreateActivationCodeRequest'];
export type CreateModelPoolAccountRequest = OpenAPISchemas['CreateModelPoolAccountRequest'];
export type UpdateModelPoolAccountRequest = OpenAPISchemas['UpdateModelPoolAccountRequest'];
export type TestModelPoolAccountRequest = OpenAPISchemas['TestModelPoolAccountRequest'];
export type RotateModelPoolAccountSecretRequest = OpenAPISchemas['RotateModelPoolAccountSecretRequest'];
export type ModelPoolConnectivityTestResponse = OpenAPISchemas['ModelPoolConnectivityTestResponse'];
export type CreateModelLeaseRequest = OpenAPISchemas['CreateModelLeaseRequest'];
export type RenewModelLeaseRequest = OpenAPISchemas['RenewModelLeaseRequest'];
export type ReleaseModelLeaseRequest = OpenAPISchemas['ReleaseModelLeaseRequest'];
export type ModelLeaseResponse = OpenAPISchemas['ModelLeaseResponse'];
export type ModelLeaseAdminSummary = OpenAPISchemas['ModelLeaseAdminSummary'];
export type ModelLeaseListResponse = OpenAPISchemas['ModelLeaseListResponse'];
export type ModelLeaseAdminDetail = OpenAPISchemas['ModelLeaseAdminDetail'];
export type ModelLeaseAdminDetailResponse = OpenAPISchemas['ModelLeaseAdminDetailResponse'];
export type ReleaseModelLeaseResponse = OpenAPISchemas['ReleaseModelLeaseResponse'];
export type DirectLLMCallRecordRequest = OpenAPISchemas['DirectLLMCallRecordRequest'];
export type DirectLLMCallRecordResponse = OpenAPISchemas['DirectLLMCallRecordResponse'];
