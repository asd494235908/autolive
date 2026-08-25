package controlplane

import "time"

const (
	RoleAdmin = "admin"
	RoleUser  = "user"

	UserStatusActive   = "active"
	UserStatusDisabled = "disabled"

	DeviceStatusPendingActivation = "pending_activation"
	DeviceStatusActive            = "active"
	DeviceStatusDisabled          = "disabled"
	DeviceStatusRevoked           = "revoked"

	ActivationCodeStatusActive  = "active"
	ActivationCodeStatusUsed    = "used"
	ActivationCodeStatusExpired = "expired"
	ActivationCodeStatusRevoked = "revoked"
	MaxActivationCodeDevices    = 100

	ModelAccountStatusActive    = "active"
	ModelAccountStatusCooldown  = "cooldown"
	ModelAccountStatusExhausted = "exhausted"
	ModelAccountStatusDisabled  = "disabled"

	ModelLeaseStatusActive         = "active"
	ModelLeaseStatusReleased       = "released"
	ModelLeaseStatusExpired        = "expired"
	ModelLeaseProxyModeDirectLease = "direct_lease"

	HashAlgorithmSHA256 = "sha256"
)

type Actor struct {
	UserID  string
	Role    string
	Product ProductCode
}

type AuditLog struct {
	ID          string      `json:"id"`
	Product     ProductCode `json:"product"`
	ActorUserID string      `json:"actor_user_id,omitempty"`
	DeviceID    string      `json:"device_id,omitempty"`
	Action      string      `json:"action"`
	TargetType  string      `json:"target_type"`
	TargetID    string      `json:"target_id,omitempty"`
	Outcome     string      `json:"outcome"`
	StatusCode  int         `json:"status_code"`
	ErrorCode   string      `json:"error_code,omitempty"`
	RequestID   string      `json:"request_id,omitempty"`
	CreatedAt   string      `json:"created_at"`
}

type AuditLogInput struct {
	ActorUserID string
	Product     ProductCode
	DeviceID    string
	Action      string
	TargetType  string
	TargetID    string
	Outcome     string
	StatusCode  int
	ErrorCode   string
	RequestID   string
}

type UserSummary struct {
	ID        string `json:"id"`
	Username  string `json:"username"`
	Role      string `json:"role"`
	Status    string `json:"status"`
	CreatedAt string `json:"created_at"`
}

// UserAuthorizationPolicy is the administrator-controlled access policy for a
// user. An empty AllowedModels list means the user may request any registered
// model. DailyTokenLimit is enforced against accepted server records only; it
// is not provider-authoritative billing and does not reserve future tokens.
type UserAuthorizationPolicy struct {
	UserID          string   `json:"user_id"`
	AllowedModels   []string `json:"allowed_models"`
	DailyTokenLimit int      `json:"daily_token_limit"`
	UpdatedAt       string   `json:"updated_at"`
}

// UserAuthorizationSummary is an operational snapshot for administrators.
// Daily usage is derived from client-reported records and is deliberately
// marked as soft usage; it is not a provider-authoritative quota.
type UserAuthorizationSummary struct {
	UserID              string   `json:"user_id"`
	DeviceCount         int      `json:"device_count"`
	ActiveDeviceCount   int      `json:"active_device_count"`
	ActiveLeaseCount    int      `json:"active_lease_count"`
	ActiveAccountCount  int      `json:"active_account_count"`
	DailyUsedTokens     int      `json:"daily_used_tokens"`
	UsageSource         string   `json:"usage_source"`
	HardQuotaConfigured bool     `json:"hard_quota_configured"`
	AllowedModels       []string `json:"allowed_models"`
	DailyTokenLimit     int      `json:"daily_token_limit"`
	QuotaEnforcement    string   `json:"quota_enforcement"`
	AsOf                string   `json:"as_of"`
}

type DeviceSummary struct {
	ID                   string      `json:"id"`
	UserID               string      `json:"user_id"`
	Product              ProductCode `json:"product"`
	DeviceName           string      `json:"device_name"`
	Platform             string      `json:"platform"`
	AppVersion           string      `json:"app_version"`
	Status               string      `json:"status"`
	DiskFreeBytes        int64       `json:"disk_free_bytes,omitempty"`
	MemoryTotalBytes     int64       `json:"memory_total_bytes,omitempty"`
	MemoryAvailableBytes int64       `json:"memory_available_bytes,omitempty"`
	CPULogicalCores      int         `json:"cpu_logical_cores,omitempty"`
	RuntimeOSName        string      `json:"runtime_os_name,omitempty"`
	RuntimeOSVersion     string      `json:"runtime_os_version,omitempty"`
	KernelVersion        string      `json:"kernel_version,omitempty"`
	CurrentMediaName     string      `json:"current_media_name,omitempty"`
	PlaybackState        string      `json:"playback_state,omitempty"`
	Online               bool        `json:"online"`
	LastSeenAt           string      `json:"last_seen_at"`
	ActivationExpiresAt  *string     `json:"activation_expires_at,omitempty"`
}

