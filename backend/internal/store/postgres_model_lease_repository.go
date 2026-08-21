package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ModelLeaseRepository = (*PostgresRepository)(nil)
var _ ModelLeaseCreator = (*PostgresRepository)(nil)

func (s *PostgresRepository) CreateModelLease(ctx context.Context, record ModelLeaseCreateRecord) (controlplane.ModelLease, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelLease{}, errors.New("normalized model lease creation requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ModelLease{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	record.DeviceID = strings.TrimSpace(record.DeviceID)
	record.Provider = strings.TrimSpace(record.Provider)
	record.Model = strings.TrimSpace(record.Model)
	record.Purpose = strings.TrimSpace(record.Purpose)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" || record.DeviceID == "" || record.Provider == "" || record.Model == "" || record.Purpose == "" || record.MaxDurationSeconds < 30 || record.MaxDurationSeconds > 3600 {
		return controlplane.ModelLease{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ModelLease{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelLease{}, err
	}
	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrUserDisabled
	}
	device, exists, err := s.loadDeviceForUpdate(operationCtx, tx, record.DeviceID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if !exists || device.UserID != record.UserID {
		return controlplane.ModelLease{}, controlplane.ErrDeviceNotFound
	}
	if device.Status != controlplane.DeviceStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
	}

	leaseID, err := newRepositoryID("lease")
	if err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model lease id: %w", err))
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, leaseID, s.Now())
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
		}
		lease, err := s.loadModelLeaseForUpdate(operationCtx, tx, storedResourceID)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
				return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease creation audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelLease{}, postgresCommitError(operationCtx, "commit idempotent normalized model lease creation", err)
		}
		return lease, nil
	}

	now := s.Now().UTC()
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE model_leases
		SET status = $1, released_at = COALESCE(released_at, $2)
		WHERE status = $3 AND expires_at <= $2
	`, controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive); err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("expire normalized model leases before creation: %w", err))
	}
	if err := s.refreshNormalizedModelAccountAvailability(operationCtx, tx, record.Provider, record.Model, now); err != nil {
		return controlplane.ModelLease{}, err
	}
	allowedModels, dailyTokenLimit, configured, err := s.loadNormalizedUserPolicyForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	modelKey := record.Provider + "/" + record.Model
	if configured && len(allowedModels) > 0 && !containsString(allowedModels, modelKey) {
		return controlplane.ModelLease{}, controlplane.ErrUserModelNotAuthorized
	}
	if configured && dailyTokenLimit > 0 {
		used, err := s.normalizedDailyUserTokens(operationCtx, tx, record.UserID, now)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if used >= dailyTokenLimit {
			return controlplane.ModelLease{}, controlplane.ErrUserRecordedQuotaExceeded
		}
	}

	dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	var accountID string
	if err := tx.QueryRowContext(operationCtx, `
		SELECT a.id
		FROM model_accounts a
		WHERE a.provider = $1 AND a.model = $2 AND a.status = $3 AND a.secret_ref <> ''
		  AND (a.daily_token_limit = 0 OR (
			SELECT COALESCE(SUM(u.total_tokens), 0)
			FROM model_usage_records u
			WHERE u.account_id = a.id AND u.created_at >= $5 AND u.created_at < $6
		  ) < a.daily_token_limit)
		  AND (
			SELECT COUNT(*)
			FROM model_leases l
			WHERE l.account_id = a.id AND l.status = $3 AND l.expires_at > $4
		  ) < a.concurrency_limit
		ORDER BY a.priority DESC, a.id
		LIMIT 1
		FOR UPDATE
	`, record.Provider, record.Model, controlplane.ModelAccountStatusActive, now, dayStart, dayEnd).Scan(&accountID); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("select normalized model account for lease: %w", err))
	}
	account, err := s.loadNormalizedModelPoolAccount(operationCtx, tx, accountID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if s.secretStore == nil {
		return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
	}
	if _, err := s.secretStore.Get(operationCtx, account.secretRef); err != nil {
		if operationCtx.Err() != nil {
			return controlplane.ModelLease{}, operationCtx.Err()
		}
		return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
	}

	lease := controlplane.ModelLease{
		ID: leaseID, UserID: record.UserID, DeviceID: record.DeviceID, AccountID: account.id,
		Purpose: record.Purpose, Provider: account.provider, Model: account.model,
		Status: controlplane.ModelLeaseStatusActive, CreatedAt: now.Format(time.RFC3339),
		ExpiresAt: now.Add(time.Duration(record.MaxDurationSeconds) * time.Second).Format(time.RFC3339),
		ProxyMode: controlplane.ModelLeaseProxyModeDirectLease, DirectBaseURL: account.baseURL,
		ConcurrencyLimit: account.concurrencyLimit,
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, released_at, provider, model, proxy_mode, concurrency_limit)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9, $10, $11, $12)
	`, lease.ID, lease.AccountID, lease.UserID, lease.DeviceID, lease.Purpose, lease.Status, now.Add(time.Duration(record.MaxDurationSeconds)*time.Second), now, lease.Provider, lease.Model, lease.ProxyMode, lease.ConcurrencyLimit); err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("create normalized model lease: %w", err))
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, now); err != nil {
			return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease creation audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelLease{}, postgresCommitError(operationCtx, "commit normalized model lease creation", err)
	}
	return lease, nil
}

