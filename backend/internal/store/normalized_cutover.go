package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// NormalizedCutoverTableCheck is a bounded read-only check for one table that
// must be available before the control plane can switch to normalized reads.
type NormalizedCutoverTableCheck struct {
	Name string
	Rows int64
}

// NormalizedCutoverReport contains only aggregate, non-sensitive readiness
// facts. It deliberately excludes URLs, user names, SecretRefs and payloads.
type NormalizedCutoverReport struct {
	BackfillStatus          string
	BackfillCompletedAt     time.Time
	Tables                  []NormalizedCutoverTableCheck
	MissingSecretReferences int64
}

var ErrNormalizedCutoverRequiresNormalizedSource = errors.New("normalized cutover verification requires normalized read source")

var normalizedCutoverTables = []string{
	"normalized_backfill_state",
	"users",
	"user_authorization_policies",
	"devices",
	"activation_codes",
	"activation_device_bindings",
	"auth_sessions",
	"model_accounts",
	"model_account_secrets",
	"model_leases",
	"model_usage_records",
	"model_pool_test_results",
	"idempotency_records",
	"audit_logs",
	"audit_outbox",
}

// VerifyNormalizedCutover performs a read-only preflight for the production
// normalized-read switch. It never changes the marker, snapshot or business
// rows, so a failed check is safe to repeat during a release.
func (s *PostgresRepository) VerifyNormalizedCutover(ctx context.Context) (NormalizedCutoverReport, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return NormalizedCutoverReport{}, ErrNormalizedCutoverRequiresNormalizedSource
	}
	if ctx == nil {
		return NormalizedCutoverReport{}, errors.New("normalized cutover verification context must not be nil")
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return NormalizedCutoverReport{}, postgresOperationError(operationCtx, fmt.Errorf("begin normalized cutover verification: %w", err))
	}
	defer func() { _ = tx.Rollback() }()

	var report NormalizedCutoverReport
	if err := tx.QueryRowContext(operationCtx, `
		SELECT status, COALESCE(completed_at, TIMESTAMPTZ 'epoch')
		  FROM public.normalized_backfill_state
	 WHERE id = TRUE
	`).Scan(&report.BackfillStatus, &report.BackfillCompletedAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return NormalizedCutoverReport{}, errors.New("normalized backfill state is missing")
		}
		return NormalizedCutoverReport{}, postgresOperationError(operationCtx, fmt.Errorf("read normalized backfill state: %w", err))
	}
	if report.BackfillStatus != "completed" || report.BackfillCompletedAt.IsZero() || report.BackfillCompletedAt.Equal(time.Unix(0, 0).UTC()) {
		return NormalizedCutoverReport{}, fmt.Errorf("normalized backfill is not complete (status %q)", report.BackfillStatus)
	}

	report.Tables = make([]NormalizedCutoverTableCheck, 0, len(normalizedCutoverTables))
	for _, table := range normalizedCutoverTables {
		var relation sql.NullString
		relationName := "public." + table
		if err := tx.QueryRowContext(operationCtx, `SELECT to_regclass($1)`, relationName).Scan(&relation); err != nil {
			return NormalizedCutoverReport{}, postgresOperationError(operationCtx, fmt.Errorf("check normalized table %s: %w", table, err))
		}
		if !relation.Valid || relation.String == "" {
			return NormalizedCutoverReport{}, fmt.Errorf("normalized table %s is missing", table)
		}
		var rows int64
		if err := tx.QueryRowContext(operationCtx, `SELECT COUNT(*) FROM public.`+table).Scan(&rows); err != nil {
			return NormalizedCutoverReport{}, postgresOperationError(operationCtx, fmt.Errorf("count normalized table %s: %w", table, err))
		}
		report.Tables = append(report.Tables, NormalizedCutoverTableCheck{Name: table, Rows: rows})
	}
	if err := tx.QueryRowContext(operationCtx, `
		SELECT COUNT(*)
		  FROM public.model_accounts a
		  LEFT JOIN public.model_account_secrets s ON s.secret_ref = a.secret_ref
		 WHERE a.secret_ref <> '' AND s.secret_ref IS NULL
	`).Scan(&report.MissingSecretReferences); err != nil {
		return NormalizedCutoverReport{}, postgresOperationError(operationCtx, fmt.Errorf("check normalized model secret references: %w", err))
	}
	if report.MissingSecretReferences != 0 {
		return NormalizedCutoverReport{}, fmt.Errorf("normalized model accounts have %d missing secret references", report.MissingSecretReferences)
	}
	return report, nil
}
