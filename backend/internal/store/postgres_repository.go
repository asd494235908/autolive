package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"autoLive/backend/internal/controlplane"
)

// PostgresRepository 是控制面当前版本的事务适配器。
// 旧任务/阶段/模型代理表属于历史迁移兼容边界，新代码不再读写这些表。
type PostgresRepository struct {
	db          *sql.DB
	now         func() time.Time
	secretStore SecretStore
}

const (
	ModelReadSourceSnapshot   = "snapshot"
	ModelReadSourceNormalized = "normalized"
)

func NewPostgresRepository(db *sql.DB, now func() time.Time) (*PostgresRepository, error) {
	return NewPostgresRepositoryWithSecretStore(db, now, nil)
}

func NewPostgresRepositoryWithSecretStore(db *sql.DB, now func() time.Time, secretStore SecretStore) (*PostgresRepository, error) {
	if db == nil {
		return nil, errors.New("postgres repository database must not be nil")
	}
	if now == nil {
		now = time.Now
	}
	return &PostgresRepository{db: db, now: now, secretStore: secretStore}, nil
}

// 兼容启动配置的模型读源参数。当前只允许快照作为事实源，规范化模型域在后续阶段重新接入。
func NewPostgresRepositoryWithSecretStoreAndModelReadSource(db *sql.DB, now func() time.Time, secretStore SecretStore, modelReadSource string) (*PostgresRepository, error) {
	if modelReadSource != ModelReadSourceSnapshot && modelReadSource != ModelReadSourceNormalized {
		return nil, fmt.Errorf("unsupported model read source %q", modelReadSource)
	}
	return NewPostgresRepositoryWithSecretStore(db, now, secretStore)
}

func (s *PostgresRepository) Now() time.Time { return s.now().UTC() }

