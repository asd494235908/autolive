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

var _ ModelPoolTestRepository = (*PostgresRepository)(nil)

const normalizedModelAccountCooldownDuration = 5 * time.Minute

var ErrNormalizedModelPoolTestRepositoryRequired = errors.New("normalized model pool test repository is required")

func (s *PostgresRepository) PrepareModelPoolAccountTest(ctx context.Context, record ModelPoolTestPrepareRecord) (ModelPoolTestPreparation, error) {
	return s.prepareModelPoolAccountTest(ctx, record, false)
}

func (s *PostgresRepository) PrepareModelPoolAccountTestForProduct(ctx context.Context, record ModelPoolTestPrepareRecord) (ModelPoolTestPreparation, error) {
	return s.prepareModelPoolAccountTest(ctx, record, true)
}

func (s *PostgresRepository) prepareModelPoolAccountTest(ctx context.Context, record ModelPoolTestPrepareRecord, strictProduct bool) (ModelPoolTestPreparation, error) {
	if s.modelReadSource != ModelReadSourceNormalized || ctx == nil {
		return ModelPoolTestPreparation{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccountID = strings.TrimSpace(record.AccountID)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccountID == "" {
		return ModelPoolTestPreparation{}, controlplane.ErrInvalidRequest
	}
	if strictProduct && !record.Product.Valid() {
		return ModelPoolTestPreparation{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return ModelPoolTestPreparation{}, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return ModelPoolTestPreparation{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return ModelPoolTestPreparation{}, err
	}
	var account normalizedModelPoolAccount
	if strictProduct {
		account, err = s.loadNormalizedModelPoolAccountWithProduct(operationCtx, tx, record.AccountID, record.Product)
	} else {
		account, err = s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	}
	if err != nil {
		return ModelPoolTestPreparation{}, err
	}
	var storedFingerprint, resourceID string
	idempotencyQuery := `
		SELECT fingerprint, resource_id
		FROM idempotency_records
		WHERE scope = $1 AND idempotency_key = $2
	`
	idempotencyArgs := []any{record.Scope, record.IdempotencyKey}
	if strictProduct {
		idempotencyQuery += " AND product = $3"
		idempotencyArgs = append(idempotencyArgs, record.Product)
	}
	err = tx.QueryRowContext(operationCtx, idempotencyQuery, idempotencyArgs...).Scan(&storedFingerprint, &resourceID)
	if err == nil {
		if storedFingerprint != record.Fingerprint {
			return ModelPoolTestPreparation{}, controlplane.ErrIdempotencyConflict
		}
		var payload []byte
		query := `SELECT payload FROM model_pool_test_results WHERE id = $1`
		args := []any{resourceID}
		if strictProduct {
			filter, filterArgs := normalizedProductFilter("product", record.Product, 2)
			query = fmt.Sprintf("SELECT payload FROM model_pool_test_results WHERE id = $1 AND %s", filter)
			args = append(args, filterArgs...)
		}
		if err := tx.QueryRowContext(operationCtx, query, args...).Scan(&payload); err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return ModelPoolTestPreparation{}, controlplane.ErrInvalidRequest
			}
			return ModelPoolTestPreparation{}, postgresOperationError(operationCtx, fmt.Errorf("load idempotent normalized model test: %w", err))
		}
		var cached controlplane.ModelPoolConnectivityTestResult
		if err := json.Unmarshal(payload, &cached); err != nil {
			return ModelPoolTestPreparation{}, postgresOperationError(operationCtx, fmt.Errorf("decode idempotent normalized model test: %w", err))
		}
		if cached.AccountID != record.AccountID {
			return ModelPoolTestPreparation{}, controlplane.ErrInvalidRequest
		}
		summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
		if err != nil {
			return ModelPoolTestPreparation{}, err
		}
		summary.SecretRef = account.secretRef
		if err := tx.Commit(); err != nil {
			return ModelPoolTestPreparation{}, postgresCommitError(operationCtx, "commit normalized model test preparation", err)
		}
		return ModelPoolTestPreparation{Account: summary, Cached: &cached}, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return ModelPoolTestPreparation{}, postgresOperationError(operationCtx, fmt.Errorf("check normalized model test idempotency: %w", err))
	}
	summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
	if err != nil {
		return ModelPoolTestPreparation{}, err
	}
	summary.SecretRef = account.secretRef
	if err := tx.Commit(); err != nil {
		return ModelPoolTestPreparation{}, postgresCommitError(operationCtx, "commit normalized model test preparation", err)
	}
	return ModelPoolTestPreparation{Account: summary}, nil
}

func (s *PostgresRepository) RecordModelPoolAccountTest(ctx context.Context, record ModelPoolTestRecord) (controlplane.ModelPoolConnectivityTestResult, error) {
	return s.recordModelPoolAccountTest(ctx, record, false)
}

func (s *PostgresRepository) RecordModelPoolAccountTestForProduct(ctx context.Context, record ModelPoolTestRecord) (controlplane.ModelPoolConnectivityTestResult, error) {
	return s.recordModelPoolAccountTest(ctx, record, true)
}

func (s *PostgresRepository) recordModelPoolAccountTest(ctx context.Context, record ModelPoolTestRecord, strictProduct bool) (controlplane.ModelPoolConnectivityTestResult, error) {
	if s.modelReadSource != ModelReadSourceNormalized || ctx == nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccountID = strings.TrimSpace(record.AccountID)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccountID == "" || strings.TrimSpace(record.Result.AccountID) != record.AccountID {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	if strictProduct && !record.Product.Valid() {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	if strictProduct {
		audit, err := normalizeOptionalAuditInputForProduct(record.Audit, record.Product)
		if err != nil {
			return controlplane.ModelPoolConnectivityTestResult{}, err
		}
		record.Audit = audit
	}
	if !validModelPoolTestStatus(record.Result.Status) || strings.TrimSpace(record.Result.TestedAt) == "" {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	testedAt, err := time.Parse(time.RFC3339, record.Result.TestedAt)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	var account normalizedModelPoolAccount
	if strictProduct {
		account, err = s.loadNormalizedModelPoolAccountWithProduct(operationCtx, tx, record.AccountID, record.Product)
	} else {
		account, err = s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	}
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	resultID, err := newRepositoryID("model_test")
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model test id: %w", err))
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if strictProduct {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, resultID, s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, resultID, s.Now())
	}
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrIdempotencyConflict
		}
		var payload []byte
		query := `SELECT payload FROM model_pool_test_results WHERE id = $1`
		args := []any{storedResourceID}
		if strictProduct {
			filter, filterArgs := normalizedProductFilter("product", record.Product, 2)
			query = fmt.Sprintf("SELECT payload FROM model_pool_test_results WHERE id = $1 AND %s", filter)
			args = append(args, filterArgs...)
		}
		if err := tx.QueryRowContext(operationCtx, query, args...).Scan(&payload); err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
			}
			return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("load normalized model test replay: %w", err))
		}
		var cached controlplane.ModelPoolConnectivityTestResult
		if err := json.Unmarshal(payload, &cached); err != nil {
			return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("decode normalized model test replay: %w", err))
		}
		if cached.AccountID != record.AccountID {
			return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelPoolConnectivityTestResult{}, postgresCommitError(operationCtx, "commit idempotent normalized model test", err)
		}
		return cached, nil
	}
	payload, err := json.Marshal(record.Result)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("marshal normalized model test: %w", err))
	}
	var insertErr error
	if strictProduct {
		_, insertErr = tx.ExecContext(operationCtx, `
			INSERT INTO model_pool_test_results (id, product, account_id, payload, created_at)
			VALUES ($1, $2, $3, $4, $5)
		`, resultID, record.Product, record.AccountID, payload, testedAt)
	} else {
		_, insertErr = tx.ExecContext(operationCtx, `
			INSERT INTO model_pool_test_results (id, account_id, payload, created_at)
			VALUES ($1, $2, $3, $4)
		`, resultID, record.AccountID, payload, testedAt)
	}
	if insertErr != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized model test: %w", insertErr))
	}
	if account.status != controlplane.ModelAccountStatusDisabled {
		if record.Result.Status == "succeeded" {
			account.status = controlplane.ModelAccountStatusActive
			account.cooldownUntil = ""
			query := `UPDATE model_accounts SET status = $2, cooldown_until = NULL, updated_at = $3 WHERE id = $1`
			args := []any{account.id, account.status, s.Now()}
			if strictProduct {
				filter, filterArgs := normalizedProductFilter("product", record.Product, len(args)+1)
				query += " AND " + filter
				args = append(args, filterArgs...)
			}
			if _, err := tx.ExecContext(operationCtx, query, args...); err != nil {
				return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("recover normalized model account after test: %w", err))
			}
		} else if record.Result.Status == "failed" || record.Result.Status == "timeout" {
			cooldownUntil := testedAt.Add(normalizedModelAccountCooldownDuration).UTC()
			account.status = controlplane.ModelAccountStatusCooldown
			account.cooldownUntil = cooldownUntil.Format(time.RFC3339)
			query := `UPDATE model_accounts SET status = $2, cooldown_until = $3, updated_at = $4 WHERE id = $1`
			args := []any{account.id, account.status, cooldownUntil, s.Now()}
			if strictProduct {
				filter, filterArgs := normalizedProductFilter("product", record.Product, len(args)+1)
				query += " AND " + filter
				args = append(args, filterArgs...)
			}
			if _, err := tx.ExecContext(operationCtx, query, args...); err != nil {
				return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("cooldown normalized model account after test: %w", err))
			}
		}
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		audit := record.Audit
		if strings.TrimSpace(audit.TargetID) == "" {
			audit.TargetID = record.AccountID
		}
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
			return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue normalized model test audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresCommitError(operationCtx, "commit normalized model test", err)
	}
	return record.Result, nil
}

func validModelPoolTestStatus(status string) bool {
	switch status {
	case "succeeded", "failed", "timeout":
		return true
	default:
		return false
	}
}
