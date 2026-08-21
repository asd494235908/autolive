package store

import (
	"context"
	"fmt"
	"time"

	"autoLive/backend/internal/controlplane"
)

func (s *MemoryStore) ReadOperationalMetrics(ctx context.Context) (OperationalMetrics, error) {
	var result OperationalMetrics
	err := s.Run(ctx, func(state *State) error {
		result = operationalMetricsFromState(state, s.Now())
		return nil
	})
	return result, err
}

func operationalMetricsFromState(state *State, now time.Time) OperationalMetrics {
	result := OperationalMetrics{ModelAccountStatusCounts: make(map[string]int64)}
	for _, account := range state.ModelPoolAccounts {
		result.ModelAccountStatusCounts[account.Status]++
	}
	for _, lease := range state.ModelLeases {
		if lease.Status != controlplane.ModelLeaseStatusActive {
			continue
		}
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil || !now.Before(expiresAt) {
			continue
		}
		result.ActiveModelLeases++
	}
	day := now.UTC().Format("2006-01-02")
	for _, usage := range state.ModelUsageRecords {
		createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
		if err == nil && createdAt.UTC().Format("2006-01-02") == day {
			result.DailyModelUsageTokens += int64(usage.TotalTokens)
		}
	}
	for _, policy := range state.UserAuthorizationPolicies {
		if policy.DailyTokenLimit > 0 || len(policy.AllowedModels) > 0 {
			result.ConfiguredUserAuthorizationPolicies++
		}
	}
	return result
}

func (s *PostgresRepository) ReadOperationalMetrics(ctx context.Context) (OperationalMetrics, error) {
	if err := ctx.Err(); err != nil {
		return OperationalMetrics{}, err
	}
	if s.modelReadSource == ModelReadSourceSnapshot {
		var result OperationalMetrics
		err := s.Run(ctx, func(state *State) error {
			result = operationalMetricsFromState(state, s.Now())
			return nil
		})
		return result, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(operationCtx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()`); err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, fmt.Errorf("lock operational metrics: %w", err))
	}
	result := OperationalMetrics{ModelAccountStatusCounts: make(map[string]int64)}
	now := s.Now().UTC()
	dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	rows, err := tx.QueryContext(operationCtx, `SELECT status, COUNT(*) FROM model_accounts GROUP BY status`)
	if err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	for rows.Next() {
		var status string
		var count int64
		if err := rows.Scan(&status, &count); err != nil {
			_ = rows.Close()
			return OperationalMetrics{}, postgresOperationError(operationCtx, err)
		}
		result.ModelAccountStatusCounts[status] = count
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	if err := rows.Close(); err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	if err := tx.QueryRowContext(operationCtx, `SELECT COUNT(*) FROM model_leases WHERE status = 'active' AND expires_at > $1`, now).Scan(&result.ActiveModelLeases); err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	if err := tx.QueryRowContext(operationCtx, `SELECT COALESCE(SUM(total_tokens), 0) FROM model_usage_records WHERE created_at >= $1 AND created_at < $2`, dayStart, dayEnd).Scan(&result.DailyModelUsageTokens); err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	if err := tx.QueryRowContext(operationCtx, `SELECT COUNT(*) FROM user_authorization_policies WHERE daily_token_limit > 0 OR allowed_models <> '[]'::jsonb`).Scan(&result.ConfiguredUserAuthorizationPolicies); err != nil {
		return OperationalMetrics{}, postgresOperationError(operationCtx, err)
	}
	return result, nil
}