func (s *PostgresRepository) Run(ctx context.Context, fn StateOperation) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	state, err := s.loadLocked(ctx, tx)
	if err != nil {
		_ = tx.Rollback()
		return err
	}
	previousRefs := modelPoolSecretRefs(state)
	committed := false
	defer func() {
		_ = tx.Rollback()
		if !committed && s.secretStore != nil {
			for _, reference := range newModelPoolSecretRefs(previousRefs, modelPoolSecretRefs(state)) {
				_ = s.secretStore.Delete(context.Background(), reference)
			}
		}
	}()
	if err := fn(state); err != nil {
		return err
	}
	if err := s.syncReferenceRows(ctx, tx, state); err != nil {
		return fmt.Errorf("sync postgres reference rows: %w", err)
	}
	payload, err := marshalStateSnapshot(state)
	if err != nil {
		return fmt.Errorf("marshal postgres state snapshot: %w", err)
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE control_plane_state SET state = $1, updated_at = CURRENT_TIMESTAMP WHERE id = TRUE
	`, payload); err != nil {
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	committed = true
	return nil
}

func (s *PostgresRepository) loadLocked(ctx context.Context, tx *sql.Tx) (*State, error) {
	var raw []byte
	if err := tx.QueryRowContext(ctx, `SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE`).Scan(&raw); err != nil {
		return nil, err
	}
	return unmarshalStateSnapshot(raw)
}

func (s *PostgresRepository) syncReferenceRows(ctx context.Context, tx *sql.Tx, state *State) error {
	for id, user := range state.Users {
		passwordHash := string(state.UserCredentialHashes[id])
		if passwordHash == "" {
			passwordHash = "!configured-outside-control-plane!"
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO users (id, username, password_hash, role, status, created_at, disabled_at)
			VALUES ($1, $2, $3, $4, $5, $6, NULL)
			ON CONFLICT (id) DO UPDATE SET username = EXCLUDED.username, password_hash = CASE WHEN EXCLUDED.password_hash = '!configured-outside-control-plane!' THEN users.password_hash ELSE EXCLUDED.password_hash END, role = EXCLUDED.role, status = EXCLUDED.status
		`, id, user.Username, passwordHash, user.Role, user.Status, user.CreatedAt); err != nil {
			return err
		}
	}
	for id, device := range state.Devices {
		var userID any
		if device.UserID != "" {
			userID = device.UserID
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO devices (id, user_id, device_key, client_version, status, disk_free_bytes, memory_total_bytes, memory_available_bytes, cpu_logical_cores, runtime_os_name, runtime_os_version, kernel_version, last_heartbeat_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NULLIF($13, '')::timestamptz)
			ON CONFLICT (id) DO UPDATE SET user_id = EXCLUDED.user_id, client_version = EXCLUDED.client_version, status = EXCLUDED.status, disk_free_bytes = EXCLUDED.disk_free_bytes, memory_total_bytes = EXCLUDED.memory_total_bytes, memory_available_bytes = EXCLUDED.memory_available_bytes, cpu_logical_cores = EXCLUDED.cpu_logical_cores, runtime_os_name = EXCLUDED.runtime_os_name, runtime_os_version = EXCLUDED.runtime_os_version, kernel_version = EXCLUDED.kernel_version, last_heartbeat_at = EXCLUDED.last_heartbeat_at
		`, id, userID, "state-device/"+id, device.AppVersion, device.Status, device.DiskFreeBytes, device.MemoryTotalBytes, device.MemoryAvailableBytes, device.CPULogicalCores, device.RuntimeOSName, device.RuntimeOSVersion, device.KernelVersion, device.LastSeenAt); err != nil {
			return err
		}
	}
	for id, record := range state.ActivationCodes {
		digest, ok := activationCodeHash(state, id)
		if !ok {
			return fmt.Errorf("activation code %s has no hash index", id)
		}
		expiresAt, err := time.Parse(time.RFC3339, record.ActivationCode.ExpiresAt)
		if err != nil {
			return fmt.Errorf("activation code %s has invalid expiry: %w", id, err)
		}
		prefix := record.CodePrefix
		if prefix == "" {
			prefix = "code_"
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO activation_codes (id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_device_id)
			VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP, $5, CASE WHEN $4 = 'used' THEN CURRENT_TIMESTAMP ELSE NULL END, NULLIF($6, ''))
			ON CONFLICT (id) DO UPDATE SET code_hash = EXCLUDED.code_hash, code_prefix = EXCLUDED.code_prefix, status = EXCLUDED.status, expires_at = EXCLUDED.expires_at, used_by_device_id = EXCLUDED.used_by_device_id
		`, id, digest, prefix, record.ActivationCode.Status, expiresAt, record.UsedByDeviceID); err != nil {
			return err
		}
	}
	for id, account := range state.ModelPoolAccounts {
		if account.SecretRef == "" {
			return fmt.Errorf("model account %s has no secret reference", id)
		}
		activeLeases := activeLeasesForAccount(state, id)
		dailyUsedTokens := dailyUsedTokensForAccount(state, id, s.Now())
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, active_requests, daily_reserved_tokens, created_at, updated_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
			ON CONFLICT (id) DO UPDATE SET provider = EXCLUDED.provider, model = EXCLUDED.model, base_url = EXCLUDED.base_url, secret_ref = EXCLUDED.secret_ref, status = EXCLUDED.status, priority = EXCLUDED.priority, concurrency_limit = EXCLUDED.concurrency_limit, daily_token_limit = EXCLUDED.daily_token_limit, active_requests = EXCLUDED.active_requests, daily_reserved_tokens = EXCLUDED.daily_reserved_tokens, updated_at = CURRENT_TIMESTAMP
		`, id, account.Provider, account.Model, account.BaseURL, account.SecretRef, account.Status, account.Priority, account.ConcurrencyLimit, account.DailyLimit, activeLeases, dailyUsedTokens); err != nil {
			return err
		}
	}
	for id, lease := range state.ModelLeases {
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			return fmt.Errorf("model lease %s has invalid expiry: %w", id, err)
		}
		releasedAt := any(nil)
		if lease.Status == controlplane.ModelLeaseStatusReleased || lease.Status == controlplane.ModelLeaseStatusExpired {
			releasedAt = s.Now()
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, released_at, provider, model, proxy_mode, concurrency_limit)
			VALUES ($1, $2, $3, $4, $5, $6, $7, CURRENT_TIMESTAMP, $8, $9, $10, $11, $12)
			ON CONFLICT (id) DO UPDATE SET account_id = EXCLUDED.account_id, user_id = EXCLUDED.user_id, device_id = EXCLUDED.device_id, purpose = EXCLUDED.purpose, status = EXCLUDED.status, expires_at = EXCLUDED.expires_at, released_at = COALESCE(model_leases.released_at, EXCLUDED.released_at), provider = EXCLUDED.provider, model = EXCLUDED.model, proxy_mode = EXCLUDED.proxy_mode, concurrency_limit = EXCLUDED.concurrency_limit
		`, id, lease.AccountID, lease.UserID, lease.DeviceID, lease.Purpose, lease.Status, expiresAt, releasedAt, lease.Provider, lease.Model, controlplane.ModelLeaseProxyModeDirectLease, lease.ConcurrencyLimit); err != nil {
			return err
		}
	}
	for id, usage := range state.ModelUsageRecords {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_usage_records (id, account_id, lease_id, user_id, device_id, provider, model, prompt_tokens, completion_tokens, total_tokens, latency_ms, request_id, client_call_id, usage_source, status, error_code, created_at)
			SELECT $1, l.account_id, $2, l.user_id, l.device_id, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NULLIF($13, ''), $14 FROM model_leases l WHERE l.id = $2
			ON CONFLICT (id) DO UPDATE SET lease_id = EXCLUDED.lease_id, provider = EXCLUDED.provider, model = EXCLUDED.model, prompt_tokens = EXCLUDED.prompt_tokens, completion_tokens = EXCLUDED.completion_tokens, total_tokens = EXCLUDED.total_tokens, latency_ms = EXCLUDED.latency_ms, request_id = EXCLUDED.request_id, client_call_id = EXCLUDED.client_call_id, usage_source = EXCLUDED.usage_source, status = EXCLUDED.status, error_code = EXCLUDED.error_code
		`, id, usage.LeaseID, usage.Provider, usage.Model, usage.InputTokens, usage.OutputTokens, usage.TotalTokens, usage.LatencyMS, usage.RequestID, usage.ClientCallID, usage.UsageSource, usage.Status, usage.ErrorCode, usage.CreatedAt); err != nil {
			return err
		}
	}
	for scope, record := range state.IdempotencyRecords {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)
			VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
			ON CONFLICT (scope, idempotency_key) DO UPDATE SET fingerprint = EXCLUDED.fingerprint, resource_id = EXCLUDED.resource_id
		`, "control-plane-state", scope, record.Fingerprint, record.ResourceID); err != nil {
			return err
		}
	}
	for id, audit := range state.AuditLogs {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO audit_logs (id, actor_user_id, device_id, action, resource_type, resource_id, request_id, payload, created_at)
			VALUES ($1, NULLIF($2, ''), NULLIF($3, ''), $4, $5, NULLIF($6, ''), NULLIF($7, ''), '{}'::jsonb, $8)
			ON CONFLICT (id) DO NOTHING
		`, id, audit.ActorUserID, audit.DeviceID, audit.Action, audit.TargetType, audit.TargetID, audit.RequestID, audit.CreatedAt); err != nil {
			return err
		}
	}
	return nil
}

func activationCodeHash(state *State, codeID string) (string, bool) {
	for digest, id := range state.ActivationCodeIndex {
		if id == codeID {
			return digest, true
		}
	}
	return "", false
}

func activeLeasesForAccount(state *State, accountID string) int {
	count := 0
	for _, lease := range state.ModelLeases {
		if lease.AccountID == accountID && lease.Status == controlplane.ModelLeaseStatusActive {
			count++
		}
	}
	return count
}

func dailyUsedTokensForAccount(state *State, accountID string, now time.Time) int {
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

func modelPoolSecretRefs(state *State) map[string]struct{} {
	refs := map[string]struct{}{}
	for _, account := range state.ModelPoolAccounts {
		if account.SecretRef != "" {
			refs[account.SecretRef] = struct{}{}
		}
	}
	return refs
}

func newModelPoolSecretRefs(before, after map[string]struct{}) []string {
	refs := make([]string, 0)
	for ref := range after {
		if _, exists := before[ref]; !exists {
			refs = append(refs, ref)
		}
	}
	return refs
}

type stateSnapshot struct {
	Version int    `json:"version"`
	State   *State `json:"state,omitempty"`
}

func marshalStateSnapshot(state *State) ([]byte, error) {
	if state == nil {
		return nil, errors.New("state must not be nil")
	}
	copyState := *state
	copyState.ModelPoolAccounts = make(map[string]controlplane.ModelPoolAccountSummary, len(state.ModelPoolAccounts))
	for id, account := range state.ModelPoolAccounts {
		account.SecretRef = ""
		copyState.ModelPoolAccounts[id] = account
	}
	copyState.ActivationCodes = make(map[string]ActivationCodeRecord, len(state.ActivationCodes))
	for id, record := range state.ActivationCodes {
		record.PlainCode = ""
		record.ActivationCode.PlainCode = nil
		copyState.ActivationCodes[id] = record
	}
	return json.Marshal(stateSnapshot{Version: 1, State: &copyState})
}

func unmarshalStateSnapshot(raw []byte) (*State, error) {
	var snapshot stateSnapshot
	if err := json.Unmarshal(raw, &snapshot); err != nil {
		return nil, fmt.Errorf("decode postgres state snapshot: %w", err)
	}
	if snapshot.Version != 1 {
		return nil, fmt.Errorf("unsupported postgres state snapshot version %d", snapshot.Version)
	}
	if snapshot.State == nil {
		return nil, errors.New("postgres state snapshot is missing state payload")
	}
	state := ensureStateMaps(snapshot.State)
	for id, account := range state.ModelPoolAccounts {
		if account.SecretConfigured {
			account.SecretRef = "model-account/" + id
			state.ModelPoolAccounts[id] = account
		}
	}
	return state, nil
}
