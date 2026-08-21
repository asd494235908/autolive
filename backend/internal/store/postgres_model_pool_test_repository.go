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
	account, err := s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	if err != nil {
		return ModelPoolTestPreparation{}, err
	}
	var storedFingerprint, resourceID string
	err = tx.QueryRowContext(operationCtx, `
		SELECT fingerprint, resource_id
		FROM idempotency_records
		WHERE scope = $1 AND idempotency_key = $2
	`, record.Scope, record.IdempotencyKey).Scan(&storedFingerprint, &resourceID)
	if err == nil {
		if storedFingerprint != record.Fingerprint {
			return ModelPoolTestPreparation{}, controlplane.ErrIdempotencyConflict
		}
		var payload []byte
		if err := tx.QueryRowContext(operationCtx, `SELECT payload FROM model_pool_test_results WHERE id = $1`, resourceID).Scan(&payload); err != nil {
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
	account, err := s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	resultID, err := newRepositoryID("model_test")
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model test id: %w", err))
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, resultID, s.Now())
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrIdempotencyConflict
		}
		var payload []byte
		if err := tx.QueryRowContext(operationCtx, `SELECT payload FROM model_pool_test_results WHERE id = $1`, storedResourceID).Scan(&payload); err != nil {
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
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO model_pool_test_results (id, account_id, payload, created_at)
		VALUES ($1, $2, $3, $4)
	`, resultID, record.AccountID, payload, testedAt); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized model test: %w", err))
	}
	if account.status != controlplane.ModelAccountStatusDisabled {
		if record.Result.Status == "succeeded" {
			account.status = controlplane.ModelAccountStatusActive
			account.cooldownUntil = ""
			if _, err := tx.ExecContext(operationCtx, `UPDATE model_accounts SET status = $2, cooldown_until = NULL, updated_at = $3 WHERE id = $1`, account.id, account.status, s.Now()); err != nil {
				return controlplane.ModelPoolConnectivityTestResult{}, postgresOperationError(operationCtx, fmt.Errorf("recover normalized model account after test: %w", err))
			}
		} else if record.Result.Status == "failed" || record.Result.Status == "timeout" {
			cooldownUntil := testedAt.Add(normalizedModelAccountCooldownDuration).UTC()
			account.status = controlplane.ModelAccountStatusCooldown
			account.cooldownUntil = cooldownUntil.Format(time.RFC3339)
			if _, err := tx.ExecContext(operationCtx, `UPDATE model_accounts SET status = $2, cooldown_until = $3, updated_at = $4 WHERE id = $1`, account.id, account.status, cooldownUntil, s.Now()); err != nil {
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
