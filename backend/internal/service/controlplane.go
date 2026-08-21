package service

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"regexp"
	"slices"
	"strings"
	"time"

	"golang.org/x/crypto/bcrypt"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

var (
	idPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{7,63}$`)
)

type ControlPlane struct {
	repository        store.Repository
	secretStore       store.SecretStore
	httpClient        *http.Client
	modelPoolResolver modelPoolIPResolver
	allowInsecureHTTP bool
}

// ControlPlaneOptions controls outbound behavior that must be explicit at the
// application boundary. Insecure HTTP is disabled by default and is intended
// only for controlled development or test environments.
type ControlPlaneOptions struct {
	AllowInsecureHTTP bool
}

const deviceOnlineThreshold = 2 * time.Minute

const modelAccountCooldownDuration = 5 * time.Minute

const secretRotationCleanupTimeout = 2 * time.Second

func NewControlPlane(memory *store.MemoryStore) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(memory, &http.Client{Timeout: 30 * time.Second}, store.NewMemorySecretStore())
}

func NewControlPlaneWithHTTPClient(memory *store.MemoryStore, client *http.Client) *ControlPlane {
	return newControlPlaneWithRepositoryAndSecretStoreOptions(memory, client, store.NewMemorySecretStore(), nil, ControlPlaneOptions{})
}

func NewControlPlaneWithRepository(repository store.Repository) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(repository, &http.Client{Timeout: 30 * time.Second}, store.NewMemorySecretStore())
}

func NewControlPlaneWithRepositoryAndHTTPClient(repository store.Repository, client *http.Client) *ControlPlane {
	return newControlPlaneWithRepositoryAndSecretStoreOptions(repository, client, store.NewMemorySecretStore(), nil, ControlPlaneOptions{})
}

func NewControlPlaneWithRepositoryAndSecretStore(repository store.Repository, client *http.Client, secretStore store.SecretStore) *ControlPlane {
	return newControlPlaneWithRepositoryAndSecretStoreOptions(repository, client, secretStore, nil, ControlPlaneOptions{})
}

func newControlPlaneWithRepositoryAndSecretStore(repository store.Repository, client *http.Client, secretStore store.SecretStore, resolver modelPoolIPResolver) *ControlPlane {
	// This unexported constructor is retained for legacy in-package HTTP tests;
	// production callers must use the exported options constructor, which keeps
	// insecure HTTP disabled by default.
	return newControlPlaneWithRepositoryAndSecretStoreOptions(repository, client, secretStore, resolver, ControlPlaneOptions{AllowInsecureHTTP: true})
}

func NewControlPlaneWithRepositoryAndSecretStoreAndOptions(repository store.Repository, client *http.Client, secretStore store.SecretStore, options ControlPlaneOptions) *ControlPlane {
	return newControlPlaneWithRepositoryAndSecretStoreOptions(repository, client, secretStore, nil, options)
}

func newControlPlaneWithRepositoryAndSecretStoreOptions(repository store.Repository, client *http.Client, secretStore store.SecretStore, resolver modelPoolIPResolver, options ControlPlaneOptions) *ControlPlane {
	if repository == nil {
		repository = store.NewMemoryStore(time.Now)
	}
	if secretStore == nil {
		secretStore = store.NewMemorySecretStore()
	}
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}
	if resolver == nil {
		resolver = defaultModelPoolIPResolver
	}
	return &ControlPlane{repository: repository, secretStore: secretStore, httpClient: client, modelPoolResolver: resolver, allowInsecureHTTP: options.AllowInsecureHTTP}
}

func (s *ControlPlane) EnsureLocalAdmin(ctx context.Context, username string) error {
	if err := checkContext(ctx); err != nil {
		return err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		// Normalized PostgreSQL bootstraps the administrator with a persisted
		// credential. The legacy user-only helper must not materialize a snapshot.
		return store.ErrNormalizedLocalAdminBootstrapRequired
	}
	if strings.TrimSpace(username) == "" {
		username = "admin"
	}

	return s.repository.Run(ctx, func(state *store.State) error {
		if _, exists := state.Users["usr_local_admin"]; exists {
			return nil
		}

		now := s.repository.Now().Format(time.RFC3339)
		state.Users["usr_local_admin"] = controlplane.UserSummary{
			ID:        "usr_local_admin",
			Username:  username,
			Role:      controlplane.RoleAdmin,
			Status:    controlplane.UserStatusActive,
			CreatedAt: now,
		}
		return nil
	})
}

// EnsureConfiguredAdmin creates the bootstrap administrator with a persisted
// bcrypt credential. Existing credentials are never overwritten by a process
// restart; password changes must go through an explicit control-plane command.
func (s *ControlPlane) EnsureConfiguredAdmin(ctx context.Context, username, password string) error {
	if err := checkContext(ctx); err != nil {
		return err
	}
	username = strings.TrimSpace(username)
	if username == "" || len(username) > 64 || len(password) < 8 || len(password) > 256 {
		return errors.New("configured administrator credentials are invalid")
	}
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
	if err != nil {
		return fmt.Errorf("hash configured administrator password: %w", err)
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		bootstrap, ok := s.repository.(store.AdminCredentialRepository)
		if !ok {
			return store.ErrNormalizedAdminCredentialRepositoryRequired
		}
		return bootstrap.EnsureConfiguredAdmin(ctx, username, passwordHash)
	}

	return s.repository.Run(ctx, func(state *store.State) error {
		admin, exists := state.Users["usr_local_admin"]
		if exists {
			if admin.Username != username || admin.Role != controlplane.RoleAdmin {
				return errors.New("persisted local administrator does not match configured identity")
			}
			if hash := state.UserCredentialHashes[admin.ID]; len(hash) > 0 {
				return nil
			}
			state.UserCredentialHashes[admin.ID] = append([]byte(nil), passwordHash...)
			return nil
		}
		for _, user := range state.Users {
			if user.Username == username {
				return controlplane.ErrUsernameAlreadyExists
			}
		}
		now := s.repository.Now().Format(time.RFC3339)
		state.Users["usr_local_admin"] = controlplane.UserSummary{
			ID:        "usr_local_admin",
			Username:  username,
			Role:      controlplane.RoleAdmin,
			Status:    controlplane.UserStatusActive,
			CreatedAt: now,
		}
		state.UserCredentialHashes["usr_local_admin"] = append([]byte(nil), passwordHash...)
		return nil
	})
}

// ChangeLocalAdminPassword rotates the persisted bootstrap administrator
// credential. It is intentionally separate from user-management password
// resets so the local administrator cannot be changed through a generic user
// route.
func (s *ControlPlane) ChangeLocalAdminPassword(ctx context.Context, idempotencyKey string, input controlplane.ChangeLocalAdminPasswordInput) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) || len(input.Password) < 12 || len(input.Password) > 256 {
		return controlplane.UserSummary{}, controlplane.ErrInvalidRequest
	}
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(input.Password), bcrypt.DefaultCost)
	if err != nil {
		return controlplane.UserSummary{}, fmt.Errorf("hash local administrator password: %w", err)
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		bootstrap, ok := s.repository.(store.AdminCredentialRepository)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedAdminCredentialRepositoryRequired
		}
		return bootstrap.ChangeLocalAdminPassword(ctx, "control-plane-state", "change-local-admin-password:"+idempotencyKey, fingerprint, passwordHash)
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		admin, ok := state.Users["usr_local_admin"]
		if !ok || admin.Role != controlplane.RoleAdmin {
			return controlplane.UserSummary{}, controlplane.ErrLocalAdminRequired
		}
		if admin.Status != controlplane.UserStatusActive {
			return controlplane.UserSummary{}, controlplane.ErrUserDisabled
		}
		scope := "change-local-admin-password:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Users[existing.ResourceID], nil
		}
		state.UserCredentialHashes[admin.ID] = append([]byte(nil), passwordHash...)
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: admin.ID}
		return admin, nil
	})
}

func (s *ControlPlane) CheckReady(ctx context.Context) error {
	if err := checkContext(ctx); err != nil {
		return err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		bootstrap, ok := s.repository.(store.AdminCredentialRepository)
		if !ok {
			return store.ErrNormalizedAdminCredentialRepositoryRequired
		}
		if err := bootstrap.CheckAdminReady(ctx); err != nil {
			return fmt.Errorf("repository readiness failed: %w", err)
		}
	} else if err := s.repository.Run(ctx, func(state *store.State) error {
		admin, exists := state.Users["usr_local_admin"]
		if !exists {
			return errors.New("local admin is not initialized")
		}
		if admin.Role != controlplane.RoleAdmin || admin.Status != controlplane.UserStatusActive || len(state.UserCredentialHashes[admin.ID]) == 0 {
			return errors.New("local admin credential is not initialized")
		}
		return nil
	}); err != nil {
		return fmt.Errorf("repository readiness failed: %w", err)
	}
	if err := s.secretStore.Ping(ctx); err != nil {
		return fmt.Errorf("secret store readiness failed: %w", err)
	}
	return nil
}

// TryAdvisoryLock exposes the repository coordination boundary to owned
// background workers without making service code depend on PostgreSQL APIs.
// Memory and other single-process repositories intentionally use a no-op lock.
func (s *ControlPlane) TryAdvisoryLock(ctx context.Context, key int64) (store.AdvisoryLockRelease, bool, error) {
	if err := checkContext(ctx); err != nil {
		return nil, false, err
	}
	if locker, ok := s.repository.(store.AdvisoryLocker); ok {
		return locker.TryAdvisoryLock(ctx, key)
	}
	return func(context.Context) error { return nil }, true, nil
}

// CleanupStagedSecrets reconciles durable rotation candidates after a process
// crash. The active SecretRef set is read from the business repository first;
// the secret store only deletes old references matching its staged prefix and
// never infers the active account state itself.
func (s *ControlPlane) CleanupStagedSecrets(ctx context.Context, request store.RetentionCleanupRequest) (int64, error) {
	if err := checkContext(ctx); err != nil {
		return 0, err
	}
	if err := request.Validate(); err != nil {
		return 0, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		cleaner, ok := s.repository.(store.NormalizedStagedSecretCleaner)
		if !ok {
			return 0, store.ErrNormalizedStagedSecretCleanerRequired
		}
		return cleaner.CleanupUnreferencedStagedSecrets(ctx, request)
	}
	protected, err := withState(ctx, s.repository, func(state *store.State) ([]string, error) {
		refs := make([]string, 0, len(state.ModelPoolAccounts))
		seen := make(map[string]struct{}, len(state.ModelPoolAccounts))
		for _, account := range state.ModelPoolAccounts {
			ref := strings.TrimSpace(account.SecretRef)
			if ref == "" {
				continue
			}
			if _, exists := seen[ref]; exists {
				continue
			}
			seen[ref] = struct{}{}
			refs = append(refs, ref)
		}
		slices.Sort(refs)
		return refs, nil
	})
	if err != nil {
		return 0, err
	}
	var deleted int64
	if cleaner, ok := s.secretStore.(store.StagedSecretCleaner); ok {
		deleted, err = cleaner.CleanupStagedSecrets(ctx, request, protected)
		if err != nil {
			return deleted, err
		}
	}
	referenceCleaner, ok := s.secretStore.(store.SecretReferenceCleaner)
	pending, err := withState(ctx, s.repository, func(state *store.State) ([]string, error) {
		refs := make([]string, 0, len(state.PendingSecretCleanup))
		for reference, queuedAt := range state.PendingSecretCleanup {
			if queuedAt.IsZero() || !queuedAt.Before(request.Cutoff) {
				continue
			}
			if _, isProtected := slices.BinarySearch(protected, reference); isProtected {
				continue
			}
			refs = append(refs, reference)
		}
		slices.Sort(refs)
		if len(refs) > request.BatchSize {
			refs = refs[:request.BatchSize]
		}
		return refs, nil
	})
	if err != nil || len(pending) == 0 {
		return deleted, err
	}
	if !ok {
		return deleted, controlplane.ErrSecretStoreUnavailable
	}
	resolved, err := referenceCleaner.CleanupSecretReferences(ctx, request, pending, protected)
	if err != nil {
		return deleted, err
	}
	resolvedSet := make(map[string]struct{}, len(resolved))
	for _, reference := range resolved {
		resolvedSet[reference] = struct{}{}
	}
	for _, reference := range pending {
		if _, exists := resolvedSet[reference]; exists {
			continue
		}
		if _, getErr := s.secretStore.Get(ctx, reference); errors.Is(getErr, store.ErrSecretNotFound) {
			resolved = append(resolved, reference)
			resolvedSet[reference] = struct{}{}
		} else if getErr != nil {
			return deleted, getErr
		}
	}
	if len(resolved) == 0 {
		return deleted, nil
	}
	if err := s.repository.Run(ctx, func(state *store.State) error {
		for _, reference := range resolved {
			queuedAt, exists := state.PendingSecretCleanup[reference]
			if exists && queuedAt.Before(request.Cutoff) {
				delete(state.PendingSecretCleanup, reference)
			}
		}
		return nil
	}); err != nil {
		return deleted, err
	}
	return deleted + int64(len(resolved)), nil
}

func withState[T any](ctx context.Context, repository store.Repository, fn func(state *store.State) (T, error)) (T, error) {
	var result T
	err := repository.Run(ctx, func(state *store.State) error {
		var err error
		result, err = fn(state)
		return err
	})
	return result, err
}

func runDeviceState(run func(store.StateOperation) error, fn func(*store.State) (controlplane.DeviceSummary, error)) (controlplane.DeviceSummary, error) {
	if run == nil {
		return controlplane.DeviceSummary{}, errors.New("device state runner is required")
	}
	var result controlplane.DeviceSummary
	err := run(func(state *store.State) error {
		var operationErr error
		result, operationErr = fn(state)
		return operationErr
	})
	return result, err
}

func (s *ControlPlane) RecordAudit(ctx context.Context, input controlplane.AuditLogInput) error {
	if err := checkContext(ctx); err != nil {
		return err
	}
	input.ActorUserID = strings.TrimSpace(input.ActorUserID)
	input.DeviceID = strings.TrimSpace(input.DeviceID)
	input.Action = strings.TrimSpace(input.Action)
	input.TargetType = strings.TrimSpace(input.TargetType)
	input.TargetID = strings.TrimSpace(input.TargetID)
	input.Outcome = strings.TrimSpace(input.Outcome)
	input.ErrorCode = strings.TrimSpace(input.ErrorCode)
	input.RequestID = strings.TrimSpace(input.RequestID)
	if input.Outcome == "" {
		input.Outcome = "unknown"
	}
	if input.Action == "" || input.TargetType == "" || len(input.Action) > 512 || len(input.TargetType) > 128 || len(input.TargetID) > 128 || len(input.ErrorCode) > 128 || len(input.RequestID) > 128 || input.StatusCode < 0 || input.StatusCode > 599 {
		return controlplane.ErrInvalidRequest
	}
	if input.Outcome != "success" && input.Outcome != "failure" && input.Outcome != "unknown" {
		return controlplane.ErrInvalidRequest
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		if writer, ok := s.repository.(store.AuditOutboxRepository); ok {
			return writer.RecordAuditWithOutbox(ctx, input)
		}
		if writer, ok := s.repository.(store.AuditRepository); ok {
			return writer.RecordAudit(ctx, input)
		}
		return store.ErrNormalizedAuditRepositoryRequired
	}
	return s.repository.Run(ctx, func(state *store.State) error {
		id := nextID(state, "audit")
		state.AuditLogs[id] = controlplane.AuditLog{
			ID:          id,
			ActorUserID: input.ActorUserID,
			DeviceID:    input.DeviceID,
			Action:      input.Action,
			TargetType:  input.TargetType,
			TargetID:    input.TargetID,
			Outcome:     input.Outcome,
			StatusCode:  input.StatusCode,
			ErrorCode:   input.ErrorCode,
			RequestID:   input.RequestID,
			CreatedAt:   s.repository.Now().Format(time.RFC3339),
		}
		return nil
	})
}

// DispatchAuditOutbox performs one bounded durable audit delivery pass. The
// method is intentionally a no-op boundary for snapshot-backed repositories;
// production normalized repositories must implement the Outbox interface.
func (s *ControlPlane) DispatchAuditOutbox(ctx context.Context, batchSize int) (int64, error) {
	if err := checkContext(ctx); err != nil {
		return 0, err
	}
	if dispatcher, ok := s.repository.(store.AuditOutboxRepository); ok {
		return dispatcher.DispatchAuditOutbox(ctx, batchSize)
	}
	return 0, store.ErrNormalizedRetentionCleanupRequired
}

func (s *ControlPlane) ListAuditLogs(ctx context.Context) ([]controlplane.AuditLog, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedAuditPageReaderRequired
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.AuditLog, error) {
		items := make([]controlplane.AuditLog, 0, len(state.AuditLogs))
		for _, item := range state.AuditLogs {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.AuditLog) int {
			if a.CreatedAt != b.CreatedAt {
				return strings.Compare(b.CreatedAt, a.CreatedAt)
			}
			return strings.Compare(b.ID, a.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) ListAuditLogsPage(ctx context.Context, page, pageSize int) ([]controlplane.AuditLog, int, error) {
	return s.ListAuditLogsPageWithOptions(ctx, page, pageSize, AuditLogListOptions{})
}

func (s *ControlPlane) ListAuditLogsPageWithOptions(ctx context.Context, page, pageSize int, options AuditLogListOptions) ([]controlplane.AuditLog, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	storageOptions, err := normalizeAuditLogListOptions(options)
	if err != nil {
		return nil, 0, err
	}
	storageOptions.Offset = offset
	storageOptions.Limit = pageSize
	if reader, ok := s.repository.(store.AuditFilteredPageReader); ok {
		result, err := reader.ListAuditLogsPageWithOptions(ctx, storageOptions)
		if err != nil {
			return nil, 0, err
		}
		return result.Items, result.Total, nil
	}
	if reader, ok := s.repository.(store.AuditPageReader); ok {
		if storageOptions.ActorUserID != "" || storageOptions.DeviceID != "" || storageOptions.Action != "" || storageOptions.TargetType != "" || storageOptions.Outcome != "" || storageOptions.ErrorCode != "" || storageOptions.RequestID != "" || storageOptions.CreatedAfter != nil || storageOptions.CreatedBefore != nil || storageOptions.Sort != "" {
			return nil, 0, controlplane.ErrInvalidRequest
		}
		result, err := reader.ListAuditLogsPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		return result.Items, result.Total, nil
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, 0, store.ErrNormalizedAuditPageReaderRequired
	}
	result, err := withState(ctx, s.repository, func(state *store.State) ([]controlplane.AuditLog, error) {
		items := make([]controlplane.AuditLog, 0, len(state.AuditLogs))
		for _, item := range state.AuditLogs {
			if auditLogMatchesListOptions(item, storageOptions) {
				items = append(items, item)
			}
		}
		sortAuditLogsForList(items, storageOptions.Sort)
		return items, nil
	})
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(result), offset, pageSize)
	return result[start:end], len(result), nil
}

func auditLogMatchesListOptions(item controlplane.AuditLog, options store.AuditLogPageOptions) bool {
	if options.ActorUserID != "" && item.ActorUserID != options.ActorUserID {
		return false
	}
	if options.DeviceID != "" && item.DeviceID != options.DeviceID {
		return false
	}
	if options.Action != "" && item.Action != options.Action {
		return false
	}
	if options.TargetType != "" && item.TargetType != options.TargetType {
		return false
	}
	if options.Outcome != "" && item.Outcome != options.Outcome {
		return false
	}
	if options.ErrorCode != "" && item.ErrorCode != options.ErrorCode {
		return false
	}
	if options.RequestID != "" && item.RequestID != options.RequestID {
		return false
	}
	if options.CreatedAfter == nil && options.CreatedBefore == nil {
		return true
	}
	createdAt, err := time.Parse(time.RFC3339, item.CreatedAt)
	if err != nil {
		return false
	}
	if options.CreatedAfter != nil && createdAt.Before(*options.CreatedAfter) {
		return false
	}
	return options.CreatedBefore == nil || !createdAt.After(*options.CreatedBefore)
}

func sortAuditLogsForList(items []controlplane.AuditLog, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.AuditLog) int {
		if a.CreatedAt != b.CreatedAt {
			createdAtOrder := store.CompareAuditLogCreatedAt(a.CreatedAt, b.CreatedAt)
			if sortKey == store.AuditLogSortCreatedAsc {
				return createdAtOrder
			}
			return -createdAtOrder
		}
		if sortKey == store.AuditLogSortCreatedAsc {
			return strings.Compare(a.ID, b.ID)
		}
		return strings.Compare(b.ID, a.ID)
	})
}

func (s *ControlPlane) ListUsers(ctx context.Context) ([]controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedUserPageReaderRequired
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.UserSummary, error) {
		items := make([]controlplane.UserSummary, 0, len(state.Users))
		for _, item := range state.Users {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.UserSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

// ListUsersPage keeps the HTTP page boundary close to the repository. A
// normalized PostgreSQL repository can execute LIMIT/OFFSET without loading
// the full control-plane snapshot; compatibility repositories retain the
// existing bounded in-memory fallback.
func (s *ControlPlane) ListUsersPage(ctx context.Context, page, pageSize int) ([]controlplane.UserSummary, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	if reader, ok := s.repository.(store.UserPageReader); ok {
		result, err := reader.ListUsersPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		return result.Items, result.Total, nil
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, 0, store.ErrNormalizedUserPageReaderRequired
	}
	items, err := s.ListUsers(ctx)
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(items), offset, pageSize)
	return items[start:end], len(items), nil
}

func (s *ControlPlane) CreateUser(ctx context.Context, idempotencyKey string, input controlplane.CreateUserInput) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	input.Username = strings.TrimSpace(input.Username)
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateUserInput(input); err != nil {
		return controlplane.UserSummary{}, err
	}
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(input.Password), bcrypt.DefaultCost)
	if err != nil {
		return controlplane.UserSummary{}, fmt.Errorf("hash user password: %w", err)
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.UserRepository)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedUserRepositoryRequired
		}
		return writer.CreateUser(ctx, "control-plane-state", "create-user:"+idempotencyKey, fingerprint, store.UserCreateRecord{
			Username:     input.Username,
			Role:         input.Role,
			PasswordHash: append([]byte(nil), passwordHash...),
			CreatedAt:    s.repository.Now(),
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		if existing, ok := state.IdempotencyRecords["create-user:"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Users[existing.ResourceID], nil
		}

		for _, user := range state.Users {
			if user.Username == input.Username {
				return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
			}
		}

		user := controlplane.UserSummary{
			ID:        nextID(state, "usr"),
			Username:  strings.TrimSpace(input.Username),
			Role:      input.Role,
			Status:    controlplane.UserStatusActive,
			CreatedAt: s.repository.Now().Format(time.RFC3339),
		}
		state.Users[user.ID] = user
		state.UserCredentialHashes[user.ID] = append([]byte(nil), passwordHash...)
		state.IdempotencyRecords["create-user:"+idempotencyKey] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  user.ID,
		}
		return user, nil
	})
}

func (s *ControlPlane) AuthenticateUser(ctx context.Context, username, password string) (controlplane.Actor, controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.Actor{}, controlplane.UserSummary{}, err
	}
	username = strings.TrimSpace(username)
	if username == "" || password == "" {
		return controlplane.Actor{}, controlplane.UserSummary{}, controlplane.ErrUnauthenticated
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.UserCredentialReader)
		if !ok {
			return controlplane.Actor{}, controlplane.UserSummary{}, store.ErrNormalizedUserCredentialReaderRequired
		}
		user, passwordHash, err := reader.GetUserCredential(ctx, username)
		if err != nil {
			return controlplane.Actor{}, controlplane.UserSummary{}, err
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.Actor{}, controlplane.UserSummary{}, controlplane.ErrUserDisabled
		}
		if bcrypt.CompareHashAndPassword(passwordHash, []byte(password)) != nil {
			return controlplane.Actor{}, controlplane.UserSummary{}, controlplane.ErrUnauthenticated
		}
		return controlplane.Actor{UserID: user.ID, Role: user.Role}, user, nil
	}

	result, err := withState(ctx, s.repository, func(state *store.State) (struct {
		Actor controlplane.Actor
		User  controlplane.UserSummary
	}, error) {
		for _, user := range state.Users {
			if user.Username != username {
				continue
			}
			if user.Status != controlplane.UserStatusActive {
				return struct {
					Actor controlplane.Actor
					User  controlplane.UserSummary
				}{}, controlplane.ErrUserDisabled
			}
			hash := state.UserCredentialHashes[user.ID]
			if bcrypt.CompareHashAndPassword(hash, []byte(password)) != nil {
				return struct {
					Actor controlplane.Actor
					User  controlplane.UserSummary
				}{}, controlplane.ErrUnauthenticated
			}
			return struct {
				Actor controlplane.Actor
				User  controlplane.UserSummary
			}{
				Actor: controlplane.Actor{UserID: user.ID, Role: user.Role},
				User:  user,
			}, nil
		}
		return struct {
			Actor controlplane.Actor
			User  controlplane.UserSummary
		}{}, controlplane.ErrUnauthenticated
	})
	if err != nil {
		return controlplane.Actor{}, controlplane.UserSummary{}, err
	}
	return result.Actor, result.User, nil
}

func (s *ControlPlane) DisableUser(ctx context.Context, idempotencyKey, userID string) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserSummary{}, controlplane.ErrUserNotFound
	}
	fingerprint, err := fingerprintValue(struct {
		UserID string `json:"user_id"`
	}{UserID: userID})
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.UserRepository)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedUserRepositoryRequired
		}
		return writer.DisableUser(ctx, "control-plane-state", "disable-user:"+userID+":"+idempotencyKey, fingerprint, userID)
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		if user.ID == "usr_local_admin" {
			return controlplane.UserSummary{}, controlplane.ErrCannotDisableLocalAdmin
		}
		scope := "disable-user:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return user, nil
		}
		if user.Status == controlplane.UserStatusDisabled {
			state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
			return user, nil
		}
		candidate := user
		candidate.Status = controlplane.UserStatusDisabled
		if countActiveAdmins(state, userID, candidate) == 0 {
			return controlplane.UserSummary{}, controlplane.ErrLastActiveAdmin
		}
		user.Status = controlplane.UserStatusDisabled
		state.Users[user.ID] = user
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
		return user, nil
	})
}

func (s *ControlPlane) UpdateUser(ctx context.Context, idempotencyKey, userID string, input controlplane.UpdateUserInput) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateUpdateUserInput(&input); err != nil {
		return controlplane.UserSummary{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserSummary{}, controlplane.ErrUserNotFound
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.UserRepository)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedUserRepositoryRequired
		}
		return writer.UpdateUser(ctx, "control-plane-state", "update-user:"+userID+":"+idempotencyKey, fingerprint, store.UserUpdateRecord{
			UserID: userID, Username: input.Username, Role: input.Role, Status: input.Status,
		})
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		if user.ID == "usr_local_admin" {
			return controlplane.UserSummary{}, controlplane.ErrCannotModifyLocalAdmin
		}
		scope := "update-user:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Users[existing.ResourceID], nil
		}
		if input.Username != nil {
			for _, existing := range state.Users {
				if existing.ID != userID && existing.Username == *input.Username {
					return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
				}
			}
			user.Username = *input.Username
		}
		if input.Role != nil {
			user.Role = *input.Role
		}
		if input.Status != nil {
			user.Status = *input.Status
		}
		if user.Role != controlplane.RoleAdmin || user.Status != controlplane.UserStatusActive {
			if countActiveAdmins(state, userID, user) == 0 {
				return controlplane.UserSummary{}, controlplane.ErrLastActiveAdmin
			}
		}
		state.Users[userID] = user
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
		return user, nil
	})
}

func (s *ControlPlane) ResetUserPassword(ctx context.Context, idempotencyKey, userID string, input controlplane.ResetUserPasswordInput) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if len(input.Password) < 8 || len(input.Password) > 256 {
		return controlplane.UserSummary{}, controlplane.ErrInvalidRequest
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserSummary{}, controlplane.ErrUserNotFound
	}
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(input.Password), bcrypt.DefaultCost)
	if err != nil {
		return controlplane.UserSummary{}, fmt.Errorf("hash user password: %w", err)
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.UserRepository)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedUserRepositoryRequired
		}
		return writer.ResetUserPassword(ctx, "control-plane-state", "reset-user-password:"+userID+":"+idempotencyKey, fingerprint, userID, append([]byte(nil), passwordHash...))
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		if user.ID == "usr_local_admin" {
			return controlplane.UserSummary{}, controlplane.ErrCannotModifyLocalAdmin
		}
		scope := "reset-user-password:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Users[existing.ResourceID], nil
		}
		state.UserCredentialHashes[userID] = append([]byte(nil), passwordHash...)
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
		return user, nil
	})
}

func (s *ControlPlane) GetUser(ctx context.Context, userID string) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.UserReader)
		if !ok {
			return controlplane.UserSummary{}, store.ErrNormalizedUserReaderRequired
		}
		return reader.GetUserByID(ctx, userID)
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		return user, nil
	})
}

func (s *ControlPlane) ListDevices(ctx context.Context) ([]controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedUserPageReaderRequired
	}
	items, err := withState(ctx, s.repository, func(state *store.State) ([]controlplane.DeviceSummary, error) {
		items := make([]controlplane.DeviceSummary, 0, len(state.Devices))
		for _, item := range state.Devices {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
	if err != nil {
		return nil, err
	}
	now := s.repository.Now()
	for index := range items {
		items[index] = decorateDeviceSummary(items[index], now)
	}
	return items, nil
}

// ListDevicesPage is the bounded counterpart to ListDevices for admin list
// endpoints. Device online state remains derived at the service boundary.
func (s *ControlPlane) ListDevicesPage(ctx context.Context, page, pageSize int) ([]controlplane.DeviceSummary, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	var items []controlplane.DeviceSummary
	var total int
	if reader, ok := s.repository.(store.UserPageReader); ok {
		result, err := reader.ListDevicesPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		items, total = result.Items, result.Total
	} else {
		if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
			return nil, 0, store.ErrNormalizedUserPageReaderRequired
		}
		items, err = s.ListDevices(ctx)
		if err != nil {
			return nil, 0, err
		}
		total = len(items)
		start, end := pageWindow(total, offset, pageSize)
		items = items[start:end]
	}
	now := s.repository.Now()
	for index := range items {
		items[index] = decorateDeviceSummary(items[index], now)
	}
	return items, total, nil
}

func (s *ControlPlane) GetDevice(ctx context.Context, deviceID string) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.DeviceReader)
		if !ok {
			return controlplane.DeviceSummary{}, store.ErrNormalizedDeviceReaderRequired
		}
		device, err := reader.GetDevice(ctx, deviceID)
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		return decorateDeviceSummary(device, s.repository.Now()), nil
	}
	device, err := withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		return device, nil
	})
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	return decorateDeviceSummary(device, s.repository.Now()), nil
}

func (s *ControlPlane) ListDevicesForUser(ctx context.Context, userID string) ([]controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return nil, controlplane.ErrUserNotFound
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedUserPageReaderRequired
	}
	items, err := withState(ctx, s.repository, func(state *store.State) ([]controlplane.DeviceSummary, error) {
		if _, ok := state.Users[userID]; !ok {
			return nil, controlplane.ErrUserNotFound
		}
		items := make([]controlplane.DeviceSummary, 0)
		for _, device := range state.Devices {
			if device.UserID == userID {
				items = append(items, device)
			}
		}
		slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
	if err != nil {
		return nil, err
	}
	now := s.repository.Now()
	for index := range items {
		items[index] = decorateDeviceSummary(items[index], now)
	}
	return items, nil
}

// ListDevicesForUserPage avoids materializing every device for normalized
// PostgreSQL reads while preserving the existing ownership check and derived
// online state.
func (s *ControlPlane) ListDevicesForUserPage(ctx context.Context, userID string, page, pageSize int) ([]controlplane.DeviceSummary, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return nil, 0, controlplane.ErrUserNotFound
	}
	var items []controlplane.DeviceSummary
	var total int
	if reader, ok := s.repository.(store.UserPageReader); ok {
		result, err := reader.ListDevicesForUserPage(ctx, userID, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		items, total = result.Items, result.Total
	} else {
		if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
			return nil, 0, store.ErrNormalizedUserPageReaderRequired
		}
		items, err = s.ListDevicesForUser(ctx, userID)
		if err != nil {
			return nil, 0, err
		}
		total = len(items)
		start, end := pageWindow(total, offset, pageSize)
		items = items[start:end]
	}
	now := s.repository.Now()
	for index := range items {
		items[index] = decorateDeviceSummary(items[index], now)
	}
	return items, total, nil
}

// GetUserAuthorizationSummary returns the current device/lease authorization
// snapshot and soft usage signal for an administrator. Usage is derived from
// client-reported records because no provider-authoritative billing source is
// available in the current product scope.
func (s *ControlPlane) GetUserAuthorizationSummary(ctx context.Context, userID string) (controlplane.UserAuthorizationSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserAuthorizationSummary{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserAuthorizationSummary{}, controlplane.ErrUserNotFound
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.UserAuthorizationSummaryReader)
		if !ok {
			return controlplane.UserAuthorizationSummary{}, store.ErrNormalizedUserAuthorizationSummaryReaderRequired
		}
		return reader.GetUserAuthorizationSummary(ctx, userID)
	}
	now := s.repository.Now()
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserAuthorizationSummary, error) {
		if _, ok := state.Users[userID]; !ok {
			return controlplane.UserAuthorizationSummary{}, controlplane.ErrUserNotFound
		}
		userLeaseIDs := make(map[string]struct{})
		accountIDs := make(map[string]struct{})
		summary := controlplane.UserAuthorizationSummary{
			UserID:              userID,
			UsageSource:         "client_reported_soft",
			HardQuotaConfigured: false,
			AllowedModels:       []string{},
			QuotaEnforcement:    "server_recorded_usage_guard",
			AsOf:                now.Format(time.RFC3339),
		}
		if policy, ok := state.UserAuthorizationPolicies[userID]; ok {
			summary.AllowedModels = append([]string{}, policy.AllowedModels...)
			summary.DailyTokenLimit = policy.DailyTokenLimit
		}
		for _, device := range state.Devices {
			if device.UserID != userID {
				continue
			}
			summary.DeviceCount++
			if device.Status == controlplane.DeviceStatusActive {
				summary.ActiveDeviceCount++
			}
		}
		for id, lease := range state.ModelLeases {
			if lease.UserID != userID {
				continue
			}
			userLeaseIDs[id] = struct{}{}
			if lease.Status != controlplane.ModelLeaseStatusActive {
				continue
			}
			summary.ActiveLeaseCount++
			if lease.AccountID != "" {
				accountIDs[lease.AccountID] = struct{}{}
			}
		}
		summary.ActiveAccountCount = len(accountIDs)
		for _, usage := range state.ModelUsageRecords {
			if _, ok := userLeaseIDs[usage.LeaseID]; !ok {
				continue
			}
			createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
			if err != nil || createdAt.UTC().Format("2006-01-02") != now.UTC().Format("2006-01-02") {
				continue
			}
			summary.DailyUsedTokens += usage.TotalTokens
		}
		return summary, nil
	})
}

// UpdateUserAuthorization stores a bounded allowlist and a server-recorded
// usage guard. The guard is deliberately separate from provider billing: it
// only uses usage records already accepted by this control plane.
func (s *ControlPlane) UpdateUserAuthorization(ctx context.Context, idempotencyKey, userID string, input controlplane.UpdateUserAuthorizationInput) (controlplane.UserAuthorizationPolicy, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserAuthorizationPolicy{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateUpdateUserAuthorizationInput(&input); err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserAuthorizationPolicy{}, controlplane.ErrUserNotFound
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		if writer, ok := s.repository.(store.UserAuthorizationRepository); ok {
			return writer.UpdateUserAuthorization(ctx, "control-plane-state", "update-user-authorization:"+userID+":"+idempotencyKey, fingerprint, userID, input)
		}
		return controlplane.UserAuthorizationPolicy{}, store.ErrNormalizedUserAuthorizationRepositoryRequired
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserAuthorizationPolicy, error) {
		if _, ok := state.Users[userID]; !ok {
			return controlplane.UserAuthorizationPolicy{}, controlplane.ErrUserNotFound
		}
		scope := "update-user-authorization:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserAuthorizationPolicy{}, controlplane.ErrIdempotencyConflict
			}
			return state.UserAuthorizationPolicies[userID], nil
		}
		now := s.repository.Now().Format(time.RFC3339)
		policy := controlplane.UserAuthorizationPolicy{
			UserID:          userID,
			AllowedModels:   append([]string{}, input.AllowedModels...),
			DailyTokenLimit: input.DailyTokenLimit,
			UpdatedAt:       now,
		}
		state.UserAuthorizationPolicies[userID] = policy
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
		return policy, nil
	})
}

func (s *ControlPlane) GetClientProfile(ctx context.Context, userID, deviceID string) (controlplane.ClientProfile, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ClientProfile{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		userReader, ok := s.repository.(store.UserReader)
		if !ok {
			return controlplane.ClientProfile{}, store.ErrNormalizedUserReaderRequired
		}
		deviceReader, ok := s.repository.(store.DeviceReader)
		if !ok {
			return controlplane.ClientProfile{}, store.ErrNormalizedDeviceReaderRequired
		}
		user, err := userReader.GetUserByID(ctx, strings.TrimSpace(userID))
		if err != nil {
			return controlplane.ClientProfile{}, err
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrUserDisabled
		}
		device, err := deviceReader.GetOwnedDevice(ctx, user.ID, strings.TrimSpace(deviceID))
		if err != nil {
			return controlplane.ClientProfile{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrDeviceDisabled
		}
		profile := controlplane.ClientProfile{
			User:        user,
			Device:      decorateDeviceSummary(device, s.repository.Now()),
			Permissions: permissionsForRole(user.Role),
		}
		if reader, ok := s.repository.(store.ActivationExpiryReader); ok {
			expiresAt, err := reader.GetActivationExpiry(ctx, user.ID, device.ID)
			if err != nil {
				return controlplane.ClientProfile{}, err
			}
			profile.Device.ActivationExpiresAt = formatActivationExpiry(expiresAt)
		}
		return profile, nil
	}
	profile, err := withState(ctx, s.repository, func(state *store.State) (controlplane.ClientProfile, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ClientProfile{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrUserDisabled
		}
		device, err := resolveOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ClientProfile{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrDeviceDisabled
		}
		profile := controlplane.ClientProfile{
			User:        user,
			Device:      device,
			Permissions: permissionsForRole(user.Role),
		}
		profile.Device.ActivationExpiresAt = activationExpiryForDevice(state, user.ID, device.ID)
		return profile, nil
	})
	if err != nil {
		return controlplane.ClientProfile{}, err
	}
	profile.Device = decorateDeviceSummary(profile.Device, s.repository.Now())
	return profile, nil
}

func (s *ControlPlane) RecordHeartbeat(ctx context.Context, idempotencyKey, userID string, input controlplane.HeartbeatInput) (controlplane.HeartbeatResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		// This compatibility entry point has no authenticated session hash;
		// normalized callers must use RecordHeartbeatWithSessionBinding.
		return controlplane.HeartbeatResult{}, store.ErrNormalizedTransactionalHeartbeatRecorderRequired
	}
	return s.recordHeartbeatWithRunner(ctx, idempotencyKey, userID, input, func(operation store.StateOperation) error {
		return s.repository.Run(ctx, operation)
	})
}

// RecordHeartbeatWithSessionBinding keeps the provisional session binding and
// heartbeat state update in the same PostgreSQL transaction.
func (s *ControlPlane) RecordHeartbeatWithSessionBinding(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.HeartbeatInput) (controlplane.HeartbeatResult, error) {
	return s.recordHeartbeatWithSessionBinding(ctx, idempotencyKey, userID, accessTokenHash, input, controlplane.AuditLogInput{})
}

// RecordHeartbeatWithSessionBindingAndAudit is the normalized HTTP path. The
// optional success event is queued inside the same business transaction; the
// middleware's later audit call is deduplicated by request ID.
func (s *ControlPlane) RecordHeartbeatWithSessionBindingAndAudit(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.HeartbeatInput, audit controlplane.AuditLogInput) (controlplane.HeartbeatResult, error) {
	return s.recordHeartbeatWithSessionBinding(ctx, idempotencyKey, userID, accessTokenHash, input, audit)
}

func (s *ControlPlane) recordHeartbeatWithSessionBinding(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.HeartbeatInput, audit controlplane.AuditLogInput) (controlplane.HeartbeatResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		recorder, ok := s.repository.(store.TransactionalHeartbeatRecorder)
		if !ok {
			return controlplane.HeartbeatResult{}, store.ErrNormalizedTransactionalHeartbeatRecorderRequired
		}
		if err := checkContext(ctx); err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		if strings.TrimSpace(userID) == "" {
			return controlplane.HeartbeatResult{}, controlplane.ErrUserNotFound
		}
		if strings.TrimSpace(accessTokenHash) == "" {
			return controlplane.HeartbeatResult{}, controlplane.ErrUnauthenticated
		}
		if !validIdempotencyKey(idempotencyKey) {
			return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyKeyRequired
		}
		if err := validateHeartbeatInput(input); err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		fingerprint, err := fingerprintValue(struct {
			UserID string                      `json:"user_id"`
			Input  controlplane.HeartbeatInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		result, err := recorder.RecordHeartbeatWithSessionBinding(ctx, store.DeviceHeartbeatRecord{
			Scope:           "control-plane-state",
			IdempotencyKey:  "heartbeat:" + userID + ":" + input.DeviceID + ":" + idempotencyKey,
			Fingerprint:     fingerprint,
			AccessTokenHash: accessTokenHash,
			UserID:          userID,
			Product:         input.Product,
			Input:           input,
			Audit:           audit,
		})
		if errors.Is(err, store.ErrSessionDeviceBindingConflict) {
			return controlplane.HeartbeatResult{}, controlplane.ErrDeviceBindingConflict
		}
		return result, err
	}
	binder, ok := s.repository.(store.TransactionalSessionBinder)
	if !ok {
		return s.RecordHeartbeat(ctx, idempotencyKey, userID, input)
	}
	return s.recordHeartbeatWithRunner(ctx, idempotencyKey, userID, input, func(operation store.StateOperation) error {
		err := binder.RunWithSessionBinding(ctx, accessTokenHash, userID, input.DeviceID, operation)
		if errors.Is(err, store.ErrSessionDeviceBindingConflict) {
			return controlplane.ErrDeviceBindingConflict
		}
		return err
	})
}

func (s *ControlPlane) recordHeartbeatWithRunner(ctx context.Context, idempotencyKey, userID string, input controlplane.HeartbeatInput, run func(store.StateOperation) error) (controlplane.HeartbeatResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateHeartbeatInput(input); err != nil {
		return controlplane.HeartbeatResult{}, err
	}

	return runHeartbeatState(run, func(state *store.State) (controlplane.HeartbeatResult, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.HeartbeatResult{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.HeartbeatResult{}, controlplane.ErrUserDisabled
		}
		device, err := resolveOwnedDevice(state, userID, input.DeviceID)
		if err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.HeartbeatResult{}, controlplane.ErrDeviceDisabled
		}

		fingerprint, err := fingerprintValue(struct {
			UserID string                      `json:"user_id"`
			Input  controlplane.HeartbeatInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		scope := "heartbeat:" + userID + ":" + input.DeviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyConflict
			}
			device = state.Devices[existing.ResourceID]
			return controlplane.HeartbeatResult{
				AcceptedAt:   device.LastSeenAt,
				DeviceStatus: device.Status,
			}, nil
		}

		acceptedAt := s.repository.Now().Format(time.RFC3339)
		device.DiskFreeBytes = input.Status.DiskFreeBytes
		device.MemoryTotalBytes = input.Status.MemoryTotalBytes
		device.MemoryAvailableBytes = input.Status.MemoryAvailableBytes
		device.CPULogicalCores = input.Status.CPULogicalCores
		device.RuntimeOSName = input.Status.OSName
		device.RuntimeOSVersion = input.Status.OSVersion
		device.KernelVersion = input.Status.KernelVersion
		device.CurrentMediaName = input.Status.CurrentMediaName
		device.PlaybackState = input.Status.PlaybackState
		device.LastSeenAt = acceptedAt
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  device.ID,
		}
		return controlplane.HeartbeatResult{
			AcceptedAt:   acceptedAt,
			DeviceStatus: device.Status,
		}, nil
	})
}

func runHeartbeatState(run func(store.StateOperation) error, fn func(*store.State) (controlplane.HeartbeatResult, error)) (controlplane.HeartbeatResult, error) {
	if run == nil {
		return controlplane.HeartbeatResult{}, errors.New("heartbeat state runner is required")
	}
	var result controlplane.HeartbeatResult
	err := run(func(state *store.State) error {
		var operationErr error
		result, operationErr = fn(state)
		return operationErr
	})
	return result, err
}

func (s *ControlPlane) ListModelPoolAccounts(ctx context.Context) ([]controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		if reader, ok := s.repository.(store.ModelPoolPageReader); ok {
			const pageSize = 200
			items := make([]controlplane.ModelPoolAccountSummary, 0)
			for offset := 0; ; {
				page, err := reader.ListModelPoolAccountsPage(ctx, offset, pageSize)
				if err != nil {
					return nil, err
				}
				if len(page.Items) == 0 {
					if len(items) >= page.Total {
						break
					}
					return nil, errors.New("normalized model pool page made no progress")
				}
				now := s.repository.Now()
				for index := range page.Items {
					normalizeModelPoolAccountSummary(&page.Items[index], now)
				}
				items = append(items, page.Items...)
				if len(items) >= page.Total {
					break
				}
				offset += len(page.Items)
			}
			return items, nil
		}
		return nil, store.ErrNormalizedModelPoolPageReaderRequired
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelPoolAccountSummary, error) {
		now := s.repository.Now()
		sweepExpiredModelLeases(state, now)
		refreshModelAccountStatuses(state, now)
		items := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
		for _, account := range state.ModelPoolAccounts {
			items = append(items, decorateModelPoolAccount(state, account, now))
		}
		slices.SortFunc(items, func(a, b controlplane.ModelPoolAccountSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) ListModelPoolAccountsPage(ctx context.Context, page, pageSize int) ([]controlplane.ModelPoolAccountSummary, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.ModelPoolPageReader)
		if !ok {
			return nil, 0, store.ErrNormalizedModelPoolPageReaderRequired
		}
		result, err := reader.ListModelPoolAccountsPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		now := s.repository.Now()
		for index := range result.Items {
			normalizeModelPoolAccountSummary(&result.Items[index], now)
		}
		return result.Items, result.Total, nil
	}
	if reader, ok := s.repository.(store.ModelPoolPageReader); ok {
		result, err := reader.ListModelPoolAccountsPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		now := s.repository.Now()
		for index := range result.Items {
			normalizeModelPoolAccountSummary(&result.Items[index], now)
		}
		return result.Items, result.Total, nil
	}
	items, err := s.ListModelPoolAccounts(ctx)
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(items), offset, pageSize)
	return items[start:end], len(items), nil
}

// ListModelPoolHealthAccounts keeps the normalized health worker on a bounded
// eligible-account query. A generic account page is insufficient here because
// disabled/cooldown/exhausted rows at the beginning of an ID-ordered page can
// otherwise hide later accounts that are ready to probe.
func (s *ControlPlane) ListModelPoolHealthAccounts(ctx context.Context, limit int) ([]controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if limit < 1 || limit > 200 {
		return nil, controlplane.ErrInvalidRequest
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.ModelPoolHealthPageReader)
		if !ok {
			return nil, store.ErrNormalizedModelPoolHealthPageReaderRequired
		}
		items, err := reader.ListModelPoolHealthAccounts(ctx, limit)
		if err != nil {
			return nil, err
		}
		now := s.repository.Now()
		for index := range items {
			normalizeModelPoolAccountSummary(&items[index], now)
		}
		return items, nil
	}
	items, err := s.ListModelPoolAccounts(ctx)
	if err != nil {
		return nil, err
	}
	if len(items) > limit {
		items = items[:limit]
	}
	return items, nil
}

func (s *ControlPlane) ListModelLeasesPage(ctx context.Context, page, pageSize int) ([]controlplane.ModelLeaseAdminSummary, int, error) {
	return s.ListModelLeasesPageWithOptions(ctx, page, pageSize, ModelLeaseListOptions{})
}

func (s *ControlPlane) ListModelLeasesPageWithOptions(ctx context.Context, page, pageSize int, options ModelLeaseListOptions) ([]controlplane.ModelLeaseAdminSummary, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	storageOptions, err := normalizeModelLeaseListOptions(options)
	if err != nil {
		return nil, 0, err
	}
	storageOptions.Offset = offset
	storageOptions.Limit = pageSize
	if reader, ok := s.repository.(store.ModelLeaseFilteredPageReader); ok {
		result, err := reader.ListModelLeasesPageWithOptions(ctx, storageOptions)
		if err != nil {
			return nil, 0, err
		}
		now := s.repository.Now()
		for index := range result.Items {
			normalizeModelLeaseAdminSummary(&result.Items[index], now)
		}
		return result.Items, result.Total, nil
	}
	if reader, ok := s.repository.(store.ModelLeasePageReader); ok {
		if storageOptions.Status != "" || storageOptions.Provider != "" || storageOptions.Model != "" || storageOptions.UserID != "" || storageOptions.DeviceID != "" || storageOptions.AccountID != "" || storageOptions.Sort != "" {
			return nil, 0, controlplane.ErrInvalidRequest
		}
		result, err := reader.ListModelLeasesPage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		now := s.repository.Now()
		for index := range result.Items {
			normalizeModelLeaseAdminSummary(&result.Items[index], now)
		}
		return result.Items, result.Total, nil
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, 0, store.ErrNormalizedModelLeasePageReaderRequired
	}
	result, err := withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelLeaseAdminSummary, error) {
		now := s.repository.Now()
		sweepExpiredModelLeases(state, now)
		items := make([]controlplane.ModelLeaseAdminSummary, 0, len(state.ModelLeases))
		for _, lease := range state.ModelLeases {
			item := modelLeaseAdminSummary(lease)
			if !modelLeaseAdminSummaryMatchesOptions(item, storageOptions, now) {
				continue
			}
			items = append(items, item)
		}
		sortModelLeaseAdminSummaries(items, storageOptions.Sort)
		return items, nil
	})
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(result), offset, pageSize)
	return result[start:end], len(result), nil
}

// GetModelLeaseAdminDetail returns lifecycle metadata only. Credentials and
// direct provider secrets never cross this administrative boundary.
func (s *ControlPlane) GetModelLeaseAdminDetail(ctx context.Context, leaseID string) (controlplane.ModelLeaseAdminDetail, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelLeaseAdminDetail{}, err
	}
	leaseID = strings.TrimSpace(leaseID)
	if leaseID == "" {
		return controlplane.ModelLeaseAdminDetail{}, controlplane.ErrInvalidRequest
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		reader, ok := s.repository.(store.ModelLeaseDetailReader)
		if !ok {
			return controlplane.ModelLeaseAdminDetail{}, store.ErrNormalizedModelLeaseDetailReaderRequired
		}
		return reader.GetModelLeaseAdminDetail(ctx, leaseID)
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelLeaseAdminDetail, error) {
		sweepExpiredModelLeases(state, s.repository.Now())
		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ModelLeaseAdminDetail{}, controlplane.ErrModelLeaseNotFound
		}
		return modelLeaseAdminDetail(lease), nil
	})
}

// ReclaimModelLease is an administrator-owned, idempotent release operation
// for stale or misconfigured leases. It does not require a client device
// binding and never returns the lease credential.
func (s *ControlPlane) ReclaimModelLease(ctx context.Context, idempotencyKey, leaseID string, input controlplane.ReleaseModelLeaseInput) (controlplane.ReleaseModelLeaseResult, error) {
	return s.reclaimModelLease(ctx, idempotencyKey, leaseID, input, controlplane.AuditLogInput{})
}

// ReclaimModelLeaseWithAudit is the normalized administrative HTTP path. The
// success event is committed with the lease release transaction.
func (s *ControlPlane) ReclaimModelLeaseWithAudit(ctx context.Context, idempotencyKey, leaseID string, input controlplane.ReleaseModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ReleaseModelLeaseResult, error) {
	return s.reclaimModelLease(ctx, idempotencyKey, leaseID, input, audit)
}

func (s *ControlPlane) reclaimModelLease(ctx context.Context, idempotencyKey, leaseID string, input controlplane.ReleaseModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ReleaseModelLeaseResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	leaseID = strings.TrimSpace(leaseID)
	input.Reason = strings.TrimSpace(input.Reason)
	if leaseID == "" || len(input.Reason) > 255 {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelLeaseRepository)
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, store.ErrNormalizedModelLeaseRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			LeaseID string                              `json:"lease_id"`
			Input   controlplane.ReleaseModelLeaseInput `json:"input"`
		}{LeaseID: leaseID, Input: input})
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		return writer.ReclaimModelLease(ctx, store.ModelLeaseReclaimRecord{
			Scope: "control-plane-state", IdempotencyKey: "admin-reclaim-model-lease:" + leaseID + ":" + idempotencyKey,
			Fingerprint: fingerprint, LeaseID: leaseID, Reason: input.Reason, Audit: audit,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ReleaseModelLeaseResult, error) {
		now := s.repository.Now()
		sweepExpiredModelLeases(state, now)
		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrModelLeaseNotFound
		}
		fingerprint, err := fingerprintValue(struct {
			LeaseID string                              `json:"lease_id"`
			Input   controlplane.ReleaseModelLeaseInput `json:"input"`
		}{LeaseID: leaseID, Input: input})
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		scope := "admin-reclaim-model-lease:" + leaseID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyConflict
			}
			return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
		}
		if lease.Status != controlplane.ModelLeaseStatusActive && lease.Status != controlplane.ModelLeaseStatusReleased && lease.Status != controlplane.ModelLeaseStatusExpired {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrModelLeaseStateConflict
		}
		if lease.Status == controlplane.ModelLeaseStatusActive {
			lease.Status = controlplane.ModelLeaseStatusReleased
			lease.ReleasedAt = now.Format(time.RFC3339)
			state.ModelLeases[lease.ID] = lease
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: leaseID}
		return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
	})
}

func (s *ControlPlane) CreateModelPoolAccount(ctx context.Context, idempotencyKey string, input controlplane.CreateModelPoolAccountInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.createModelPoolAccount(ctx, idempotencyKey, input, controlplane.AuditLogInput{})
}

// CreateModelPoolAccountWithAudit is the normalized administrative HTTP path.
// The optional success event is committed with the account row, encrypted
// secret and idempotency record when the repository supports that boundary.
func (s *ControlPlane) CreateModelPoolAccountWithAudit(ctx context.Context, idempotencyKey string, input controlplane.CreateModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.createModelPoolAccount(ctx, idempotencyKey, input, audit)
}

func (s *ControlPlane) createModelPoolAccount(ctx context.Context, idempotencyKey string, input controlplane.CreateModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateModelPoolAccountInput(&input); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	fingerprint, err := fingerprintValue(input)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		creator, ok := s.repository.(store.ModelPoolAccountCreator)
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, store.ErrNormalizedModelPoolAccountCreatorRequired
		}
		account, err := creator.CreateModelPoolAccount(ctx, store.ModelPoolAccountCreateRecord{
			Scope:            "control-plane-state",
			IdempotencyKey:   "create-model-account:" + idempotencyKey,
			Fingerprint:      fingerprint,
			Provider:         input.Provider,
			Model:            input.Model,
			BaseURL:          input.BaseURL,
			APIKey:           input.APIKey,
			Status:           input.Status,
			Priority:         input.Priority,
			DailyLimit:       input.DailyLimit,
			ConcurrencyLimit: input.ConcurrencyLimit,
			Audit:            audit,
		})
		if errors.Is(err, store.ErrTransactionalSecretStoreRequired) {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
		}
		return account, err
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		scope := "create-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		account := controlplane.ModelPoolAccountSummary{
			ID:               nextID(state, "mpa"),
			Provider:         input.Provider,
			Model:            input.Model,
			BaseURL:          input.BaseURL,
			Status:           input.Status,
			Priority:         input.Priority,
			DailyLimit:       input.DailyLimit,
			ConcurrencyLimit: input.ConcurrencyLimit,
			SecretConfigured: true,
		}
		account.SecretRef = "model-account/" + account.ID
		state.ModelPoolAccounts[account.ID] = account
		if err := s.secretStore.Put(ctx, account.SecretRef, input.APIKey); err != nil {
			delete(state.ModelPoolAccounts, account.ID)
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  account.ID,
		}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) DisableModelPoolAccount(ctx context.Context, idempotencyKey, accountID string) (controlplane.ModelPoolAccountSummary, error) {
	return s.disableModelPoolAccount(ctx, idempotencyKey, accountID, controlplane.AuditLogInput{})
}

// DisableModelPoolAccountWithAudit is the normalized administrative HTTP
// path. The success event shares the account mutation transaction.
func (s *ControlPlane) DisableModelPoolAccountWithAudit(ctx context.Context, idempotencyKey, accountID string, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.disableModelPoolAccount(ctx, idempotencyKey, accountID, audit)
}

func (s *ControlPlane) disableModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
	}
	fingerprint, err := fingerprintValue(struct {
		AccountID string `json:"account_id"`
	}{AccountID: accountID})
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelPoolRepository)
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, store.ErrNormalizedModelPoolRepositoryRequired
		}
		return writer.DisableModelPoolAccount(ctx, store.ModelPoolAccountMutationRecord{
			Scope: "control-plane-state", IdempotencyKey: "disable-model-account:" + idempotencyKey, Fingerprint: fingerprint, AccountID: accountID, Audit: audit,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		scope := "disable-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		sweepExpiredModelLeases(state, s.repository.Now())
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
		}
		if activeLeaseCountForAccount(state, account.ID) > 0 {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountInUse
		}
		account.Status = controlplane.ModelAccountStatusDisabled
		state.ModelPoolAccounts[account.ID] = account
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  account.ID,
		}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) UpdateModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.UpdateModelPoolAccountInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.updateModelPoolAccount(ctx, idempotencyKey, accountID, input, controlplane.AuditLogInput{})
}

// UpdateModelPoolAccountWithAudit is the normalized administrative HTTP path.
// The success event shares the account mutation transaction.
func (s *ControlPlane) UpdateModelPoolAccountWithAudit(ctx context.Context, idempotencyKey, accountID string, input controlplane.UpdateModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.updateModelPoolAccount(ctx, idempotencyKey, accountID, input, audit)
}

func (s *ControlPlane) updateModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.UpdateModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
	}
	if input.BaseURL == nil && input.Priority == nil && input.DailyLimit == nil && input.ConcurrencyLimit == nil && input.Status == nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.BaseURL != nil {
		value := strings.TrimRight(strings.TrimSpace(*input.BaseURL), "/")
		input.BaseURL = &value
		if value != "" && !validModelBaseURL(value) {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
		}
	}
	if input.Priority != nil && *input.Priority < 0 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.DailyLimit != nil && *input.DailyLimit < 0 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.ConcurrencyLimit != nil && *input.ConcurrencyLimit < 1 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.Status != nil && !validModelAccountStatus(*input.Status) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	fingerprint, err := fingerprintValue(struct {
		AccountID string                                   `json:"account_id"`
		Input     controlplane.UpdateModelPoolAccountInput `json:"input"`
	}{AccountID: accountID, Input: input})
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelPoolRepository)
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, store.ErrNormalizedModelPoolRepositoryRequired
		}
		return writer.UpdateModelPoolAccount(ctx, store.ModelPoolAccountUpdateRecord{
			ModelPoolAccountMutationRecord: store.ModelPoolAccountMutationRecord{
				Scope: "control-plane-state", IdempotencyKey: "update-model-account:" + idempotencyKey, Fingerprint: fingerprint, AccountID: accountID, Audit: audit,
			}, Input: input,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		scope := "update-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
		}
		sweepExpiredModelLeases(state, s.repository.Now())
		if input.Status != nil && *input.Status == controlplane.ModelAccountStatusDisabled && activeLeaseCountForAccount(state, account.ID) > 0 {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountInUse
		}
		if input.ConcurrencyLimit != nil && activeLeaseCountForAccount(state, account.ID) > *input.ConcurrencyLimit {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolConcurrencyConflict
		}
		if input.BaseURL != nil {
			account.BaseURL = *input.BaseURL
		}
		if input.Priority != nil {
			account.Priority = *input.Priority
		}
		if input.DailyLimit != nil {
			account.DailyLimit = *input.DailyLimit
		}
		if input.ConcurrencyLimit != nil {
			account.ConcurrencyLimit = *input.ConcurrencyLimit
		}
		if input.Status != nil {
			account.Status = *input.Status
			if *input.Status != controlplane.ModelAccountStatusCooldown {
				account.CooldownUntil = ""
			}
		}
		state.ModelPoolAccounts[account.ID] = account
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: account.ID}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) RotateModelPoolAccountSecret(ctx context.Context, idempotencyKey, accountID string, input controlplane.RotateModelPoolAccountSecretInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.rotateModelPoolAccountSecret(ctx, idempotencyKey, accountID, input, controlplane.AuditLogInput{})
}

// RotateModelPoolAccountSecretWithAudit uses the normalized transactional
// SecretStore boundary when available. Compatibility stores retain the staged
// fallback and the regular post-request audit path.
func (s *ControlPlane) RotateModelPoolAccountSecretWithAudit(ctx context.Context, idempotencyKey, accountID string, input controlplane.RotateModelPoolAccountSecretInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	return s.rotateModelPoolAccountSecret(ctx, idempotencyKey, accountID, input, audit)
}

func (s *ControlPlane) rotateModelPoolAccountSecret(ctx context.Context, idempotencyKey, accountID string, input controlplane.RotateModelPoolAccountSecretInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" || validateRotateModelPoolAccountSecretInput(&input) != nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	type rotationContext struct {
		account     controlplane.ModelPoolAccountSummary
		fingerprint string
		existing    *controlplane.ModelPoolAccountSummary
	}
	fingerprint, err := fingerprintValue(struct {
		AccountID string                                         `json:"account_id"`
		Input     controlplane.RotateModelPoolAccountSecretInput `json:"input"`
	}{AccountID: accountID, Input: input})
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	scope := "rotate-model-account-secret:" + accountID + ":" + idempotencyKey
	normalized := false
	if source, ok := s.repository.(store.NormalizedReadSource); ok {
		normalized = source.UsesNormalizedReadSource()
	}
	var rotation rotationContext
	if normalized {
		preparer, ok := s.repository.(store.ModelPoolSecretRotationPreparer)
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, store.ErrNormalizedModelPoolSecretRotationPreparerRequired
		}
		rotator, ok := s.repository.(store.ModelPoolSecretRotator)
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, store.ErrNormalizedModelPoolSecretRotatorRequired
		}
		preparation, prepareErr := preparer.PrepareModelPoolAccountSecretRotation(ctx, "control-plane-state", scope, fingerprint, accountID)
		if prepareErr != nil {
			return controlplane.ModelPoolAccountSummary{}, prepareErr
		}
		rotation = rotationContext{account: preparation.Account, fingerprint: fingerprint, existing: preparation.Existing}
		if rotation.existing != nil {
			return *rotation.existing, nil
		}
		probe := s.probeModelPoolAccount(ctx, rotation.account, controlplane.TestModelPoolAccountInput{TimeoutSeconds: input.TimeoutSeconds}, input.APIKey)
		if probe.Status != "succeeded" {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolSecretValidation
		}
		rotated, rotateErr := rotator.RotateModelPoolAccountSecret(ctx, store.ModelPoolSecretRotationRecord{
			Scope: "control-plane-state", IdempotencyKey: scope,
			Fingerprint: fingerprint, AccountID: accountID, ExpectedSecretRef: rotation.account.SecretRef,
			APIKey: input.APIKey, Probe: probe, Audit: audit,
		})
		if errors.Is(rotateErr, store.ErrTransactionalSecretStoreRequired) {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
		}
		return rotated, rotateErr
	}
	rotation, err = withState(ctx, s.repository, func(state *store.State) (rotationContext, error) {
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return rotationContext{}, controlplane.ErrModelPoolAccountNotFound
		}
		if existing, exists := state.IdempotencyRecords[scope]; exists {
			if existing.Fingerprint != fingerprint {
				return rotationContext{}, controlplane.ErrIdempotencyConflict
			}
			stored, found := state.ModelPoolAccounts[existing.ResourceID]
			if !found {
				return rotationContext{}, controlplane.ErrModelPoolAccountNotFound
			}
			summary := decorateModelPoolAccount(state, stored, s.repository.Now())
			return rotationContext{account: stored, fingerprint: fingerprint, existing: &summary}, nil
		}
		return rotationContext{account: account, fingerprint: fingerprint}, nil
	})
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if rotation.existing != nil {
		return *rotation.existing, nil
	}
	probe := s.probeModelPoolAccount(ctx, rotation.account, controlplane.TestModelPoolAccountInput{TimeoutSeconds: input.TimeoutSeconds}, input.APIKey)
	if probe.Status != "succeeded" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolSecretValidation
	}
	oldSecretRef := rotation.account.SecretRef
	rotationToken, err := randomToken("rotation_", 12)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.NewError(http.StatusInternalServerError, "RANDOM_GENERATION_FAILED", "无法生成密钥轮换引用")
	}
	stagedSecretRef := oldSecretRef + "/" + rotationToken
	deleteSecret := func(reference string) error {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.WithoutCancel(ctx), secretRotationCleanupTimeout)
		defer cleanupCancel()
		err := s.secretStore.Delete(cleanupCtx, reference)
		if errors.Is(err, store.ErrSecretNotFound) {
			return nil
		}
		return err
	}
	if err := s.secretStore.Put(ctx, stagedSecretRef, input.APIKey); err != nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
	}
	committedRotation := false
	if err := s.repository.Run(ctx, func(state *store.State) error {
		account, exists := state.ModelPoolAccounts[accountID]
		if !exists {
			return controlplane.ErrModelPoolAccountNotFound
		}
		scope := "rotate-model-account-secret:" + accountID + ":" + idempotencyKey
		if existing, exists := state.IdempotencyRecords[scope]; exists {
			if existing.Fingerprint != rotation.fingerprint {
				return controlplane.ErrIdempotencyConflict
			}
			return nil
		}
		if account.SecretRef != rotation.account.SecretRef {
			return controlplane.ErrModelPoolSecretRotationConflict
		}
		account.SecretRef = stagedSecretRef
		state.ModelPoolAccounts[account.ID] = account
		if oldSecretRef != "" {
			if state.PendingSecretCleanup == nil {
				state.PendingSecretCleanup = make(map[string]time.Time)
			}
			if _, exists := state.PendingSecretCleanup[oldSecretRef]; !exists {
				state.PendingSecretCleanup[oldSecretRef] = s.repository.Now()
			}
		}
		resultID := nextID(state, "model_test")
		state.ModelPoolTestResults[resultID] = probe
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: rotation.fingerprint, ResourceID: account.ID}
		committedRotation = true
		return nil
	}); err != nil {
		// A commit error (or cancellation while committing) does not prove the
		// transaction rolled back. Keep the candidate for staged-secret
		// reconciliation instead of deleting a value a committed row may use.
		if !errors.Is(err, store.ErrCommitOutcomeUnknown) && !errors.Is(err, context.Canceled) && !errors.Is(err, context.DeadlineExceeded) {
			if cleanupErr := deleteSecret(stagedSecretRef); cleanupErr != nil {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
			}
		}
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !committedRotation {
		if cleanupErr := deleteSecret(stagedSecretRef); cleanupErr != nil {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
		}
		return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
			account, exists := state.ModelPoolAccounts[accountID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		})
	}
	// The active reference is switched transactionally with the business state.
	// The old reference is queued in that same transaction, so an unknown
	// commit outcome still leaves a durable compensation fact.
	if cleanupErr := deleteSecret(oldSecretRef); cleanupErr != nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
	}
	if err := s.repository.Run(ctx, func(state *store.State) error {
		delete(state.PendingSecretCleanup, oldSecretRef)
		return nil
	}); err != nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		account, exists := state.ModelPoolAccounts[accountID]
		if !exists {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
		}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}
func (s *ControlPlane) CreateModelLease(ctx context.Context, idempotencyKey, userID, deviceID string, input controlplane.CreateModelLeaseInput) (controlplane.ModelLease, error) {
	return s.createModelLease(ctx, idempotencyKey, userID, deviceID, input, controlplane.AuditLogInput{})
}

// CreateModelLeaseWithAudit is the normalized client HTTP path. The success
// event is committed with the lease creation transaction.
func (s *ControlPlane) CreateModelLeaseWithAudit(ctx context.Context, idempotencyKey, userID, deviceID string, input controlplane.CreateModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ModelLease, error) {
	return s.createModelLease(ctx, idempotencyKey, userID, deviceID, input, audit)
}

func (s *ControlPlane) createModelLease(ctx context.Context, idempotencyKey, userID, deviceID string, input controlplane.CreateModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ModelLease, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelLease{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelLease{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateModelLeaseInput(&input); err != nil {
		return controlplane.ModelLease{}, err
	}
	userID = strings.TrimSpace(userID)
	deviceID = strings.TrimSpace(deviceID)
	fingerprint, err := fingerprintValue(struct {
		UserID   string                             `json:"user_id"`
		DeviceID string                             `json:"device_id"`
		Input    controlplane.CreateModelLeaseInput `json:"input"`
	}{UserID: userID, DeviceID: deviceID, Input: input})
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		creator, ok := s.repository.(store.ModelLeaseCreator)
		if !ok {
			return controlplane.ModelLease{}, store.ErrNormalizedModelLeaseCreatorRequired
		}
		return creator.CreateModelLease(ctx, store.ModelLeaseCreateRecord{
			Scope: "control-plane-state", IdempotencyKey: "create-model-lease:" + userID + ":" + deviceID + ":" + idempotencyKey,
			Fingerprint: fingerprint, UserID: userID, DeviceID: deviceID, Provider: input.Provider,
			Model: input.Model, Purpose: input.Purpose, MaxDurationSeconds: input.MaxDurationSeconds, Audit: audit,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelLease, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		scope := "create-model-lease:" + userID + ":" + device.ID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
			}
			lease, exists := state.ModelLeases[existing.ResourceID]
			if !exists {
				return controlplane.ModelLease{}, controlplane.ErrModelLeaseNotFound
			}
			return lease, nil
		}
		if policy, configured := state.UserAuthorizationPolicies[userID]; configured {
			if len(policy.AllowedModels) > 0 && !slices.Contains(policy.AllowedModels, input.Provider+"/"+input.Model) {
				return controlplane.ModelLease{}, controlplane.ErrUserModelNotAuthorized
			}
			if policy.DailyTokenLimit > 0 && dailyUsedTokensForUser(state, userID, s.repository.Now()) >= policy.DailyTokenLimit {
				return controlplane.ModelLease{}, controlplane.ErrUserRecordedQuotaExceeded
			}
		}

		account, ok := selectModelPoolAccount(state, input.Provider, input.Model, s.repository.Now())
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}
		if _, err := s.secretStore.Get(ctx, account.SecretRef); err != nil {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}

		now := s.repository.Now()
		lease := controlplane.ModelLease{
			ID:               nextID(state, "lease"),
			UserID:           userID,
			DeviceID:         device.ID,
			AccountID:        account.ID,
			Purpose:          input.Purpose,
			Provider:         account.Provider,
			Model:            account.Model,
			Status:           controlplane.ModelLeaseStatusActive,
			CreatedAt:        now.Format(time.RFC3339),
			ExpiresAt:        now.Add(time.Duration(input.MaxDurationSeconds) * time.Second).UTC().Format(time.RFC3339),
			ProxyMode:        controlplane.ModelLeaseProxyModeDirectLease,
			DirectBaseURL:    account.BaseURL,
			ConcurrencyLimit: account.ConcurrencyLimit,
		}
		state.ModelLeases[lease.ID] = lease
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  lease.ID,
		}
		return lease, nil
	})
}

func (s *ControlPlane) RenewModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.RenewModelLeaseInput) (controlplane.ModelLease, error) {
	return s.renewModelLease(ctx, idempotencyKey, userID, deviceID, leaseID, input, controlplane.AuditLogInput{})
}

// RenewModelLeaseWithAudit is the normalized client HTTP path. The success
// event is committed with the lease renewal transaction.
func (s *ControlPlane) RenewModelLeaseWithAudit(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.RenewModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ModelLease, error) {
	return s.renewModelLease(ctx, idempotencyKey, userID, deviceID, leaseID, input, audit)
}

func (s *ControlPlane) renewModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.RenewModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ModelLease, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelLease{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelLease{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateRenewModelLeaseInput(&input); err != nil {
		return controlplane.ModelLease{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelLeaseRepository)
		if !ok {
			return controlplane.ModelLease{}, store.ErrNormalizedModelLeaseRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			UserID       string                            `json:"user_id"`
			DeviceID     string                            `json:"device_id"`
			LeaseID      string                            `json:"lease_id"`
			RenewRequest controlplane.RenewModelLeaseInput `json:"renew_request"`
		}{UserID: userID, DeviceID: deviceID, LeaseID: leaseID, RenewRequest: input})
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		return writer.RenewModelLease(ctx, store.ModelLeaseRenewRecord{
			Scope: "control-plane-state", IdempotencyKey: "renew-model-lease:" + leaseID + ":" + idempotencyKey,
			Fingerprint: fingerprint, UserID: userID, DeviceID: deviceID, LeaseID: leaseID, ExtendSeconds: input.ExtendSeconds, Audit: audit,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelLease, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ModelLease{}, controlplane.ErrForbidden
		}

		fingerprint, err := fingerprintValue(struct {
			UserID       string                            `json:"user_id"`
			DeviceID     string                            `json:"device_id"`
			LeaseID      string                            `json:"lease_id"`
			RenewRequest controlplane.RenewModelLeaseInput `json:"renew_request"`
		}{
			UserID:       userID,
			DeviceID:     device.ID,
			LeaseID:      leaseID,
			RenewRequest: input,
		})
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		scope := "renew-model-lease:" + leaseID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
			}
			return state.ModelLeases[leaseID], nil
		}

		if lease.Status != controlplane.ModelLeaseStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
		}
		account, ok := state.ModelPoolAccounts[lease.AccountID]
		if !ok || account.Status != controlplane.ModelAccountStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
		}
		lease.ExpiresAt = expiresAt.Add(time.Duration(input.ExtendSeconds) * time.Second).UTC().Format(time.RFC3339)
		state.ModelLeases[lease.ID] = lease
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  lease.ID,
		}
		return lease, nil
	})
}

func (s *ControlPlane) ReleaseModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.ReleaseModelLeaseInput) (controlplane.ReleaseModelLeaseResult, error) {
	return s.releaseModelLease(ctx, idempotencyKey, userID, deviceID, leaseID, input, controlplane.AuditLogInput{})
}

// ReleaseModelLeaseWithAudit is the normalized client HTTP path. The success
// event is committed with the lease release transaction.
func (s *ControlPlane) ReleaseModelLeaseWithAudit(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.ReleaseModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ReleaseModelLeaseResult, error) {
	return s.releaseModelLease(ctx, idempotencyKey, userID, deviceID, leaseID, input, audit)
}

func (s *ControlPlane) releaseModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.ReleaseModelLeaseInput, audit controlplane.AuditLogInput) (controlplane.ReleaseModelLeaseResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	if len(strings.TrimSpace(input.Reason)) > 255 {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	input.Reason = strings.TrimSpace(input.Reason)
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelLeaseRepository)
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, store.ErrNormalizedModelLeaseRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			UserID       string                              `json:"user_id"`
			DeviceID     string                              `json:"device_id"`
			LeaseID      string                              `json:"lease_id"`
			ReleaseInput controlplane.ReleaseModelLeaseInput `json:"release_input"`
		}{UserID: userID, DeviceID: deviceID, LeaseID: leaseID, ReleaseInput: input})
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		return writer.ReleaseModelLease(ctx, store.ModelLeaseReleaseRecord{
			Scope: "control-plane-state", IdempotencyKey: "release-model-lease:" + leaseID + ":" + idempotencyKey,
			Fingerprint: fingerprint, UserID: userID, DeviceID: deviceID, LeaseID: leaseID, Reason: input.Reason, Audit: audit,
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ReleaseModelLeaseResult, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrForbidden
		}

		fingerprint, err := fingerprintValue(struct {
			UserID       string                              `json:"user_id"`
			DeviceID     string                              `json:"device_id"`
			LeaseID      string                              `json:"lease_id"`
			ReleaseInput controlplane.ReleaseModelLeaseInput `json:"release_input"`
		}{
			UserID:       userID,
			DeviceID:     device.ID,
			LeaseID:      leaseID,
			ReleaseInput: input,
		})
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		scope := "release-model-lease:" + leaseID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyConflict
			}
			return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
		}
		if lease.Status == controlplane.ModelLeaseStatusActive {
			lease.Status = controlplane.ModelLeaseStatusReleased
			lease.ReleasedAt = s.repository.Now().Format(time.RFC3339)
			state.ModelLeases[lease.ID] = lease
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  leaseID,
		}
		return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
	})
}

func (s *ControlPlane) TestModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.TestModelPoolAccountInput) (controlplane.ModelPoolConnectivityTestResult, error) {
	return s.testModelPoolAccount(ctx, idempotencyKey, accountID, input, controlplane.AuditLogInput{})
}

// TestModelPoolAccountWithAudit is the normalized administrative HTTP path.
// The success event is committed with the test result and cooldown transition.
func (s *ControlPlane) TestModelPoolAccountWithAudit(ctx context.Context, idempotencyKey, accountID string, input controlplane.TestModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolConnectivityTestResult, error) {
	return s.testModelPoolAccount(ctx, idempotencyKey, accountID, input, audit)
}

func (s *ControlPlane) testModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.TestModelPoolAccountInput, audit controlplane.AuditLogInput) (controlplane.ModelPoolConnectivityTestResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" || validateTestModelPoolAccountInput(&input) != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	fingerprint, err := fingerprintValue(struct {
		AccountID string                                 `json:"account_id"`
		Input     controlplane.TestModelPoolAccountInput `json:"input"`
	}{AccountID: accountID, Input: input})
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		if tester, ok := s.repository.(store.ModelPoolTestRepository); ok {
			preparation, prepareErr := tester.PrepareModelPoolAccountTest(ctx, store.ModelPoolTestPrepareRecord{
				Scope: "control-plane-state", IdempotencyKey: "test-model-account:" + accountID + ":" + idempotencyKey,
				Fingerprint: fingerprint, AccountID: accountID,
			})
			if prepareErr != nil {
				return controlplane.ModelPoolConnectivityTestResult{}, prepareErr
			}
			if preparation.Cached != nil {
				return *preparation.Cached, nil
			}
			secret, secretErr := s.secretStore.Get(ctx, preparation.Account.SecretRef)
			if secretErr != nil {
				return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrSecretStoreUnavailable
			}
			result := s.probeModelPoolAccount(ctx, preparation.Account, input, secret)
			return tester.RecordModelPoolAccountTest(ctx, store.ModelPoolTestRecord{
				Scope: "control-plane-state", IdempotencyKey: "test-model-account:" + accountID + ":" + idempotencyKey,
				Fingerprint: fingerprint, AccountID: accountID, Result: result, Audit: audit,
			})
		}
		return controlplane.ModelPoolConnectivityTestResult{}, store.ErrNormalizedModelPoolTestRepositoryRequired
	}
	type modelPoolTestContext struct {
		account controlplane.ModelPoolAccountSummary
		cached  *controlplane.ModelPoolConnectivityTestResult
	}
	testContext, err := withState(ctx, s.repository, func(state *store.State) (modelPoolTestContext, error) {
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return modelPoolTestContext{}, controlplane.ErrModelPoolAccountNotFound
		}
		if existing, ok := state.IdempotencyRecords["test-model-account:"+accountID+":"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return modelPoolTestContext{}, controlplane.ErrIdempotencyConflict
			}
			cached, exists := state.ModelPoolTestResults[existing.ResourceID]
			if !exists {
				return modelPoolTestContext{}, controlplane.ErrModelPoolAccountNotFound
			}
			return modelPoolTestContext{account: account, cached: &cached}, nil
		}
		return modelPoolTestContext{account: account}, nil
	})
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if testContext.cached != nil {
		return *testContext.cached, nil
	}
	account := testContext.account
	secret, err := s.secretStore.Get(ctx, account.SecretRef)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrSecretStoreUnavailable
	}
	result := s.probeModelPoolAccount(ctx, account, input, secret)
	if err := s.repository.Run(ctx, func(state *store.State) error {
		scope := "test-model-account:" + accountID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ErrIdempotencyConflict
			}
			if cached, exists := state.ModelPoolTestResults[existing.ResourceID]; exists {
				result = cached
			}
			return nil
		}
		resultID := nextID(state, "model_test")
		state.ModelPoolTestResults[resultID] = result
		if account, exists := state.ModelPoolAccounts[accountID]; exists && account.Status != controlplane.ModelAccountStatusDisabled {
			if result.Status == "succeeded" {
				account.Status = controlplane.ModelAccountStatusActive
				account.CooldownUntil = ""
			} else if result.Status == "failed" || result.Status == "timeout" {
				account.Status = controlplane.ModelAccountStatusCooldown
				cooldownUntil := s.repository.Now().Add(modelAccountCooldownDuration)
				if testedAt, parseErr := time.Parse(time.RFC3339, result.TestedAt); parseErr == nil {
					cooldownUntil = testedAt.Add(modelAccountCooldownDuration)
				}
				account.CooldownUntil = cooldownUntil.UTC().Format(time.RFC3339)
			}
			state.ModelPoolAccounts[accountID] = account
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: resultID}
		return nil
	}); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	return result, nil
}

func (s *ControlPlane) RecordDirectLLMCall(ctx context.Context, idempotencyKey, userID, deviceID, requestID string, input controlplane.CreateDirectLLMCallRecordInput) (controlplane.ModelUsageRecord, error) {
	return s.recordDirectLLMCall(ctx, idempotencyKey, userID, deviceID, requestID, input, controlplane.AuditLogInput{})
}

// RecordDirectLLMCallWithAudit is the normalized HTTP path. The optional
// success event is written to the same transaction as the usage summary,
// quota transition and idempotency record; compatibility stores retain the
// existing post-request audit path.
func (s *ControlPlane) RecordDirectLLMCallWithAudit(ctx context.Context, idempotencyKey, userID, deviceID, requestID string, input controlplane.CreateDirectLLMCallRecordInput, audit controlplane.AuditLogInput) (controlplane.ModelUsageRecord, error) {
	return s.recordDirectLLMCall(ctx, idempotencyKey, userID, deviceID, requestID, input, audit)
}

func (s *ControlPlane) recordDirectLLMCall(ctx context.Context, idempotencyKey, userID, deviceID, requestID string, input controlplane.CreateDirectLLMCallRecordInput, audit controlplane.AuditLogInput) (controlplane.ModelUsageRecord, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateDirectLLMCallRecordInput(&input); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if input.TotalTokens == 0 {
		input.TotalTokens = input.InputTokens + input.OutputTokens
	}
	if input.TotalTokens < input.InputTokens+input.OutputTokens {
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ModelUsageRepository)
		if !ok {
			return controlplane.ModelUsageRecord{}, store.ErrNormalizedModelUsageRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			UserID   string                                      `json:"user_id"`
			DeviceID string                                      `json:"device_id"`
			Input    controlplane.CreateDirectLLMCallRecordInput `json:"input"`
		}{UserID: userID, DeviceID: deviceID, Input: input})
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		return writer.RecordDirectLLMCall(ctx, store.ModelUsageWriteRecord{
			Scope: "control-plane-state", IdempotencyKey: "record-direct-llm-call:" + userID + ":" + deviceID + ":" + idempotencyKey,
			Fingerprint: fingerprint, UserID: userID, DeviceID: deviceID, RequestID: strings.TrimSpace(requestID), Input: input, Audit: audit,
		})
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelUsageRecord, error) {
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		lease, ok := state.ModelLeases[input.LeaseID]
		if !ok {
			return controlplane.ModelUsageRecord{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
		}
		if lease.Provider != input.Provider || lease.Model != input.Model {
			return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
		}
		fingerprint, err := fingerprintValue(struct {
			UserID   string                                      `json:"user_id"`
			DeviceID string                                      `json:"device_id"`
			Input    controlplane.CreateDirectLLMCallRecordInput `json:"input"`
		}{UserID: userID, DeviceID: device.ID, Input: input})
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		scope := "record-direct-llm-call:" + userID + ":" + device.ID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
			}
			return state.ModelUsageRecords[existing.ResourceID], nil
		}
		for _, existing := range state.ModelUsageRecords {
			if existing.ClientCallID == input.ClientCallID && existing.LeaseID == input.LeaseID {
				if existing.Provider != input.Provider || existing.Model != input.Model || existing.InputTokens != input.InputTokens || existing.OutputTokens != input.OutputTokens || existing.TotalTokens != input.TotalTokens || existing.LatencyMS != input.LatencyMS || existing.Status != input.Status || existing.UsageSource != input.UsageSource || existing.ErrorCode != input.ErrorCode {
					return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
				}
				state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: existing.ID}
				return existing, nil
			}
		}
		if policy, configured := state.UserAuthorizationPolicies[userID]; configured && policy.DailyTokenLimit > 0 && dailyUsedTokensForUser(state, userID, s.repository.Now()) >= policy.DailyTokenLimit {
			return controlplane.ModelUsageRecord{}, controlplane.ErrUserRecordedQuotaExceeded
		}
		record := controlplane.ModelUsageRecord{
			ID:           nextID(state, "usage"),
			LeaseID:      input.LeaseID,
			ClientCallID: input.ClientCallID,
			RequestID:    strings.TrimSpace(requestID),
			Provider:     input.Provider,
			Model:        input.Model,
			InputTokens:  input.InputTokens,
			OutputTokens: input.OutputTokens,
			TotalTokens:  input.TotalTokens,
			LatencyMS:    input.LatencyMS,
			Status:       input.Status,
			UsageSource:  input.UsageSource,
			ErrorCode:    input.ErrorCode,
			CreatedAt:    s.repository.Now().Format(time.RFC3339),
		}
		state.ModelUsageRecords[record.ID] = record
		account, accountExists := state.ModelPoolAccounts[lease.AccountID]
		if accountExists && account.DailyLimit > 0 && dailyUsedTokensForAccount(state, account.ID, s.repository.Now()) >= account.DailyLimit {
			account.Status = controlplane.ModelAccountStatusExhausted
			state.ModelPoolAccounts[account.ID] = account
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: record.ID}
		return record, nil
	})
}

func (s *ControlPlane) ListModelUsage(ctx context.Context) ([]controlplane.ModelUsageRecord, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedModelUsagePageReaderRequired
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelUsageRecord, error) {
		items := make([]controlplane.ModelUsageRecord, 0, len(state.ModelUsageRecords))
		for _, item := range state.ModelUsageRecords {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.ModelUsageRecord) int {
			if a.CreatedAt != b.CreatedAt {
				return strings.Compare(b.CreatedAt, a.CreatedAt)
			}
			return strings.Compare(b.ID, a.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) ListModelUsagePage(ctx context.Context, page, pageSize int) ([]controlplane.ModelUsageRecord, int, error) {
	return s.ListModelUsagePageWithOptions(ctx, page, pageSize, ModelUsageListOptions{})
}

func (s *ControlPlane) ListModelUsagePageWithOptions(ctx context.Context, page, pageSize int, options ModelUsageListOptions) ([]controlplane.ModelUsageRecord, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	storageOptions, err := normalizeModelUsageListOptions(options)
	if err != nil {
		return nil, 0, err
	}
	storageOptions.Offset = offset
	storageOptions.Limit = pageSize
	if reader, ok := s.repository.(store.ModelUsageFilteredPageReader); ok {
		result, err := reader.ListModelUsagePageWithOptions(ctx, storageOptions)
		if err != nil {
			return nil, 0, err
		}
		return result.Items, result.Total, nil
	}
	if reader, ok := s.repository.(store.ModelUsagePageReader); ok {
		if storageOptions.Provider != "" || storageOptions.Model != "" || storageOptions.UserID != "" || storageOptions.DeviceID != "" || storageOptions.RequestID != "" || storageOptions.CreatedAfter != nil || storageOptions.CreatedBefore != nil || (storageOptions.Sort != "" && storageOptions.Sort != store.ModelUsageSortCreatedDesc) {
			return nil, 0, controlplane.ErrInvalidRequest
		}
		result, err := reader.ListModelUsagePage(ctx, offset, pageSize)
		if err != nil {
			return nil, 0, err
		}
		return result.Items, result.Total, nil
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, 0, store.ErrNormalizedModelUsagePageReaderRequired
	}
	result, err := withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelUsageRecord, error) {
		items := make([]controlplane.ModelUsageRecord, 0, len(state.ModelUsageRecords))
		for _, item := range state.ModelUsageRecords {
			lease := state.ModelLeases[item.LeaseID]
			if !modelUsageMatchesListOptions(item, storageOptions, lease.UserID, lease.DeviceID) {
				continue
			}
			items = append(items, item)
		}
		sortModelUsageRecordsForList(items, storageOptions.Sort)
		return items, nil
	})
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(result), offset, pageSize)
	return result[start:end], len(result), nil
}

func modelUsageMatchesListOptions(item controlplane.ModelUsageRecord, options store.ModelUsagePageOptions, userID, deviceID string) bool {
	if options.Provider != "" && item.Provider != options.Provider {
		return false
	}
	if options.Model != "" && item.Model != options.Model {
		return false
	}
	if options.UserID != "" && userID != options.UserID {
		return false
	}
	if options.DeviceID != "" && deviceID != options.DeviceID {
		return false
	}
	if options.RequestID != "" && item.RequestID != options.RequestID {
		return false
	}
	if options.CreatedAfter == nil && options.CreatedBefore == nil {
		return true
	}
	createdAt, err := time.Parse(time.RFC3339, item.CreatedAt)
	if err != nil {
		return false
	}
	if options.CreatedAfter != nil && createdAt.Before(*options.CreatedAfter) {
		return false
	}
	return options.CreatedBefore == nil || !createdAt.After(*options.CreatedBefore)
}

func sortModelUsageRecordsForList(items []controlplane.ModelUsageRecord, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.ModelUsageRecord) int {
		if a.CreatedAt != b.CreatedAt {
			parsedA, errA := time.Parse(time.RFC3339, a.CreatedAt)
			parsedB, errB := time.Parse(time.RFC3339, b.CreatedAt)
			if errA == nil && errB == nil {
				if parsedA.Before(parsedB) {
					if sortKey == store.ModelUsageSortCreatedAsc {
						return -1
					}
					return 1
				}
				if parsedA.After(parsedB) {
					if sortKey == store.ModelUsageSortCreatedAsc {
						return 1
					}
					return -1
				}
			} else if sortKey == store.ModelUsageSortCreatedAsc {
				return strings.Compare(a.CreatedAt, b.CreatedAt)
			} else {
				return strings.Compare(b.CreatedAt, a.CreatedAt)
			}
		}
		if sortKey == store.ModelUsageSortCreatedAsc {
			return strings.Compare(a.ID, b.ID)
		}
		return strings.Compare(b.ID, a.ID)
	})
}

func (s *ControlPlane) CreateActivationCode(ctx context.Context, idempotencyKey string, input controlplane.CreateActivationCodeInput) (controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ActivationCode{}, controlplane.ErrIdempotencyKeyRequired
	}
	if input.MaxDevices == 0 {
		input.MaxDevices = 1
	}
	if input.MaxDevices < 1 || input.MaxDevices > controlplane.MaxActivationCodeDevices || input.ExpiresAt.IsZero() || !input.ExpiresAt.After(s.repository.Now()) {
		return controlplane.ActivationCode{}, controlplane.ErrInvalidRequest
	}
	fingerprint, err := fingerprintValue(struct {
		ExpiresAt  string `json:"expires_at"`
		MaxDevices int    `json:"max_devices"`
	}{
		ExpiresAt:  input.ExpiresAt.UTC().Format(time.RFC3339),
		MaxDevices: input.MaxDevices,
	})
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ActivationRepository)
		if !ok {
			return controlplane.ActivationCode{}, store.ErrNormalizedActivationRepositoryRequired
		}
		plainCode, err := randomToken("code_", 18)
		if err != nil {
			return controlplane.ActivationCode{}, controlplane.NewError(http.StatusInternalServerError, "RANDOM_GENERATION_FAILED", "无法生成激活码")
		}
		return writer.CreateActivationCode(ctx, "control-plane-state", "create-activation-code:"+idempotencyKey, fingerprint, store.ActivationCodeCreateRecord{
			Product: controlplane.ProductAutoLive, PlainCode: plainCode, CodeHash: secretDigest(plainCode), CodePrefix: plainCode[:12], ExpiresAt: input.ExpiresAt, MaxDevices: input.MaxDevices, CreatedAt: s.repository.Now(),
		})
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ActivationCode, error) {
		if existing, ok := state.IdempotencyRecords["create-activation-code:"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
			}
			record := state.ActivationCodes[existing.ResourceID]
			if record.PlainCode == "" {
				// PostgreSQL 快照不会恢复明文激活码；重复请求只能返回
				// 已持久化的元数据，不能伪造一个空字符串指针。
				record.ActivationCode.PlainCode = nil
			} else {
				plainCode := record.PlainCode
				record.ActivationCode.PlainCode = &plainCode
			}
			record.ActivationCode.CodePrefix = record.CodePrefix
			return record.ActivationCode, nil
		}

		id := nextID(state, "ac")
		plainCode, err := randomToken("code_", 18)
		if err != nil {
			return controlplane.ActivationCode{}, controlplane.NewError(http.StatusInternalServerError, "RANDOM_GENERATION_FAILED", "无法生成激活码")
		}
		code := controlplane.ActivationCode{
			ID:           id,
			Product:      controlplane.ProductAutoLive,
			Status:       controlplane.ActivationCodeStatusActive,
			ExpiresAt:    input.ExpiresAt.UTC().Format(time.RFC3339),
			MaxDevices:   input.MaxDevices,
			BoundDevices: 0,
			CodePrefix:   plainCode[:12],
			PlainCode:    &plainCode,
		}
		state.ActivationCodes[id] = store.ActivationCodeRecord{
			ActivationCode: code,
			PlainCode:      plainCode,
			CodePrefix:     plainCode[:12],
		}
		state.ActivationCodeIndex[secretDigest(plainCode)] = id
		state.IdempotencyRecords["create-activation-code:"+idempotencyKey] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  id,
		}
		return code, nil
	})
}

func (s *ControlPlane) ListActivationCodes(ctx context.Context) ([]controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil, store.ErrNormalizedActivationPageReaderRequired
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ActivationCode, error) {
		items := make([]controlplane.ActivationCode, 0, len(state.ActivationCodes))
		now := s.repository.Now()
		for id, record := range state.ActivationCodes {
			code := decorateActivationCode(state, record)
			if code.Status == controlplane.ActivationCodeStatusActive {
				expiresAt, _ := time.Parse(time.RFC3339, code.ExpiresAt)
				if !now.Before(expiresAt) {
					code.Status = controlplane.ActivationCodeStatusExpired
					record.ActivationCode.Status = code.Status
					state.ActivationCodes[id] = record
				}
			}
			code.PlainCode = nil
			items = append(items, code)
		}
		slices.SortFunc(items, func(a, b controlplane.ActivationCode) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

// ListActivationCodesPage keeps the HTTP page boundary close to storage. A
// normalized PostgreSQL repository uses bounded SQL; compatibility stores
// retain the existing in-memory fallback until their migration is complete.
func (s *ControlPlane) ListActivationCodesPage(ctx context.Context, page, pageSize int) ([]controlplane.ActivationCode, int, error) {
	if err := checkContext(ctx); err != nil {
		return nil, 0, err
	}
	offset, err := pageOffset(page, pageSize)
	if err != nil {
		return nil, 0, err
	}
	useNormalizedReader := true
	if source, ok := s.repository.(store.NormalizedReadSource); ok {
		useNormalizedReader = source.UsesNormalizedReadSource()
	}
	if useNormalizedReader {
		if reader, ok := s.repository.(store.ActivationPageReader); ok {
			result, err := reader.ListActivationCodesPage(ctx, offset, pageSize)
			if err != nil {
				return nil, 0, err
			}
			return result.Items, result.Total, nil
		}
		if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
			return nil, 0, store.ErrNormalizedActivationPageReaderRequired
		}
	}
	items, err := s.ListActivationCodes(ctx)
	if err != nil {
		return nil, 0, err
	}
	start, end := pageWindow(len(items), offset, pageSize)
	return items[start:end], len(items), nil
}

func decorateActivationCode(state *store.State, record store.ActivationCodeRecord) controlplane.ActivationCode {
	code := record.ActivationCode
	code.PlainCode = nil
	code.CodePrefix = record.CodePrefix
	if code.MaxDevices == 0 {
		code.MaxDevices = 1
	}
	if code.BoundDevices == 0 && record.UsedByDeviceID != "" {
		code.BoundDevices = 1
	}
	code.UsedByUserID = record.UsedByUserID
	code.UsedByDeviceID = record.UsedByDeviceID
	code.UsedAt = record.UsedAt
	if record.UsedByDeviceID != "" {
		if code.UsedByUserID == "" {
			if device, ok := state.Devices[record.UsedByDeviceID]; ok {
				code.UsedByUserID = device.UserID
			}
		}
	}
	return code
}

func (s *ControlPlane) RevokeActivationCode(ctx context.Context, idempotencyKey, codeID string) (controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ActivationCode{}, controlplane.ErrIdempotencyKeyRequired
	}
	codeID = strings.TrimSpace(codeID)
	if codeID == "" {
		return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
	}
	fingerprint, err := fingerprintValue(struct {
		CodeID string `json:"code_id"`
	}{CodeID: codeID})
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		writer, ok := s.repository.(store.ActivationRepository)
		if !ok {
			return controlplane.ActivationCode{}, store.ErrNormalizedActivationRepositoryRequired
		}
		return writer.RevokeActivationCode(ctx, "control-plane-state", "revoke-activation-code:"+idempotencyKey, fingerprint, codeID)
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ActivationCode, error) {
		scope := "revoke-activation-code:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
			}
			record, exists := state.ActivationCodes[existing.ResourceID]
			if !exists {
				return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
			}
			record.ActivationCode.PlainCode = nil
			return decorateActivationCode(state, record), nil
		}

		record, ok := state.ActivationCodes[codeID]
		if !ok {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		if record.ActivationCode.Status != controlplane.ActivationCodeStatusActive && record.ActivationCode.Status != controlplane.ActivationCodeStatusRevoked {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeStateConflict
		}
		record.ActivationCode.Status = controlplane.ActivationCodeStatusRevoked
		record.PlainCode = ""
		record.ActivationCode.PlainCode = nil
		state.ActivationCodes[codeID] = record
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  codeID,
		}
		return decorateActivationCode(state, record), nil
	})
}

func (s *ControlPlane) ActivateDevice(ctx context.Context, idempotencyKey, userID string, input controlplane.ActivateDeviceInput) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		// This compatibility entry point has no authenticated session hash;
		// normalized callers must use ActivateDeviceWithSessionBinding.
		return controlplane.DeviceSummary{}, store.ErrNormalizedTransactionalDeviceActivatorRequired
	}
	return s.activateDeviceWithRunner(ctx, idempotencyKey, userID, input, func(operation store.StateOperation) error {
		return s.repository.Run(ctx, operation)
	})
}

// SupportsTransactionalSessionBinding reports whether the repository can
// atomically bind a SQL session with a device state mutation.
func (s *ControlPlane) SupportsTransactionalSessionBinding() bool {
	_, ok := s.repository.(store.TransactionalSessionBinder)
	return ok
}

// SupportsTransactionalDeviceLifecycle reports whether normalized device
// disable/unbind also revokes sessions and releases leases in the same SQL
// transaction. HTTP handlers use this to avoid issuing a second best-effort
// session write after the domain transaction has already committed.
func (s *ControlPlane) SupportsTransactionalDeviceLifecycle() bool {
	source, sourceOK := s.repository.(store.NormalizedReadSource)
	_, lifecycleOK := s.repository.(store.DeviceLifecycleRepository)
	return sourceOK && source.UsesNormalizedReadSource() && lifecycleOK
}

// ActivateDeviceWithSessionBinding is the PostgreSQL production path. The
// authenticated access-token hash is used only as a lookup key; it is never
// persisted or returned by the service layer.
func (s *ControlPlane) ActivateDeviceWithSessionBinding(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.ActivateDeviceInput) (controlplane.DeviceSummary, error) {
	return s.activateDeviceWithSessionBinding(ctx, idempotencyKey, userID, accessTokenHash, input, controlplane.AuditLogInput{})
}

// ActivateDeviceWithSessionBindingAndAudit is the normalized HTTP path. The
// success audit event is inserted into the same transaction as activation,
// device registration and session binding.
func (s *ControlPlane) ActivateDeviceWithSessionBindingAndAudit(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.ActivateDeviceInput, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	return s.activateDeviceWithSessionBinding(ctx, idempotencyKey, userID, accessTokenHash, input, audit)
}

func (s *ControlPlane) activateDeviceWithSessionBinding(ctx context.Context, idempotencyKey, userID, accessTokenHash string, input controlplane.ActivateDeviceInput, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		activator, ok := s.repository.(store.TransactionalDeviceActivator)
		if !ok {
			return controlplane.DeviceSummary{}, store.ErrNormalizedTransactionalDeviceActivatorRequired
		}
		if err := checkContext(ctx); err != nil {
			return controlplane.DeviceSummary{}, err
		}
		if strings.TrimSpace(userID) == "" {
			return controlplane.DeviceSummary{}, controlplane.ErrUserNotFound
		}
		if strings.TrimSpace(accessTokenHash) == "" {
			return controlplane.DeviceSummary{}, controlplane.ErrUnauthenticated
		}
		if !validIdempotencyKey(idempotencyKey) {
			return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
		}
		if err := validateActivateDeviceInput(input); err != nil {
			return controlplane.DeviceSummary{}, err
		}
		fingerprint, err := fingerprintValue(struct {
			UserID string                           `json:"user_id"`
			Input  controlplane.ActivateDeviceInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		device, err := activator.ActivateDeviceWithSessionBinding(ctx, store.DeviceActivationRecord{
			Scope:              "control-plane-state",
			IdempotencyKey:     "activate-device:" + userID + ":" + idempotencyKey,
			Fingerprint:        fingerprint,
			AccessTokenHash:    accessTokenHash,
			UserID:             userID,
			Product:            input.Device.Product,
			ActivationCodeHash: secretDigest(input.ActivationCode),
			Device:             input.Device,
			Audit:              audit,
		})
		if errors.Is(err, store.ErrSessionDeviceBindingConflict) {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingConflict
		}
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		return decorateDeviceSummary(device, s.repository.Now()), nil
	}
	binder, ok := s.repository.(store.TransactionalSessionBinder)
	if !ok {
		return s.ActivateDevice(ctx, idempotencyKey, userID, input)
	}
	return s.activateDeviceWithRunner(ctx, idempotencyKey, userID, input, func(operation store.StateOperation) error {
		err := binder.RunWithSessionBinding(ctx, accessTokenHash, userID, input.Device.DeviceID, operation)
		if errors.Is(err, store.ErrSessionDeviceBindingConflict) {
			return controlplane.ErrDeviceBindingConflict
		}
		return err
	})
}

func (s *ControlPlane) activateDeviceWithRunner(ctx context.Context, idempotencyKey, userID string, input controlplane.ActivateDeviceInput, run func(store.StateOperation) error) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateActivateDeviceInput(input); err != nil {
		return controlplane.DeviceSummary{}, err
	}

	device, err := runDeviceState(run, func(state *store.State) (controlplane.DeviceSummary, error) {
		fingerprint, err := fingerprintValue(struct {
			UserID string                           `json:"user_id"`
			Input  controlplane.ActivateDeviceInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "activate-device:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			device, exists := state.Devices[existing.ResourceID]
			if !exists {
				return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
			}
			return device, nil
		}
		user, exists := state.Users[userID]
		if !exists {
			return controlplane.DeviceSummary{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.DeviceSummary{}, controlplane.ErrUserDisabled
		}
		if existing, exists := state.Devices[input.Device.DeviceID]; exists {
			if existing.Status == controlplane.DeviceStatusPendingActivation && existing.UserID == "" {
				// 允许管理员解除绑定后的同一设备使用新激活码重新绑定。
			} else {
				if existing.UserID != userID || existing.Status == controlplane.DeviceStatusDisabled {
					return controlplane.DeviceSummary{}, controlplane.ErrForbidden
				}
				return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingConflict
			}
		}
		codeID, ok := state.ActivationCodeIndex[secretDigest(input.ActivationCode)]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeNotFound
		}
		record := state.ActivationCodes[codeID]
		expiresAt, _ := time.Parse(time.RFC3339, record.ActivationCode.ExpiresAt)
		if !s.repository.Now().Before(expiresAt) {
			record.ActivationCode.Status = controlplane.ActivationCodeStatusExpired
			state.ActivationCodes[codeID] = record
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeExpired
		}
		if record.ActivationCode.Status == controlplane.ActivationCodeStatusRevoked {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeRevoked
		}
		if record.ActivationCode.Status == controlplane.ActivationCodeStatusUsed || record.ActivationCode.BoundDevices >= record.ActivationCode.MaxDevices {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeAlreadyUsed
		}

		now := s.repository.Now().Format(time.RFC3339)
		device := controlplane.DeviceSummary{
			ID:                  input.Device.DeviceID,
			UserID:              userID,
			Product:             input.Device.Product,
			DeviceName:          strings.TrimSpace(input.Device.DeviceName),
			Platform:            strings.TrimSpace(input.Device.Platform),
			AppVersion:          strings.TrimSpace(input.Device.AppVersion),
			Status:              controlplane.DeviceStatusActive,
			LastSeenAt:          now,
			ActivationExpiresAt: formatActivationExpiryString(record.ActivationCode.ExpiresAt),
		}
		state.Devices[device.ID] = device
		record.ActivationCode.BoundDevices++
		if record.ActivationCode.BoundDevices >= record.ActivationCode.MaxDevices {
			record.ActivationCode.Status = controlplane.ActivationCodeStatusUsed
		} else {
			record.ActivationCode.Status = controlplane.ActivationCodeStatusActive
		}
		record.PlainCode = ""
		record.ActivationCode.PlainCode = nil
		record.ActivationCode.CodePrefix = record.CodePrefix
		if record.UsedByDeviceID == "" {
			record.ActivationCode.UsedByUserID = userID
			record.ActivationCode.UsedByDeviceID = device.ID
			record.ActivationCode.UsedAt = now
			record.UsedByUserID = userID
			record.UsedByDeviceID = device.ID
			record.UsedAt = now
		}
		state.ActivationCodes[codeID] = record
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: device.ID}
		return device, nil
	})
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	return decorateDeviceSummary(device, s.repository.Now()), nil
}

func (s *ControlPlane) DisableDevice(ctx context.Context, idempotencyKey, deviceID string) (controlplane.DeviceSummary, error) {
	return s.disableDevice(ctx, idempotencyKey, deviceID, controlplane.AuditLogInput{})
}

// DisableDeviceWithAudit is the normalized administrative HTTP path. The
// success event is committed with the device lifecycle mutation when the
// repository supports the transaction boundary.
func (s *ControlPlane) DisableDeviceWithAudit(ctx context.Context, idempotencyKey, deviceID string, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	return s.disableDevice(ctx, idempotencyKey, deviceID, audit)
}

func (s *ControlPlane) disableDevice(ctx context.Context, idempotencyKey, deviceID string, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		lifecycle, ok := s.repository.(store.DeviceLifecycleRepository)
		if !ok {
			return controlplane.DeviceSummary{}, store.ErrNormalizedDeviceLifecycleRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		device, err := lifecycle.DisableDevice(ctx, store.DeviceMutationRecord{
			Scope: "control-plane-state", IdempotencyKey: "disable-device:" + deviceID + ":" + idempotencyKey, Fingerprint: fingerprint, DeviceID: deviceID,
			Audit: audit,
		})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		return decorateDeviceSummary(device, s.repository.Now()), nil
	}
	device, err := withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "disable-device:" + deviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			releaseActiveModelLeasesForDevice(state, deviceID, s.repository.Now())
			return device, nil
		}
		if device.Status == controlplane.DeviceStatusDisabled {
			releaseActiveModelLeasesForDevice(state, deviceID, s.repository.Now())
			state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: deviceID}
			return device, nil
		}
		device.Status = controlplane.DeviceStatusDisabled
		releaseActiveModelLeasesForDevice(state, deviceID, s.repository.Now())
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: deviceID}
		return device, nil
	})
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	return decorateDeviceSummary(device, s.repository.Now()), nil
}

func (s *ControlPlane) UnbindDevice(ctx context.Context, idempotencyKey, deviceID string) (controlplane.DeviceSummary, error) {
	return s.unbindDevice(ctx, idempotencyKey, deviceID, controlplane.AuditLogInput{})
}

// UnbindDeviceWithAudit is the normalized administrative HTTP path. The
// success event is committed with the device lifecycle mutation.
func (s *ControlPlane) UnbindDeviceWithAudit(ctx context.Context, idempotencyKey, deviceID string, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	return s.unbindDevice(ctx, idempotencyKey, deviceID, audit)
}

func (s *ControlPlane) unbindDevice(ctx context.Context, idempotencyKey, deviceID string, audit controlplane.AuditLogInput) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		lifecycle, ok := s.repository.(store.DeviceLifecycleRepository)
		if !ok {
			return controlplane.DeviceSummary{}, store.ErrNormalizedDeviceLifecycleRepositoryRequired
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		device, err := lifecycle.UnbindDevice(ctx, store.DeviceMutationRecord{
			Scope: "control-plane-state", IdempotencyKey: "unbind-device:" + deviceID + ":" + idempotencyKey, Fingerprint: fingerprint, DeviceID: deviceID,
			Audit: audit,
		})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		return decorateDeviceSummary(device, s.repository.Now()), nil
	}
	device, err := withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "unbind-device:" + deviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			releaseActiveModelLeasesForDevice(state, deviceID, s.repository.Now())
			return state.Devices[existing.ResourceID], nil
		}
		releaseActiveModelLeasesForDevice(state, deviceID, s.repository.Now())
		device.UserID = ""
		device.Status = controlplane.DeviceStatusPendingActivation
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: device.ID}
		return device, nil
	})
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	return decorateDeviceSummary(device, s.repository.Now()), nil
}

func decorateDeviceSummary(device controlplane.DeviceSummary, now time.Time) controlplane.DeviceSummary {
	device.Online = false
	if device.Status != controlplane.DeviceStatusActive || strings.TrimSpace(device.LastSeenAt) == "" {
		return device
	}
	lastSeen, err := time.Parse(time.RFC3339, device.LastSeenAt)
	if err != nil {
		return device
	}
	device.Online = !lastSeen.After(now.Add(5*time.Second)) && now.Sub(lastSeen) <= deviceOnlineThreshold
	return device
}

func activationExpiryForDevice(state *store.State, userID, deviceID string) *string {
	for _, record := range state.ActivationCodes {
		if record.UsedByUserID == userID && record.UsedByDeviceID == deviceID {
			return formatActivationExpiryString(record.ActivationCode.ExpiresAt)
		}
	}
	return nil
}

func formatActivationExpiry(expiresAt *time.Time) *string {
	if expiresAt == nil {
		return nil
	}
	value := expiresAt.UTC().Format(time.RFC3339)
	return &value
}

func formatActivationExpiryString(expiresAt string) *string {
	parsed, err := time.Parse(time.RFC3339, strings.TrimSpace(expiresAt))
	if err != nil {
		return nil
	}
	return formatActivationExpiry(&parsed)
}

func validateCreateUserInput(input controlplane.CreateUserInput) error {
	username := strings.TrimSpace(input.Username)
	if len(username) < 3 || len(username) > 64 || len(input.Password) < 8 || len(input.Password) > 256 {
		return controlplane.ErrInvalidRequest
	}
	if input.Role != controlplane.RoleAdmin && input.Role != controlplane.RoleUser {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateUpdateUserInput(input *controlplane.UpdateUserInput) error {
	if input == nil || (input.Username == nil && input.Role == nil && input.Status == nil) {
		return controlplane.ErrInvalidRequest
	}
	if input.Username != nil {
		username := strings.TrimSpace(*input.Username)
		if len(username) < 3 || len(username) > 64 {
			return controlplane.ErrInvalidRequest
		}
		input.Username = &username
	}
	if input.Role != nil && *input.Role != controlplane.RoleAdmin && *input.Role != controlplane.RoleUser {
		return controlplane.ErrInvalidRequest
	}
	if input.Status != nil && *input.Status != controlplane.UserStatusActive && *input.Status != controlplane.UserStatusDisabled {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateUpdateUserAuthorizationInput(input *controlplane.UpdateUserAuthorizationInput) error {
	if input == nil || input.DailyTokenLimit < 0 || input.DailyTokenLimit > 1_000_000_000 || len(input.AllowedModels) > 100 {
		return controlplane.ErrInvalidRequest
	}
	seen := make(map[string]struct{}, len(input.AllowedModels))
	canonical := make([]string, 0, len(input.AllowedModels))
	for _, modelKey := range input.AllowedModels {
		modelKey = strings.TrimSpace(modelKey)
		separator := strings.IndexByte(modelKey, '/')
		if separator <= 0 || separator == len(modelKey)-1 || strings.IndexByte(modelKey[separator+1:], '/') >= 0 {
			return controlplane.ErrInvalidRequest
		}
		provider, model := strings.TrimSpace(modelKey[:separator]), strings.TrimSpace(modelKey[separator+1:])
		if len(provider) == 0 || len(provider) > 64 || len(model) == 0 || len(model) > 128 {
			return controlplane.ErrInvalidRequest
		}
		modelKey = provider + "/" + model
		if _, ok := seen[modelKey]; ok {
			return controlplane.ErrInvalidRequest
		}
		seen[modelKey] = struct{}{}
		canonical = append(canonical, modelKey)
	}
	slices.Sort(canonical)
	input.AllowedModels = canonical
	return nil
}

func countActiveAdmins(state *store.State, excludedID string, candidate controlplane.UserSummary) int {
	count := 0
	for id, user := range state.Users {
		if id == excludedID {
			user = candidate
		}
		if user.Role == controlplane.RoleAdmin && user.Status == controlplane.UserStatusActive {
			count++
		}
	}
	if _, exists := state.Users[excludedID]; !exists && candidate.Role == controlplane.RoleAdmin && candidate.Status == controlplane.UserStatusActive {
		count++
	}
	return count
}

func validateCreateModelPoolAccountInput(input *controlplane.CreateModelPoolAccountInput) error {
	input.Provider = strings.TrimSpace(input.Provider)
	input.Model = strings.TrimSpace(input.Model)
	input.BaseURL = strings.TrimRight(strings.TrimSpace(input.BaseURL), "/")
	input.APIKey = strings.TrimSpace(input.APIKey)
	if input.Status == "" {
		input.Status = controlplane.ModelAccountStatusActive
	}
	if len(input.Provider) == 0 || len(input.Provider) > 64 || len(input.Model) == 0 || len(input.Model) > 128 || len(input.APIKey) < 8 || len(input.APIKey) > 4096 {
		return controlplane.ErrInvalidRequest
	}
	if input.BaseURL != "" && !validModelBaseURL(input.BaseURL) {
		return controlplane.ErrInvalidRequest
	}
	if input.DailyLimit < 0 || input.ConcurrencyLimit < 1 {
		return controlplane.ErrInvalidRequest
	}
	if !validModelAccountStatus(input.Status) {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validModelAccountStatus(status string) bool {
	switch status {
	case controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown, controlplane.ModelAccountStatusExhausted, controlplane.ModelAccountStatusDisabled:
		return true
	default:
		return false
	}
}

func validateCreateModelLeaseInput(input *controlplane.CreateModelLeaseInput) error {
	input.Provider = strings.TrimSpace(input.Provider)
	input.Model = strings.TrimSpace(input.Model)
	input.Purpose = strings.TrimSpace(input.Purpose)
	if input.MaxDurationSeconds == 0 {
		input.MaxDurationSeconds = 300
	}
	if len(input.Provider) == 0 || len(input.Provider) > 64 || len(input.Model) == 0 || len(input.Model) > 128 {
		return controlplane.ErrInvalidRequest
	}
	switch input.Purpose {
	case "realtime_script", "chat", "validation":
	default:
		return controlplane.ErrInvalidRequest
	}
	if input.MaxDurationSeconds < 30 || input.MaxDurationSeconds > 3600 {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateRenewModelLeaseInput(input *controlplane.RenewModelLeaseInput) error {
	if input.ExtendSeconds == 0 {
		input.ExtendSeconds = 300
	}
	if input.ExtendSeconds < 30 || input.ExtendSeconds > 3600 {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func permissionsForRole(role string) []string {
	if role == controlplane.RoleAdmin {
		return []string{"admin", "client"}
	}
	return []string{"client"}
}

func resolveOwnedDevice(state *store.State, userID, deviceID string) (controlplane.DeviceSummary, error) {
	if deviceID != "" {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		if device.UserID != userID {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		return device, nil
	}

	var selected *controlplane.DeviceSummary
	for _, device := range state.Devices {
		if device.UserID != userID {
			continue
		}
		candidate := device
		if selected == nil || candidate.LastSeenAt > selected.LastSeenAt {
			selected = &candidate
		}
	}
	if selected == nil {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return *selected, nil
}

func resolveBoundOwnedDevice(state *store.State, userID, deviceID string) (controlplane.DeviceSummary, error) {
	if strings.TrimSpace(deviceID) == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingRequired
	}
	return resolveOwnedDevice(state, userID, deviceID)
}

func sweepExpiredModelLeases(state *store.State, now time.Time) {
	for id, lease := range state.ModelLeases {
		if lease.Status != controlplane.ModelLeaseStatusActive {
			continue
		}
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			continue
		}
		if !now.Before(expiresAt) {
			lease.Status = controlplane.ModelLeaseStatusExpired
			if lease.ReleasedAt == "" {
				lease.ReleasedAt = now.UTC().Format(time.RFC3339)
			}
			state.ModelLeases[id] = lease
		}
	}
}

func releaseActiveModelLeasesForDevice(state *store.State, deviceID string, now time.Time) {
	for id, lease := range state.ModelLeases {
		if lease.DeviceID == deviceID && lease.Status == controlplane.ModelLeaseStatusActive {
			lease.Status = controlplane.ModelLeaseStatusReleased
			lease.ReleasedAt = now.UTC().Format(time.RFC3339)
			state.ModelLeases[id] = lease
		}
	}
}

func selectModelPoolAccount(state *store.State, provider, model string, now time.Time) (controlplane.ModelPoolAccountSummary, bool) {
	refreshModelAccountStatuses(state, now)
	candidates := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
	for _, account := range state.ModelPoolAccounts {
		if account.Provider != provider || account.Model != model || account.Status != controlplane.ModelAccountStatusActive {
			continue
		}
		if !account.SecretConfigured || strings.TrimSpace(account.SecretRef) == "" {
			continue
		}
		if account.DailyLimit > 0 && dailyUsedTokensForAccount(state, account.ID, now) >= account.DailyLimit {
			continue
		}
		candidates = append(candidates, account)
	}
	slices.SortFunc(candidates, func(a, b controlplane.ModelPoolAccountSummary) int {
		if a.Priority != b.Priority {
			if a.Priority > b.Priority {
				return -1
			}
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
	for _, account := range candidates {
		if activeLeaseCountForAccount(state, account.ID) < account.ConcurrencyLimit {
			return account, true
		}
	}
	return controlplane.ModelPoolAccountSummary{}, false
}

func activeLeaseCountForAccount(state *store.State, accountID string) int {
	count := 0
	for _, lease := range state.ModelLeases {
		if lease.AccountID == accountID && lease.Status == controlplane.ModelLeaseStatusActive {
			count++
		}
	}
	return count
}

func dailyUsedTokensForAccount(state *store.State, accountID string, now time.Time) int {
	day := now.UTC().Format("2006-01-02")
	leaseAccountByID := make(map[string]string, len(state.ModelLeases))
	for id, lease := range state.ModelLeases {
		leaseAccountByID[id] = lease.AccountID
	}
	used := 0
	for _, usage := range state.ModelUsageRecords {
		if leaseAccountByID[usage.LeaseID] != accountID {
			continue
		}
		createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
		if err != nil || createdAt.UTC().Format("2006-01-02") != day {
			continue
		}
		used += usage.TotalTokens
	}
	return used
}

func dailyUsedTokensForUser(state *store.State, userID string, now time.Time) int {
	day := now.UTC().Format("2006-01-02")
	leaseUserByID := make(map[string]string, len(state.ModelLeases))
	for id, lease := range state.ModelLeases {
		leaseUserByID[id] = lease.UserID
	}
	used := 0
	for _, usage := range state.ModelUsageRecords {
		if leaseUserByID[usage.LeaseID] != userID {
			continue
		}
		createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
		if err != nil || createdAt.UTC().Format("2006-01-02") != day {
			continue
		}
		used += usage.TotalTokens
	}
	return used
}

func decorateModelPoolAccount(state *store.State, account controlplane.ModelPoolAccountSummary, now time.Time) controlplane.ModelPoolAccountSummary {
	account.ActiveLeases = activeLeaseCountForAccount(state, account.ID)
	account.DailyUsedTokens = dailyUsedTokensForAccount(state, account.ID, now)
	var latest controlplane.ModelPoolConnectivityTestResult
	for _, result := range state.ModelPoolTestResults {
		if result.AccountID != account.ID || result.TestedAt == "" || result.TestedAt <= latest.TestedAt {
			continue
		}
		latest = result
	}
	if latest.TestedAt != "" {
		account.LastTestStatus = latest.Status
		account.LastTestedAt = latest.TestedAt
		if account.Status == controlplane.ModelAccountStatusCooldown && (latest.Status == "failed" || latest.Status == "timeout") {
			if testedAt, err := time.Parse(time.RFC3339, latest.TestedAt); err == nil {
				account.CooldownUntil = testedAt.Add(modelAccountCooldownDuration).UTC().Format(time.RFC3339)
			}
		}
	}
	return account
}

func normalizeModelPoolAccountSummary(account *controlplane.ModelPoolAccountSummary, now time.Time) {
	if account == nil || account.Status == controlplane.ModelAccountStatusDisabled {
		return
	}
	switch account.Status {
	case controlplane.ModelAccountStatusActive:
		if account.DailyLimit > 0 && account.DailyUsedTokens >= account.DailyLimit {
			account.Status = controlplane.ModelAccountStatusExhausted
		}
	case controlplane.ModelAccountStatusExhausted:
		if account.DailyLimit > 0 && account.DailyUsedTokens < account.DailyLimit {
			account.Status = controlplane.ModelAccountStatusActive
		}
	case controlplane.ModelAccountStatusCooldown:
		cooldownUntil := account.CooldownUntil
		if cooldownUntil == "" && (account.LastTestStatus == "failed" || account.LastTestStatus == "timeout") {
			if testedAt, err := time.Parse(time.RFC3339, account.LastTestedAt); err == nil {
				cooldownUntil = testedAt.Add(modelAccountCooldownDuration).UTC().Format(time.RFC3339)
				account.CooldownUntil = cooldownUntil
			}
		}
		if cooldownUntil != "" {
			if until, err := time.Parse(time.RFC3339, cooldownUntil); err == nil && !now.Before(until) {
				account.Status = controlplane.ModelAccountStatusActive
				account.CooldownUntil = ""
			}
		}
	}
	if account.Status == controlplane.ModelAccountStatusActive && account.DailyLimit > 0 && account.DailyUsedTokens >= account.DailyLimit {
		account.Status = controlplane.ModelAccountStatusExhausted
	}
}

func modelLeaseAdminSummary(lease controlplane.ModelLease) controlplane.ModelLeaseAdminSummary {
	return controlplane.ModelLeaseAdminSummary{
		ID:               lease.ID,
		AccountID:        lease.AccountID,
		UserID:           lease.UserID,
		DeviceID:         lease.DeviceID,
		Purpose:          lease.Purpose,
		Provider:         lease.Provider,
		Model:            lease.Model,
		Status:           lease.Status,
		ExpiresAt:        lease.ExpiresAt,
		ProxyMode:        lease.ProxyMode,
		ConcurrencyLimit: lease.ConcurrencyLimit,
	}
}

func modelLeaseAdminDetail(lease controlplane.ModelLease) controlplane.ModelLeaseAdminDetail {
	return controlplane.ModelLeaseAdminDetail{
		ID:               lease.ID,
		AccountID:        lease.AccountID,
		UserID:           lease.UserID,
		DeviceID:         lease.DeviceID,
		Purpose:          lease.Purpose,
		Provider:         lease.Provider,
		Model:            lease.Model,
		Status:           lease.Status,
		CreatedAt:        lease.CreatedAt,
		ExpiresAt:        lease.ExpiresAt,
		ReleasedAt:       lease.ReleasedAt,
		ProxyMode:        lease.ProxyMode,
		ConcurrencyLimit: lease.ConcurrencyLimit,
	}
}

func normalizeModelLeaseAdminSummary(lease *controlplane.ModelLeaseAdminSummary, now time.Time) {
	if lease == nil || lease.Status != controlplane.ModelLeaseStatusActive || lease.ExpiresAt == "" {
		return
	}
	expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
	if err == nil && !now.Before(expiresAt) {
		lease.Status = controlplane.ModelLeaseStatusExpired
	}
}

func modelLeaseAdminSummaryMatchesOptions(item controlplane.ModelLeaseAdminSummary, options store.ModelLeasePageOptions, now time.Time) bool {
	normalizeModelLeaseAdminSummary(&item, now)
	if options.Status != "" && item.Status != options.Status {
		return false
	}
	return (options.Provider == "" || item.Provider == options.Provider) &&
		(options.Model == "" || item.Model == options.Model) &&
		(options.UserID == "" || item.UserID == options.UserID) &&
		(options.DeviceID == "" || item.DeviceID == options.DeviceID) &&
		(options.AccountID == "" || item.AccountID == options.AccountID)
}

func sortModelLeaseAdminSummaries(items []controlplane.ModelLeaseAdminSummary, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.ModelLeaseAdminSummary) int {
		switch sortKey {
		case store.ModelLeaseSortExpiresAsc:
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(a.ExpiresAt, b.ExpiresAt)
			}
		case store.ModelLeaseSortStatus:
			if a.Status != b.Status {
				return strings.Compare(a.Status, b.Status)
			}
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(b.ExpiresAt, a.ExpiresAt)
			}
		case store.ModelLeaseSortProviderModel:
			if value := strings.Compare(a.Provider, b.Provider); value != 0 {
				return value
			}
			if value := strings.Compare(a.Model, b.Model); value != 0 {
				return value
			}
		default:
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(b.ExpiresAt, a.ExpiresAt)
			}
		}
		return strings.Compare(b.ID, a.ID)
	})
}

func refreshModelAccountStatuses(state *store.State, now time.Time) {
	for id, account := range state.ModelPoolAccounts {
		if account.Status == controlplane.ModelAccountStatusDisabled {
			continue
		}
		used := dailyUsedTokensForAccount(state, id, now)
		switch account.Status {
		case controlplane.ModelAccountStatusActive:
			if account.DailyLimit > 0 && used >= account.DailyLimit {
				account.Status = controlplane.ModelAccountStatusExhausted
			}
		case controlplane.ModelAccountStatusExhausted:
			if account.DailyLimit > 0 && used < account.DailyLimit {
				account.Status = controlplane.ModelAccountStatusActive
			}
		case controlplane.ModelAccountStatusCooldown:
			cooldownUntil := account.CooldownUntil
			if cooldownUntil == "" {
				latest, ok := latestModelPoolTestResult(state, id)
				if ok && (latest.Status == "failed" || latest.Status == "timeout") {
					if testedAt, err := time.Parse(time.RFC3339, latest.TestedAt); err == nil {
						cooldownUntil = testedAt.Add(modelAccountCooldownDuration).UTC().Format(time.RFC3339)
					}
				}
			}
			if cooldownUntil != "" {
				if until, err := time.Parse(time.RFC3339, cooldownUntil); err == nil && !now.Before(until) {
					account.Status = controlplane.ModelAccountStatusActive
					account.CooldownUntil = ""
				}
			}
		}
		if account.Status == controlplane.ModelAccountStatusActive && account.DailyLimit > 0 && used >= account.DailyLimit {
			account.Status = controlplane.ModelAccountStatusExhausted
		}
		state.ModelPoolAccounts[id] = account
	}
}

func latestModelPoolTestResult(state *store.State, accountID string) (controlplane.ModelPoolConnectivityTestResult, bool) {
	var latest controlplane.ModelPoolConnectivityTestResult
	found := false
	for _, result := range state.ModelPoolTestResults {
		if result.AccountID != accountID || result.TestedAt == "" || (found && result.TestedAt <= latest.TestedAt) {
			continue
		}
		latest = result
		found = true
	}
	return latest, found
}
