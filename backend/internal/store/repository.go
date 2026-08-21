package store

import (
	"context"
	"errors"
	"time"

	"autoLive/backend/internal/controlplane"
)

// StateOperation 是控制面一次受保护读写操作的边界。
// MemoryStore 用互斥锁实现；生产实现必须映射为数据库事务，不得静默回退到内存。
type StateOperation func(state *State) error

// Repository 是 ControlPlane 的可替换持久化边界。
// State 目前保留领域层的统一快照形状，后续 PostgreSQL 适配器负责把它映射到迁移表，
// Secret Store 只负责密钥字段，不得通过该接口把明文密钥返回给 API 层。
type Repository interface {
	Now() time.Time
	Run(ctx context.Context, fn StateOperation) error
}

// TransactionalSessionBinder is the compatibility transaction boundary for
// client activation/heartbeat flows. Implementations bind the authenticated
// session and execute the legacy StateOperation on the same database
// transaction. The normalized PostgreSQL paths use the smaller domain
// interfaces below; Memory and other compatibility repositories keep this
// optional boundary and their existing compensating path.
type TransactionalSessionBinder interface {
	RunWithSessionBinding(ctx context.Context, accessTokenHash, userID, deviceID string, fn StateOperation) error
}

// TransactionalDeviceActivator is the normalized PostgreSQL activation path.
// It redeems one capacity slot from the activation code, writes the device and
// binds the authenticated session in one transaction without loading or
// rewriting the legacy snapshot.
type TransactionalDeviceActivator interface {
	ActivateDeviceWithSessionBinding(ctx context.Context, record DeviceActivationRecord) (controlplane.DeviceSummary, error)
}

// TransactionalHeartbeatRecorder is the normalized PostgreSQL heartbeat path.
// It updates the owned device, idempotency record and authenticated-session
// binding in one transaction without rewriting the legacy snapshot.
type TransactionalHeartbeatRecorder interface {
	RecordHeartbeatWithSessionBinding(ctx context.Context, record DeviceHeartbeatRecord) (controlplane.HeartbeatResult, error)
}

// DeviceLifecycleRepository owns normalized administrative device transitions.
// Lease release and session revocation are committed with the device state so
// an admin operation cannot leave an active lease or session behind.
type DeviceLifecycleRepository interface {
	DisableDevice(ctx context.Context, record DeviceMutationRecord) (controlplane.DeviceSummary, error)
	UnbindDevice(ctx context.Context, record DeviceMutationRecord) (controlplane.DeviceSummary, error)
}

// ModelLeaseRepository owns normalized lease lifecycle writes. Creation has a
// separate capability because it also performs account selection, policy
// checks and SecretStore availability reads.
type ModelLeaseRepository interface {
	RenewModelLease(ctx context.Context, record ModelLeaseRenewRecord) (controlplane.ModelLease, error)
	ReleaseModelLease(ctx context.Context, record ModelLeaseReleaseRecord) (controlplane.ReleaseModelLeaseResult, error)
	ReclaimModelLease(ctx context.Context, record ModelLeaseReclaimRecord) (controlplane.ReleaseModelLeaseResult, error)
}

// ModelLeaseCreator creates a normalized lease only after all ownership,
// policy, quota, account-capacity and secret-availability checks share the
// same transaction boundary.
type ModelLeaseCreator interface {
	CreateModelLease(ctx context.Context, record ModelLeaseCreateRecord) (controlplane.ModelLease, error)
}

// ModelUsageRepository owns normalized client-call summary writes. It stores
// only provider metadata, token counts and outcome fields; model/audio/video
// content never crosses this boundary.
type ModelUsageRepository interface {
	RecordDirectLLMCall(ctx context.Context, record ModelUsageWriteRecord) (controlplane.ModelUsageRecord, error)
}