func (s *PostgresRepository) loadNormalizedUserPolicyForUpdate(ctx context.Context, tx *sql.Tx, userID string) ([]string, int, bool, error) {
	var raw []byte
	var dailyLimit int
	err := tx.QueryRowContext(ctx, `
		SELECT allowed_models, daily_token_limit
		FROM user_authorization_policies
		WHERE user_id = $1
		FOR UPDATE
	`, userID).Scan(&raw, &dailyLimit)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, 0, false, nil
	}
	if err != nil {
		return nil, 0, false, postgresOperationError(ctx, fmt.Errorf("load normalized user authorization policy: %w", err))
	}
	var allowedModels []string
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &allowedModels); err != nil {
			return nil, 0, false, postgresOperationError(ctx, fmt.Errorf("decode normalized user authorization policy: %w", err))
		}
	}
	return allowedModels, dailyLimit, true, nil
}

func (s *PostgresRepository) normalizedDailyUserTokens(ctx context.Context, tx *sql.Tx, userID string, now time.Time) (int, error) {
	dayStart := time.Date(now.UTC().Year(), now.UTC().Month(), now.UTC().Day(), 0, 0, 0, 0, time.UTC)
	var used int
	if err := tx.QueryRowContext(ctx, `
		SELECT COALESCE(SUM(total_tokens), 0)
		FROM model_usage_records
		WHERE user_id = $1 AND created_at >= $2 AND created_at < $3
	`, userID, dayStart, dayStart.Add(24*time.Hour)).Scan(&used); err != nil {
		return 0, postgresOperationError(ctx, fmt.Errorf("load normalized user daily tokens: %w", err))
	}
	return used, nil
}