type HeartbeatStatus struct {
	DiskFreeBytes        int64  `json:"disk_free_bytes"`
	MemoryTotalBytes     int64  `json:"memory_total_bytes,omitempty"`
	MemoryAvailableBytes int64  `json:"memory_available_bytes,omitempty"`
	CPULogicalCores      int    `json:"cpu_logical_cores,omitempty"`
	OSName               string `json:"os_name,omitempty"`
	OSVersion            string `json:"os_version,omitempty"`
	KernelVersion        string `json:"kernel_version,omitempty"`
	CurrentMediaName     string `json:"current_media_name,omitempty"`
	PlaybackState        string `json:"playback_state,omitempty"`
}

type HeartbeatInput struct {
	Product  ProductCode     `json:"product"`
	DeviceID string          `json:"device_id"`
	SentAt   time.Time       `json:"sent_at"`
	Status   HeartbeatStatus `json:"status"`
}

type HeartbeatResult struct {
	AcceptedAt   string `json:"accepted_at"`
	DeviceStatus string `json:"device_status"`
}

type ClientProfile struct {
	Product     ProductCode   `json:"product"`
	User        UserSummary   `json:"user"`
	Device      DeviceSummary `json:"device"`
	Permissions []string      `json:"permissions"`
}

type DeviceRegistration struct {
	Product    ProductCode `json:"product"`
	DeviceID   string      `json:"device_id"`
	DeviceName string      `json:"device_name"`
	Platform   string      `json:"platform"`
	AppVersion string      `json:"app_version"`
	OSVersion  string      `json:"os_version,omitempty"`
}

type ActivationCode struct {
	ID             string      `json:"id"`
	Product        ProductCode `json:"product"`
	UserID         string      `json:"user_id,omitempty"`
	Status         string      `json:"status"`
	ExpiresAt      string      `json:"expires_at"`
	MaxDevices     int         `json:"max_devices"`
	BoundDevices   int         `json:"bound_devices"`
	CodePrefix     string      `json:"code_prefix,omitempty"`
	UsedByUserID   string      `json:"used_by_user_id,omitempty"`
	UsedByDeviceID string      `json:"used_by_device_id,omitempty"`
	UsedAt         string      `json:"used_at,omitempty"`
	PlainCode      *string     `json:"plain_code"`
}

type ModelPoolAccountSummary struct {
	ID               string      `json:"id"`
	Product          ProductCode `json:"product"`
	Provider         string      `json:"provider"`
	Model            string      `json:"model"`
	BaseURL          string      `json:"base_url,omitempty"`
	Status           string      `json:"status"`
	Priority         int         `json:"priority"`
	DailyLimit       int         `json:"daily_limit"`
	ConcurrencyLimit int         `json:"concurrency_limit"`
	SecretConfigured bool        `json:"secret_configured"`
	ActiveLeases     int         `json:"active_leases"`
	DailyUsedTokens  int         `json:"daily_used_tokens"`
	CooldownUntil    string      `json:"cooldown_until,omitempty"`
	LastTestStatus   string      `json:"last_test_status,omitempty"`
	LastTestedAt     string      `json:"last_tested_at,omitempty"`
	SecretRef        string      `json:"-"`
}

type CreateModelPoolAccountInput struct {
	Provider         string `json:"provider"`
	Model            string `json:"model"`
	BaseURL          string `json:"base_url,omitempty"`
	APIKey           string `json:"api_key"`
	Priority         int    `json:"priority"`
	DailyLimit       int    `json:"daily_limit"`
	ConcurrencyLimit int    `json:"concurrency_limit"`
	Status           string `json:"status,omitempty"`
}

type UpdateModelPoolAccountInput struct {
	BaseURL          *string `json:"base_url,omitempty"`
	Priority         *int    `json:"priority,omitempty"`
	DailyLimit       *int    `json:"daily_limit,omitempty"`
	ConcurrencyLimit *int    `json:"concurrency_limit,omitempty"`
	Status           *string `json:"status,omitempty"`
}

type TestModelPoolAccountInput struct {
	TimeoutSeconds int `json:"timeout_seconds,omitempty"`
}

type RotateModelPoolAccountSecretInput struct {
	APIKey         string `json:"api_key"`
	TimeoutSeconds int    `json:"timeout_seconds,omitempty"`
}

type ModelPoolConnectivityTestResult struct {
	AccountID       string `json:"account_id"`
	Provider        string `json:"provider"`
	Model           string `json:"model"`
	Status          string `json:"status"`
	TestedAt        string `json:"tested_at"`
	LatencyMS       int64  `json:"latency_ms"`
	HTTPStatus      int    `json:"http_status,omitempty"`
	ResponseSummary string `json:"response_summary,omitempty"`
	ErrorCode       string `json:"error_code,omitempty"`
}

type CreateModelLeaseInput struct {
	Provider           string `json:"provider"`
	Model              string `json:"model"`
	Purpose            string `json:"purpose"`
	MaxDurationSeconds int    `json:"max_duration_seconds,omitempty"`
}

type RenewModelLeaseInput struct {
	ExtendSeconds int `json:"extend_seconds,omitempty"`
}