// ModelPoolRepository owns normalized model-account mutation writes. Creation
// and secret rotation have separate boundaries because they involve SecretStore
// commit/recovery semantics.
type ModelPoolRepository interface {
	DisableModelPoolAccount(ctx context.Context, record ModelPoolAccountMutationRecord) (controlplane.ModelPoolAccountSummary, error)
	UpdateModelPoolAccount(ctx context.Context, record ModelPoolAccountUpdateRecord) (controlplane.ModelPoolAccountSummary, error)
}

// ModelPoolAccountCreator is implemented only by repositories that can commit
// the normalized account row and its encrypted secret in one database
// transaction.
type ModelPoolAccountCreator interface {
	CreateModelPoolAccount(ctx context.Context, record ModelPoolAccountCreateRecord) (controlplane.ModelPoolAccountSummary, error)
}

// ModelPoolSecretRotator owns normalized secret rotation. The candidate
// ciphertext, account reference, test result, idempotency record and optional
// audit event share one SQL transaction; plaintext never enters a repository
// row or response.
type ModelPoolSecretRotator interface {
	RotateModelPoolAccountSecret(ctx context.Context, record ModelPoolSecretRotationRecord) (controlplane.ModelPoolAccountSummary, error)
}

// ModelPoolSecretRotationPreparer reads the normalized account and checks the
// rotation idempotency fact without materializing the compatibility snapshot.
// The returned account is only used for the bounded provider probe; the final
// reference switch remains owned by ModelPoolSecretRotator.
type ModelPoolSecretRotationPreparer interface {
	PrepareModelPoolAccountSecretRotation(ctx context.Context, scope, idempotencyKey, fingerprint, accountID string) (ModelPoolSecretRotationPreparation, error)
}

type ModelPoolSecretRotationPreparation struct {
	Account  controlplane.ModelPoolAccountSummary
	Existing *controlplane.ModelPoolAccountSummary
}

// ModelPoolTestRepository separates the network-free preparation transaction
// from the bounded provider probe. The probe itself never runs while a SQL
// transaction or row lock is held; completion persists the result, cooldown
// transition and idempotency fact atomically.
type ModelPoolTestRepository interface {
	PrepareModelPoolAccountTest(ctx context.Context, record ModelPoolTestPrepareRecord) (ModelPoolTestPreparation, error)
	RecordModelPoolAccountTest(ctx context.Context, record ModelPoolTestRecord) (controlplane.ModelPoolConnectivityTestResult, error)
}

// AuditRepository appends redacted audit facts directly to the normalized
// table. It does not claim an Outbox or business-state transaction boundary.
type AuditRepository interface {
	RecordAudit(ctx context.Context, input controlplane.AuditLogInput) error
}

// AuditOutboxRepository durably queues redacted audit facts before delivery.
// RecordAuditWithOutbox is the request boundary; DispatchAuditOutbox is a
// bounded retry pass used by the retention worker.
type AuditOutboxRepository interface {
	RecordAuditWithOutbox(ctx context.Context, input controlplane.AuditLogInput) error
	DispatchAuditOutbox(ctx context.Context, batchSize int) (int64, error)
}

type DeviceActivationRecord struct {
	Scope              string
	IdempotencyKey     string
	Fingerprint        string
	AccessTokenHash    string
	UserID             string
	Product            controlplane.ProductCode
	ActivationCodeHash string
	Device             controlplane.DeviceRegistration
	// Audit is optional for compatibility callers. Normalized HTTP activation
	// supplies the prevalidated success event so it is committed with the
	// activation, device and session binding transaction.
	Audit controlplane.AuditLogInput
}

type DeviceHeartbeatRecord struct {
	Scope           string
	IdempotencyKey  string
	Fingerprint     string
	AccessTokenHash string
	UserID          string
	Product         controlplane.ProductCode
	Input           controlplane.HeartbeatInput
	// Audit is optional for compatibility callers. Normalized HTTP heartbeat
	// supplies the prevalidated success event for the same transaction.
	Audit controlplane.AuditLogInput
}

type DeviceMutationRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	DeviceID       string
	Product        controlplane.ProductCode
	// Audit is optional for compatibility callers and is enqueued in the
	// same normalized device lifecycle transaction when supplied.
	Audit controlplane.AuditLogInput
}

type ModelLeaseRenewRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	UserID         string
	DeviceID       string
	Product        controlplane.ProductCode
	LeaseID        string
	ExtendSeconds  int
	Audit          controlplane.AuditLogInput
}

type ModelLeaseReleaseRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	UserID         string
	DeviceID       string
	Product        controlplane.ProductCode
	LeaseID        string
	Reason         string
	Audit          controlplane.AuditLogInput
}

type ModelLeaseReclaimRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	LeaseID        string
	Product        controlplane.ProductCode
	Reason         string
	Audit          controlplane.AuditLogInput
}

type ModelLeaseCreateRecord struct {
	Scope              string
	IdempotencyKey     string
	Fingerprint        string
	UserID             string
	DeviceID           string
	Product            controlplane.ProductCode
	Provider           string
	Model              string
	Purpose            string
	MaxDurationSeconds int
	Audit              controlplane.AuditLogInput
}

type ModelUsageWriteRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	UserID         string
	DeviceID       string
	Product        controlplane.ProductCode
	RequestID      string
	Input          controlplane.CreateDirectLLMCallRecordInput
	// Audit is optional for compatibility callers. Normalized client call
	// records provide the success event so usage, account quota state,
	// idempotency and the durable audit outbox commit together.
	Audit controlplane.AuditLogInput
}

type ModelPoolAccountMutationRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	AccountID      string
	Product        controlplane.ProductCode
	// Audit is optional for compatibility callers. Normalized model-account
	// mutations enqueue the success event in the same transaction as the
	// account state and idempotency record.
	Audit controlplane.AuditLogInput
}

type ModelPoolAccountCreateRecord struct {
	Scope            string
	IdempotencyKey   string
	Fingerprint      string
	Product          controlplane.ProductCode
	Provider         string
	Model            string
	BaseURL          string
	APIKey           string
	Status           string
	Priority         int
	DailyLimit       int
	ConcurrencyLimit int
	// Audit is optional for compatibility callers. Normalized account creation
	// commits the success event with the account row and encrypted secret.
	Audit controlplane.AuditLogInput
}

type ModelPoolAccountUpdateRecord struct {
	ModelPoolAccountMutationRecord
	Input controlplane.UpdateModelPoolAccountInput
}

type ModelPoolSecretRotationRecord struct {
	Scope             string
	IdempotencyKey    string
	Fingerprint       string
	AccountID         string
	Product           controlplane.ProductCode
	ExpectedSecretRef string
	APIKey            string
	Probe             controlplane.ModelPoolConnectivityTestResult
	Audit             controlplane.AuditLogInput
}

type ModelPoolTestPrepareRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	AccountID      string
	Product        controlplane.ProductCode
}

type ModelPoolTestPreparation struct {
	Account controlplane.ModelPoolAccountSummary
	Cached  *controlplane.ModelPoolConnectivityTestResult
}

type ModelPoolTestRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	AccountID      string
	Product        controlplane.ProductCode
	Result         controlplane.ModelPoolConnectivityTestResult
	Audit          controlplane.AuditLogInput
}

var ErrSessionDeviceBindingConflict = errors.New("auth session device binding conflict")

// AdvisoryLockRelease releases a session-scoped advisory lock. The caller
// must invoke it exactly once, including when the owning operation is
// cancelled, so the dedicated PostgreSQL connection can be returned to the
// pool.
type AdvisoryLockRelease func(ctx context.Context) error

// AdvisoryLocker provides a bounded, cross-process coordination primitive for
// background jobs. PostgreSQL holds the lock on a dedicated connection for the
// duration of the caller's work; repositories without distributed storage use
// the service's single-process fallback.
type AdvisoryLocker interface {
	TryAdvisoryLock(ctx context.Context, key int64) (AdvisoryLockRelease, bool, error)
}

