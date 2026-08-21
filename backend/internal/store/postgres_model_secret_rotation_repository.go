package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ModelPoolSecretRotator = (*PostgresRepository)(nil)

// RotateModelPoolAccountSecret commits a validated candidate secret, the
// active model-account reference, its probe result, idempotency fact and the
// optional success audit in one normalized PostgreSQL transaction. This keeps
// a failed commit from leaving a candidate that the account cannot reference,
// and prevents a successful commit from requiring a process-local cleanup.
func (s *PostgresRepository) RotateModelPoolAccountSecret(ctx context.Context, record ModelPoolSecretRotationRecord) (controlplane.ModelPoolAccountSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelPoolAccountSummary{}, errors.New("normalized model secret rotation requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccountID = strings.TrimSpace(record.AccountID)
	record.ExpectedSecretRef = strings.TrimSpace(record.ExpectedSecretRef)
	record.APIKey = strings.TrimSpace(record.APIKey)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccountID == "" || len(record.APIKey) < 8 || len(record.APIKey) > 4096 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if record.Probe.Status != "succeeded" || (strings.TrimSpace(record.Probe.AccountID) != "" && strings.TrimSpace(record.Probe.AccountID) != record.AccountID) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	account, err := s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.AccountID, s.Now())
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != record.AccountID {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
		}
		summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			audit := record.Audit
			if strings.TrimSpace(audit.TargetID) == "" {
				audit.TargetID = account.id
			}
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue idempotent model secret rotation audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit idempotent normalized model secret rotation", err)
		}
		return summary, nil
	}
	if record.ExpectedSecretRef != "" && account.secretRef != record.ExpectedSecretRef {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolSecretRotationConflict
	}
	secretWriter, ok := s.secretStore.(TransactionalSecretWriter)
	if !ok {
		return controlplane.ModelPoolAccountSummary{}, ErrTransactionalSecretStoreRequired
	}
	if account.secretRef == "" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
	}
	rotationID, err := newRepositoryID("rotation")
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model secret rotation reference: %w", err))
	}
	stagedSecretRef := account.secretRef + "/" + rotationID
	if err := secretWriter.PutTx(operationCtx, tx, stagedSecretRef, record.APIKey); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized rotated model secret: %w", err))
	}
	now := s.Now()
	testedAt := now
	if value := strings.TrimSpace(record.Probe.TestedAt); value != "" {
		parsed, parseErr := time.Parse(time.RFC3339, value)
		if parseErr != nil {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
		}
		testedAt = parsed
	}
	payload, err := json.Marshal(record.Probe)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("marshal normalized model secret probe: %w", err))
	}
	updateResult, err := tx.ExecContext(operationCtx, `
		UPDATE model_accounts
		SET secret_ref = $2, updated_at = $3
		WHERE id = $1 AND secret_ref = $4
	`, account.id, stagedSecretRef, now, account.secretRef)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("switch normalized model secret reference: %w", err))
	}
	if updated, err := updateResult.RowsAffected(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("verify normalized model secret reference: %w", err))
	} else if updated != 1 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolSecretRotationConflict
	}
	testID, err := newRepositoryID("model_test")
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model secret test id: %w", err))
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO model_pool_test_results (id, account_id, payload, created_at)
		VALUES ($1, $2, $3, $4)
	`, testID, account.id, payload, testedAt); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized model secret test result: %w", err))
	}
	deleteResult, err := tx.ExecContext(operationCtx, `
		DELETE FROM model_account_secrets
		WHERE secret_ref = $1
	`, account.secretRef)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("retire normalized model secret: %w", err))
	}
	if deleted, err := deleteResult.RowsAffected(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("verify retired normalized model secret: %w", err))
	} else if deleted != 1 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
	}
	account.secretRef = stagedSecretRef
	summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, now)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		audit := record.Audit
		if strings.TrimSpace(audit.TargetID) == "" {
			audit.TargetID = account.id
		}
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, now); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model secret rotation audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit normalized model secret rotation", err)
	}
	return summary, nil
}