type ReleaseModelLeaseInput struct {
	Reason string `json:"reason,omitempty"`
}

type ModelLease struct {
	ID               string      `json:"id"`
	Product          ProductCode `json:"product"`
	UserID           string      `json:"-"`
	DeviceID         string      `json:"-"`
	AccountID        string      `json:"-"`
	Purpose          string      `json:"-"`
	Provider         string      `json:"provider"`
	Model            string      `json:"model"`
	Status           string      `json:"status"`
	CreatedAt        string      `json:"created_at,omitempty"`
	ExpiresAt        string      `json:"expires_at"`
	ReleasedAt       string      `json:"released_at,omitempty"`
	ProxyMode        string      `json:"proxy_mode"`
	DirectBaseURL    string      `json:"direct_base_url,omitempty"`
	ConcurrencyLimit int         `json:"concurrency_limit"`
}

// ModelLeaseAdminSummary is the redacted control-plane view of a lease.
// It exposes ownership and lifecycle metadata to administrators without any
// lease credential or provider secret.
type ModelLeaseAdminSummary struct {
	ID               string      `json:"id"`
	Product          ProductCode `json:"product"`
	AccountID        string      `json:"account_id"`
	UserID           string      `json:"user_id"`
	DeviceID         string      `json:"device_id"`
	Purpose          string      `json:"purpose"`
	Provider         string      `json:"provider"`
	Model            string      `json:"model"`
	Status           string      `json:"status"`
	ExpiresAt        string      `json:"expires_at"`
	ProxyMode        string      `json:"proxy_mode"`
	ConcurrencyLimit int         `json:"concurrency_limit"`
}

// ModelLeaseAdminDetail is a redacted single-lease view. It deliberately
// excludes direct credentials and provider secrets while exposing lifecycle
// timestamps needed for safe administrative recovery.
type ModelLeaseAdminDetail struct {
	ID               string      `json:"id"`
	Product          ProductCode `json:"product"`
	AccountID        string      `json:"account_id"`
	UserID           string      `json:"user_id"`
	DeviceID         string      `json:"device_id"`
	Purpose          string      `json:"purpose"`
	Provider         string      `json:"provider"`
	Model            string      `json:"model"`
	Status           string      `json:"status"`
	CreatedAt        string      `json:"created_at,omitempty"`
	ExpiresAt        string      `json:"expires_at"`
	ReleasedAt       string      `json:"released_at,omitempty"`
	ProxyMode        string      `json:"proxy_mode"`
	ConcurrencyLimit int         `json:"concurrency_limit"`
}

type ReleaseModelLeaseResult struct {
	LeaseID  string `json:"lease_id"`
	Released bool   `json:"released"`
}

type ModelUsageRecord struct {
	ID           string      `json:"id"`
	Product      ProductCode `json:"product"`
	LeaseID      string      `json:"lease_id"`
	ClientCallID string      `json:"client_call_id"`
	RequestID    string      `json:"request_id"`
	Provider     string      `json:"provider"`
	Model        string      `json:"model"`
	InputTokens  int         `json:"input_tokens"`
	OutputTokens int         `json:"output_tokens"`
	TotalTokens  int         `json:"total_tokens"`
	LatencyMS    int64       `json:"latency_ms"`
	Status       string      `json:"status"`
	UsageSource  string      `json:"usage_source"`
	ErrorCode    string      `json:"error_code,omitempty"`
	CreatedAt    string      `json:"created_at"`
}

type CreateDirectLLMCallRecordInput struct {
	ClientCallID string `json:"client_call_id"`
	LeaseID      string `json:"lease_id"`
	Provider     string `json:"provider"`
	Model        string `json:"model"`
	InputTokens  int    `json:"input_tokens,omitempty"`
	OutputTokens int    `json:"output_tokens,omitempty"`
	TotalTokens  int    `json:"total_tokens,omitempty"`
	LatencyMS    int64  `json:"latency_ms,omitempty"`
	Status       string `json:"status"`
	UsageSource  string `json:"usage_source"`
	ErrorCode    string `json:"error_code,omitempty"`
	FinishReason string `json:"finish_reason,omitempty"`
}

type CreateUserInput struct {
	Username string `json:"username"`
	Password string `json:"password"`
	Role     string `json:"role"`
}

type UpdateUserInput struct {
	Username *string `json:"username,omitempty"`
	Role     *string `json:"role,omitempty"`
	Status   *string `json:"status,omitempty"`
}

type UpdateUserAuthorizationInput struct {
	AllowedModels   []string `json:"allowed_models"`
	DailyTokenLimit int      `json:"daily_token_limit"`
}

type ResetUserPasswordInput struct {
	Password string `json:"password"`
}

type ChangeLocalAdminPasswordInput struct {
	Password string `json:"password"`
}

type CreateActivationCodeInput struct {
	UserID     string    `json:"user_id"`
	ExpiresAt  time.Time `json:"expires_at"`
	MaxDevices int       `json:"max_devices"`
}

type ActivateDeviceInput struct {
	Device DeviceRegistration `json:"device"`
}