// UserPageReader provides bounded, ordered list reads for the admin control
// plane. PostgreSQL uses the normalized tables directly; snapshot-backed
// implementations may use the compatibility fallback while migration is in
// progress. The offset and limit are validated by the caller and must remain
// bounded at the API boundary.
type UserPageReader interface {
	ListUsersPage(ctx context.Context, offset, limit int) (UserPage, error)
	ListDevicesPage(ctx context.Context, offset, limit int) (DevicePage, error)
	ListDevicesForUserPage(ctx context.Context, userID string, offset, limit int) (DevicePage, error)
}

// UserRepository owns normalized user writes without exposing the legacy
// control-plane snapshot to callers. PasswordHash is already bcrypt-derived
// at the service boundary and must never be logged or returned.
type UserRepository interface {
	CreateUser(ctx context.Context, scope, idempotencyKey, fingerprint string, record UserCreateRecord) (controlplane.UserSummary, error)
	UpdateUser(ctx context.Context, scope, idempotencyKey, fingerprint string, record UserUpdateRecord) (controlplane.UserSummary, error)
	ResetUserPassword(ctx context.Context, scope, idempotencyKey, fingerprint, userID string, passwordHash []byte) (controlplane.UserSummary, error)
	DisableUser(ctx context.Context, scope, idempotencyKey, fingerprint, userID string) (controlplane.UserSummary, error)
}

// UserCredentialReader reads one normalized user row and its bcrypt hash for
// the authentication boundary. The hash is transient process data only; it
// must never be serialized, logged, or returned by an HTTP handler.
type UserCredentialReader interface {
	GetUserCredential(ctx context.Context, username string) (controlplane.UserSummary, []byte, error)
}

// UserReader provides a bounded normalized user lookup for authenticated
// session refresh and ownership checks.
type UserReader interface {
	GetUserByID(ctx context.Context, userID string) (controlplane.UserSummary, error)
}

// DeviceReader owns normalized device detail/profile reads. The service keeps
// ownership and role-derived permissions at its boundary; this interface only
// returns the persisted device and user-owned selection.
type DeviceReader interface {
	GetDevice(ctx context.Context, deviceID string) (controlplane.DeviceSummary, error)
	GetOwnedDevice(ctx context.Context, userID, deviceID string) (controlplane.DeviceSummary, error)
}

// ActivationExpiryReader reads the effective expiry of the activation bound
// to one device. It is supplementary profile data and never returns the code.
type ActivationExpiryReader interface {
	GetActivationExpiry(ctx context.Context, userID, deviceID string) (*time.Time, error)
}

// AdminCredentialRepository owns bootstrap/readiness/password-rotation facts
// for the fixed local administrator in normalized PostgreSQL mode.
type AdminCredentialRepository interface {
	EnsureConfiguredAdmin(ctx context.Context, username string, passwordHash []byte) error
	CheckAdminReady(ctx context.Context) error
	ChangeLocalAdminPassword(ctx context.Context, scope, idempotencyKey, fingerprint string, passwordHash []byte) (controlplane.UserSummary, error)
}

// UserAuthorizationRepository owns normalized policy writes separately from
// user identity mutations. The policy input is already canonicalized by the
// service boundary and must not contain secrets.
type UserAuthorizationRepository interface {
	UpdateUserAuthorization(ctx context.Context, scope, idempotencyKey, fingerprint, userID string, input controlplane.UpdateUserAuthorizationInput) (controlplane.UserAuthorizationPolicy, error)
}

// UserAuthorizationSummaryReader reads the bounded administrator summary from
// normalized tables without loading the legacy control-plane snapshot.
type UserAuthorizationSummaryReader interface {
	GetUserAuthorizationSummary(ctx context.Context, userID string) (controlplane.UserAuthorizationSummary, error)
}

// ActivationRepository owns normalized activation-code creation. PlainCode
// is transient input/output only; implementations must persist only its hash.
type ActivationRepository interface {
	CreateActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint string, record ActivationCodeCreateRecord) (controlplane.ActivationCode, error)
	RevokeActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint, codeID string) (controlplane.ActivationCode, error)
}