func (s *PostgresRepository) refreshNormalizedModelAccountAvailability(ctx context.Context, tx *sql.Tx, provider, model string, now time.Time) error {
	if _, err := tx.ExecContext(ctx, `
		UPDATE model_accounts
		SET status = $4, cooldown_until = NULL, updated_at = $3
		WHERE provider = $1 AND model = $2 AND status = $5 AND cooldown_until IS NOT NULL AND cooldown_until <= $3
	`, provider, model, now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("refresh normalized model account cooldown: %w", err))
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE model_accounts a
		SET status = $4, updated_at = $3
		WHERE a.provider = $1 AND a.model = $2 AND a.status = $5 AND a.daily_token_limit > 0
		  AND (
			SELECT COALESCE(SUM(u.total_tokens), 0)
			FROM model_usage_records u
			WHERE u.account_id = a.id AND u.created_at >= $6 AND u.created_at < $7
		  ) < a.daily_token_limit
	`, provider, model, now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusExhausted,
		time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC), time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC).Add(24*time.Hour)); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("refresh normalized model account daily availability: %w", err))
	}
	return nil
}

func containsString(values []string, target string) bool {
	for _, value := range values {
		if value == target {
			return true
		}
	}
	return false
}

func (s *PostgresRepository) RenewModelLease(ctx context.Context, record ModelLeaseRenewRecord) (controlplane.ModelLease, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelLease{}, errors.New("normalized model lease renewal requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ModelLease{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	record.DeviceID = strings.TrimSpace(record.DeviceID)
	record.LeaseID = strings.TrimSpace(record.LeaseID)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" || record.DeviceID == "" || record.LeaseID == "" || record.ExtendSeconds < 30 || record.ExtendSeconds > 3600 {
		return controlplane.ModelLease{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ModelLease{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelLease{}, err
	}
	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrUserDisabled
	}
	device, exists, err := s.loadDeviceForUpdate(operationCtx, tx, record.DeviceID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if !exists || device.UserID != record.UserID {
		return controlplane.ModelLease{}, controlplane.ErrDeviceNotFound
	}
	if device.Status != controlplane.DeviceStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
	}
	lease, err := s.loadModelLeaseForUpdate(operationCtx, tx, record.LeaseID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if lease.UserID != record.UserID || lease.DeviceID != device.ID {
		return controlplane.ModelLease{}, controlplane.ErrForbidden
	}
	if err := sweepNormalizedModelLease(operationCtx, tx, &lease, s.Now()); err != nil {
		return controlplane.ModelLease{}, err
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.LeaseID, s.Now())
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != record.LeaseID {
			return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
				return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease renewal audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelLease{}, postgresCommitError(operationCtx, "commit idempotent normalized model lease renewal", err)
		}
		return lease, nil
	}
	if lease.Status != controlplane.ModelLeaseStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
	}
	account, err := s.loadModelAccountForUpdate(operationCtx, tx, lease.AccountID)
	if err != nil {
		return controlplane.ModelLease{}, err
	}
	if account.status != controlplane.ModelAccountStatusActive {
		return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
	}
	expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
	if err != nil {
		return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
	}
	lease.ExpiresAt = expiresAt.Add(time.Duration(record.ExtendSeconds) * time.Second).UTC().Format(time.RFC3339)
	lease.DirectBaseURL = account.baseURL
	lease.Provider = account.provider
	lease.Model = account.model
	lease.ConcurrencyLimit = account.concurrencyLimit
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE model_leases SET expires_at = $2, provider = $3, model = $4, proxy_mode = $5, concurrency_limit = $6
		WHERE id = $1
	`, lease.ID, expiresAt.Add(time.Duration(record.ExtendSeconds)*time.Second), lease.Provider, lease.Model, lease.ProxyMode, lease.ConcurrencyLimit); err != nil {
		return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("renew normalized model lease: %w", err))
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
			return controlplane.ModelLease{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease renewal audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelLease{}, postgresCommitError(operationCtx, "commit normalized model lease renewal", err)
	}
	return lease, nil
}

