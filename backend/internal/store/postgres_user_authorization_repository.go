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

var _ UserAuthorizationRepository = (*PostgresRepository)(nil)
var _ UserAuthorizationSummaryReader = (*PostgresRepository)(nil)

var ErrNormalizedUserAuthorizationSummaryReaderRequired = errors.New("normalized user authorization summary reader is required")

// GetUserAuthorizationSummary aggregates only normalized user, device, lease,
// policy and client-reported usage facts. Provider-authoritative quota is not
// inferred from these records.
func (s *PostgresRepository) GetUserAuthorizationSummary(ctx context.Context, userID string) (controlplane.UserAuthorizationSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserAuthorizationSummary{}, ErrNormalizedUserAuthorizationSummaryReaderRequired
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserAuthorizationSummary{}, controlplane.ErrUserNotFound
	}
	now := s.Now()
	dayStart := time.Date(now.UTC().Year(), now.UTC().Month(), now.UTC().Day(), 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (controlplane.UserAuthorizationSummary, error) {
		var exists bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)`, userID).Scan(&exists); err != nil {
			return controlplane.UserAuthorizationSummary{}, err
		}
		if !exists {
			return controlplane.UserAuthorizationSummary{}, controlplane.ErrUserNotFound
		}

		summary := controlplane.UserAuthorizationSummary{
			UserID:              userID,
			UsageSource:         "client_reported_soft",
			QuotaEnforcement:    "server_recorded_usage_guard",
			HardQuotaConfigured: false,
			AllowedModels:       []string{},
			AsOf:                now.Format(time.RFC3339),
		}
		var allowedModels []byte
		var dailyTokenLimit int
		err := tx.QueryRowContext(ctx, `
			SELECT allowed_models, daily_token_limit
			FROM user_authorization_policies
			WHERE user_id = $1
		`, userID).Scan(&allowedModels, &dailyTokenLimit)
		if err == nil {
			if err := json.Unmarshal(allowedModels, &summary.AllowedModels); err != nil {
				return controlplane.UserAuthorizationSummary{}, fmt.Errorf("decode normalized authorization summary policy: %w", err)
			}
			summary.DailyTokenLimit = dailyTokenLimit
		} else if !errors.Is(err, sql.ErrNoRows) {
			return controlplane.UserAuthorizationSummary{}, err
		}

		if err := tx.QueryRowContext(ctx, `
			SELECT COUNT(*), COUNT(*) FILTER (WHERE status = 'active')
			FROM devices
			WHERE user_id = $1
		`, userID).Scan(&summary.DeviceCount, &summary.ActiveDeviceCount); err != nil {
			return controlplane.UserAuthorizationSummary{}, err
		}
		if err := tx.QueryRowContext(ctx, `
			SELECT COUNT(*) FILTER (WHERE status = 'active' AND expires_at > $2),
			       COUNT(DISTINCT CASE WHEN status = 'active' AND expires_at > $2 THEN account_id END)
			FROM model_leases
			WHERE user_id = $1
		`, userID, now).Scan(&summary.ActiveLeaseCount, &summary.ActiveAccountCount); err != nil {
			return controlplane.UserAuthorizationSummary{}, err
		}
		if err := tx.QueryRowContext(ctx, `
			SELECT COALESCE(SUM(u.total_tokens), 0)
			FROM model_usage_records u
			WHERE u.user_id = $1
			  AND u.created_at >= $2
			  AND u.created_at < $3
			  AND EXISTS (SELECT 1 FROM model_leases l WHERE l.id = u.lease_id AND l.user_id = $1)
		`, userID, dayStart, dayEnd).Scan(&summary.DailyUsedTokens); err != nil {
			return controlplane.UserAuthorizationSummary{}, err
		}
		return summary, nil
	})
}

// UpdateUserAuthorization writes the normalized policy and its idempotency
// record in one transaction. The user row is locked first so a concurrent
// disable/delete cannot leave a policy without its owning user.
func (s *PostgresRepository) UpdateUserAuthorization(ctx context.Context, scope, idempotencyKey, fingerprint, userID string, input controlplane.UpdateUserAuthorizationInput) (controlplane.UserAuthorizationPolicy, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserAuthorizationPolicy{}, errors.New("normalized authorization repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	userID = strings.TrimSpace(userID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || userID == "" || input.DailyTokenLimit < 0 || input.DailyTokenLimit > 1_000_000_000 || len(input.AllowedModels) > 100 {
		return controlplane.UserAuthorizationPolicy{}, errors.New("normalized authorization update arguments are invalid")
	}
	allowedModels := append([]string(nil), input.AllowedModels...)
	payload, err := json.Marshal(allowedModels)
	if err != nil {
		return controlplane.UserAuthorizationPolicy{}, fmt.Errorf("marshal normalized authorization policy: %w", err)
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.UserAuthorizationPolicy{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	if _, err := s.loadUserForUpdate(operationCtx, tx, userID); err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, userID, s.Now())
	if err != nil {
		return controlplane.UserAuthorizationPolicy{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserAuthorizationPolicy{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserAuthorizationPolicy(operationCtx, tx, storedResourceID)
	}
	now := s.Now()
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO user_authorization_policies (user_id, allowed_models, daily_token_limit, updated_at)
		VALUES ($1, $2::jsonb, $3, $4)
		ON CONFLICT (user_id) DO UPDATE SET allowed_models = EXCLUDED.allowed_models, daily_token_limit = EXCLUDED.daily_token_limit, updated_at = EXCLUDED.updated_at
	`, userID, payload, input.DailyTokenLimit, now); err != nil {
		return controlplane.UserAuthorizationPolicy{}, postgresOperationError(operationCtx, fmt.Errorf("write normalized authorization policy: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserAuthorizationPolicy{}, postgresCommitError(operationCtx, "commit normalized authorization policy", err)
	}
	return controlplane.UserAuthorizationPolicy{UserID: userID, AllowedModels: allowedModels, DailyTokenLimit: input.DailyTokenLimit, UpdatedAt: now.Format(time.RFC3339)}, nil
}

func (s *PostgresRepository) loadUserAuthorizationPolicy(ctx context.Context, tx *sql.Tx, userID string) (controlplane.UserAuthorizationPolicy, error) {
	var policy controlplane.UserAuthorizationPolicy
	var payload []byte
	var updatedAt time.Time
	if err := tx.QueryRowContext(ctx, `
		SELECT user_id, allowed_models, daily_token_limit, updated_at
		FROM user_authorization_policies
		WHERE user_id = $1
	`, userID).Scan(&policy.UserID, &payload, &policy.DailyTokenLimit, &updatedAt); err != nil {
		return controlplane.UserAuthorizationPolicy{}, postgresOperationError(ctx, fmt.Errorf("load idempotent normalized authorization policy: %w", err))
	}
	if err := json.Unmarshal(payload, &policy.AllowedModels); err != nil {
		return controlplane.UserAuthorizationPolicy{}, fmt.Errorf("decode normalized authorization policy: %w", err)
	}
	policy.UpdatedAt = updatedAt.UTC().Format(time.RFC3339)
	return policy, nil
}