// ActivationPageReader provides bounded, redacted activation-code reads.
// Normalized PostgreSQL derives expiry at read time without rewriting the
// legacy snapshot or storing plaintext codes.
type ActivationPageReader interface {
	ListActivationCodesPage(ctx context.Context, offset, limit int) (ActivationCodePage, error)
}

type UserCreateRecord struct {
	Username     string
	Role         string
	PasswordHash []byte
	CreatedAt    time.Time
}

type UserUpdateRecord struct {
	UserID   string
	Username *string
	Role     *string
	Status   *string
}

type ActivationCodeCreateRecord struct {
	Product    controlplane.ProductCode
	PlainCode  string
	CodeHash   string
	CodePrefix string
	ExpiresAt  time.Time
	MaxDevices int
	CreatedAt  time.Time
}

type UserPage struct {
	Items []controlplane.UserSummary
	Total int
}

type DevicePage struct {
	Items []controlplane.DeviceSummary
	Total int
}

type ActivationCodePage struct {
	Items []controlplane.ActivationCode
	Total int
}

type ModelUsagePage struct {
	Items []controlplane.ModelUsageRecord
	Total int
}

type AuditPage struct {
	Items []controlplane.AuditLog
	Total int
}

type AuditLogPageOptions struct {
	Offset        int
	Limit         int
	ActorUserID   string
	DeviceID      string
	Action        string
	TargetType    string
	Outcome       string
	ErrorCode     string
	RequestID     string
	CreatedAfter  *time.Time
	CreatedBefore *time.Time
	Sort          string
}

type ModelPoolPage struct {
	Items []controlplane.ModelPoolAccountSummary
	Total int
}

type ModelLeasePage struct {
	Items []controlplane.ModelLeaseAdminSummary
	Total int
}

type ModelLeasePageOptions struct {
	Offset    int
	Limit     int
	Status    string
	Provider  string
	Model     string
	UserID    string
	DeviceID  string
	AccountID string
	Sort      string
}

type ModelUsagePageReader interface {
	ListModelUsagePage(ctx context.Context, offset, limit int) (ModelUsagePage, error)
}

type ModelUsageFilteredPageReader interface {
	ListModelUsagePageWithOptions(ctx context.Context, options ModelUsagePageOptions) (ModelUsagePage, error)
}

type AuditPageReader interface {
	ListAuditLogsPage(ctx context.Context, offset, limit int) (AuditPage, error)
}

type AuditFilteredPageReader interface {
	ListAuditLogsPageWithOptions(ctx context.Context, options AuditLogPageOptions) (AuditPage, error)
}

type ModelPoolPageReader interface {
	ListModelPoolAccountsPage(ctx context.Context, offset, limit int) (ModelPoolPage, error)
}

// ModelPoolHealthPageReader returns only accounts that are currently eligible
// for a bounded health probe. It must apply status, cooldown, secret and daily
// usage predicates in the database so an ineligible prefix cannot hide later
// probe candidates.
type ModelPoolHealthPageReader interface {
	ListModelPoolHealthAccounts(ctx context.Context, limit int) ([]controlplane.ModelPoolAccountSummary, error)
}

// ErrNormalizedModelPoolPageReaderRequired prevents normalized background
// workers from silently materializing the legacy control-plane snapshot.
var ErrNormalizedModelPoolPageReaderRequired = errors.New("normalized model pool page reader is required")

var ErrNormalizedModelPoolHealthPageReaderRequired = errors.New("normalized model pool health page reader is required")