func (s *PostgresRepository) ReleaseModelLease(ctx context.Context, record ModelLeaseReleaseRecord) (controlplane.ReleaseModelLeaseResult, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ReleaseModelLeaseResult{}, errors.New("normalized model lease release requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	record.DeviceID = strings.TrimSpace(record.DeviceID)
	record.LeaseID = strings.TrimSpace(record.LeaseID)
	record.Reason = strings.TrimSpace(record.Reason)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" || record.DeviceID == "" || record.LeaseID == "" || len(record.Reason) > 255 {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	return s.releaseModelLease(ctx, record.LeaseID, record.UserID, record.DeviceID, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Audit, false)
}

func (s *PostgresRepository) ReclaimModelLease(ctx context.Context, record ModelLeaseReclaimRecord) (controlplane.ReleaseModelLeaseResult, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ReleaseModelLeaseResult{}, errors.New("normalized model lease reclaim requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.LeaseID = strings.TrimSpace(record.LeaseID)
	record.Reason = strings.TrimSpace(record.Reason)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.LeaseID == "" || len(record.Reason) > 255 {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	return s.releaseModelLease(ctx, record.LeaseID, "", "", record.Scope, record.IdempotencyKey, record.Fingerprint, record.Audit, true)
}

func (s *PostgresRepository) releaseModelLease(ctx context.Context, leaseID, userID, deviceID, scope, idempotencyKey, fingerprint string, audit controlplane.AuditLogInput, reclaim bool) (controlplane.ReleaseModelLeaseResult, error) {
	if err := ctx.Err(); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ReleaseModelLeaseResult{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !reclaim {
		user, err := s.loadUserForUpdate(operationCtx, tx, userID)
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrUserDisabled
		}
		device, exists, err := s.loadDeviceForUpdate(operationCtx, tx, deviceID)
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		if !exists || device.UserID != userID {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrDeviceNotFound
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrDeviceDisabled
		}
	}
	lease, err := s.loadModelLeaseForUpdate(operationCtx, tx, leaseID)
	if err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !reclaim && (lease.UserID != userID || lease.DeviceID != deviceID) {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrForbidden
	}
	if err := sweepNormalizedModelLease(operationCtx, tx, &lease, s.Now()); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, leaseID, s.Now())
	if err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint || storedResourceID != leaseID {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyConflict
		}
		if strings.TrimSpace(audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ReleaseModelLeaseResult{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease release audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ReleaseModelLeaseResult{}, postgresCommitError(operationCtx, "commit idempotent normalized model lease release", err)
		}
		return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
	}
	if lease.Status != controlplane.ModelLeaseStatusActive && lease.Status != controlplane.ModelLeaseStatusReleased && lease.Status != controlplane.ModelLeaseStatusExpired {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrModelLeaseStateConflict
	}
	if lease.Status == controlplane.ModelLeaseStatusActive {
		now := s.Now()
		if _, err := tx.ExecContext(operationCtx, `
			UPDATE model_leases SET status = $2, released_at = $3 WHERE id = $1
		`, leaseID, controlplane.ModelLeaseStatusReleased, now); err != nil {
			return controlplane.ReleaseModelLeaseResult{}, postgresOperationError(operationCtx, fmt.Errorf("release normalized model lease: %w", err))
		}
	}
	if strings.TrimSpace(audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
			return controlplane.ReleaseModelLeaseResult{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model lease release audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, postgresCommitError(operationCtx, "commit normalized model lease release", err)
	}
	return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
}

type normalizedModelAccount struct {
	provider         string
	model            string
	baseURL          string
	status           string
	concurrencyLimit int
	dailyLimit       int
}

func (s *PostgresRepository) loadModelAccountForUpdate(ctx context.Context, tx *sql.Tx, accountID string) (normalizedModelAccount, error) {
	var account normalizedModelAccount
	if err := tx.QueryRowContext(ctx, `
		SELECT provider, model, base_url, status, concurrency_limit, daily_token_limit
		FROM model_accounts
		WHERE id = $1
		FOR UPDATE
	`, accountID).Scan(&account.provider, &account.model, &account.baseURL, &account.status, &account.concurrencyLimit, &account.dailyLimit); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return normalizedModelAccount{}, controlplane.ErrModelPoolUnavailable
		}
		return normalizedModelAccount{}, postgresOperationError(ctx, fmt.Errorf("lock normalized model account: %w", err))
	}
	return account, nil
}

func (s *PostgresRepository) loadModelLeaseForUpdate(ctx context.Context, tx *sql.Tx, leaseID string) (controlplane.ModelLease, error) {
	var lease controlplane.ModelLease
	var expiresAt, createdAt time.Time
	var releasedAt sql.NullTime
	if err := tx.QueryRowContext(ctx, `
		SELECT id, account_id, user_id, device_id, purpose, status, expires_at,
		       created_at, released_at, provider, model, proxy_mode, concurrency_limit
		FROM model_leases
		WHERE id = $1
		FOR UPDATE
	`, leaseID).Scan(&lease.ID, &lease.AccountID, &lease.UserID, &lease.DeviceID, &lease.Purpose, &lease.Status, &expiresAt, &createdAt, &releasedAt, &lease.Provider, &lease.Model, &lease.ProxyMode, &lease.ConcurrencyLimit); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseNotFound
		}
		return controlplane.ModelLease{}, postgresOperationError(ctx, fmt.Errorf("lock normalized model lease: %w", err))
	}
	lease.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	lease.ExpiresAt = expiresAt.UTC().Format(time.RFC3339)
	if releasedAt.Valid {
		lease.ReleasedAt = releasedAt.Time.UTC().Format(time.RFC3339)
	}
	return lease, nil
}

func sweepNormalizedModelLease(ctx context.Context, tx *sql.Tx, lease *controlplane.ModelLease, now time.Time) error {
	if lease == nil || lease.Status != controlplane.ModelLeaseStatusActive {
		return nil
	}
	expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
	if err != nil {
		return controlplane.ErrModelLeaseStateConflict
	}
	if now.Before(expiresAt) {
		return nil
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE model_leases SET status = $2, released_at = COALESCE(released_at, $3) WHERE id = $1 AND status = $4
	`, lease.ID, controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("expire normalized model lease: %w", err))
	}
	lease.Status = controlplane.ModelLeaseStatusExpired
	lease.ReleasedAt = now.UTC().Format(time.RFC3339)
	return nil
}