// The normalized service boundary must never materialize the legacy state
// snapshot merely because a bounded page reader is missing. These errors make
// an incomplete cutover explicit and fail closed at the service boundary.
var (
	ErrNormalizedUserPageReaderRequired                  = errors.New("normalized user page reader is required")
	ErrNormalizedActivationPageReaderRequired            = errors.New("normalized activation page reader is required")
	ErrNormalizedAuditPageReaderRequired                 = errors.New("normalized audit page reader is required")
	ErrNormalizedModelLeasePageReaderRequired            = errors.New("normalized model lease page reader is required")
	ErrNormalizedModelUsagePageReaderRequired            = errors.New("normalized model usage page reader is required")
	ErrNormalizedTransactionalDeviceActivatorRequired    = errors.New("normalized transactional device activator is required")
	ErrNormalizedTransactionalHeartbeatRecorderRequired  = errors.New("normalized transactional heartbeat recorder is required")
	ErrNormalizedDeviceLifecycleRepositoryRequired       = errors.New("normalized device lifecycle repository is required")
	ErrNormalizedModelLeaseCreatorRequired               = errors.New("normalized model lease creator is required")
	ErrNormalizedModelLeaseRepositoryRequired            = errors.New("normalized model lease repository is required")
	ErrNormalizedModelPoolAccountCreatorRequired         = errors.New("normalized model pool account creator is required")
	ErrNormalizedModelPoolRepositoryRequired             = errors.New("normalized model pool repository is required")
	ErrNormalizedActivationRepositoryRequired            = errors.New("normalized activation repository is required")
	ErrNormalizedModelUsageRepositoryRequired            = errors.New("normalized model usage repository is required")
	ErrNormalizedAuditRepositoryRequired                 = errors.New("normalized audit repository is required")
	ErrNormalizedUserAuthorizationRepositoryRequired     = errors.New("normalized user authorization repository is required")
	ErrNormalizedLocalAdminBootstrapRequired             = errors.New("normalized local admin must use configured bootstrap")
	ErrNormalizedModelPoolSecretRotationPreparerRequired = errors.New("normalized model pool secret rotation preparer is required")
	ErrNormalizedModelPoolSecretRotatorRequired          = errors.New("normalized model pool secret rotator is required")
)

type ModelLeasePageReader interface {
	ListModelLeasesPage(ctx context.Context, offset, limit int) (ModelLeasePage, error)
}

type ModelLeaseFilteredPageReader interface {
	ListModelLeasesPageWithOptions(ctx context.Context, options ModelLeasePageOptions) (ModelLeasePage, error)
}

// ModelLeaseDetailReader reads one redacted lease detail directly from the
// normalized table. Normalized services must not fall back to StateOperation
// for this administrative read, because that would materialize the legacy
// control-plane snapshot.
type ModelLeaseDetailReader interface {
	GetModelLeaseAdminDetail(ctx context.Context, leaseID string) (controlplane.ModelLeaseAdminDetail, error)
}

var ErrNormalizedModelLeaseDetailReaderRequired = errors.New("normalized model lease detail reader is required")

var ErrNormalizedStagedSecretCleanerRequired = errors.New("normalized staged secret cleaner is required")

// NormalizedReadSource lets services avoid using normalized-only projections
// when a PostgreSQL repository is still serving the compatibility snapshot.
type NormalizedReadSource interface {
	UsesNormalizedReadSource() bool
}

// OperationalMetrics is a low-cardinality control-plane snapshot intended for
// Prometheus. It deliberately excludes user, account and provider IDs.
type OperationalMetrics struct {
	ModelAccountStatusCounts            map[string]int64
	ActiveModelLeases                   int64
	DailyModelUsageTokens               int64
	ConfiguredUserAuthorizationPolicies int64
}

// OperationalMetricsReader is implemented by repositories that can expose
// bounded, aggregate operational state without returning business payloads.
type OperationalMetricsReader interface {
	ReadOperationalMetrics(ctx context.Context) (OperationalMetrics, error)
}

func validatePageWindow(offset, limit int) error {
	if offset < 0 || limit < 1 || limit > 200 {
		return errors.New("page window is out of range")
	}
	return nil
}

func pageWindow(total, offset, limit int) (int, int) {
	if offset >= total {
		return total, total
	}
	end := offset + limit
	if end < offset || end > total {
		end = total
	}
	return offset, end
}
