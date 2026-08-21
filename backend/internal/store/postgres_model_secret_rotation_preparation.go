package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"

	"autoLive/backend/internal/controlplane"
)

var _ ModelPoolSecretRotationPreparer = (*PostgresRepository)(nil)

// PrepareModelPoolAccountSecretRotation keeps the normalized rotation
// pre-read bounded to the account and idempotency rows. It deliberately does
// not use Repository.Run, which would materialize and synchronize the legacy
// control-plane snapshot before the transactional rotation.
func (s *PostgresRepository) PrepareModelPoolAccountSecretRotation(ctx context.Context, scope, idempotencyKey, fingerprint, accountID string) (ModelPoolSecretRotationPreparation, error) {
	return s.prepareModelPoolAccountSecretRotation(ctx, scope, idempotencyKey, fingerprint, accountID, "", false)
}

func (s *PostgresRepository) PrepareModelPoolAccountSecretRotationForProduct(ctx context.Context, scope, idempotencyKey, fingerprint, accountID string, product controlplane.ProductCode) (ModelPoolSecretRotationPreparation, error) {
	return s.prepareModelPoolAccountSecretRotation(ctx, scope, idempotencyKey, fingerprint, accountID, product, true)
}

func (s *PostgresRepository) prepareModelPoolAccountSecretRotation(ctx context.Context, scope, idempotencyKey, fingerprint, accountID string, product controlplane.ProductCode, strictProduct bool) (ModelPoolSecretRotationPreparation, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return ModelPoolSecretRotationPreparation{}, errors.New("normalized model secret rotation preparation requires normalized read source")
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	accountID = strings.TrimSpace(accountID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || accountID == "" {
		return ModelPoolSecretRotationPreparation{}, controlplane.ErrInvalidRequest
	}
	if strictProduct && !product.Valid() {
		return ModelPoolSecretRotationPreparation{}, controlplane.ErrInvalidRequest
	}
	return runPostgresReadPage(s, ctx, func(operationCtx context.Context, tx *sql.Tx) (ModelPoolSecretRotationPreparation, error) {
		var account normalizedModelPoolAccount
		var err error
		if strictProduct {
			account, err = s.loadNormalizedModelPoolAccountWithProduct(operationCtx, tx, accountID, product)
		} else {
			account, err = s.loadNormalizedModelPoolAccount(operationCtx, tx, accountID)
		}
		if err != nil {
			return ModelPoolSecretRotationPreparation{}, err
		}
		summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
		if err != nil {
			return ModelPoolSecretRotationPreparation{}, err
		}
		// SecretRef is intentionally absent from ordinary account summaries, but
		// the transactional rotator needs the current reference for its compare
		// and swap guard. It never leaves this service/store boundary.
		summary.SecretRef = account.secretRef
		var storedFingerprint, storedResourceID string
		idempotencyQuery := `
			SELECT fingerprint, resource_id
			FROM idempotency_records
			WHERE scope = $1 AND idempotency_key = $2
		`
		idempotencyArgs := []any{scope, idempotencyKey}
		if strictProduct {
			idempotencyQuery += " AND product = $3"
			idempotencyArgs = append(idempotencyArgs, product)
		}
		err = tx.QueryRowContext(operationCtx, idempotencyQuery, idempotencyArgs...).Scan(&storedFingerprint, &storedResourceID)
		if errors.Is(err, sql.ErrNoRows) {
			return ModelPoolSecretRotationPreparation{Account: summary}, nil
		}
		if err != nil {
			return ModelPoolSecretRotationPreparation{}, postgresOperationError(operationCtx, fmt.Errorf("load normalized model secret rotation idempotency: %w", err))
		}
		if storedFingerprint != fingerprint || storedResourceID != accountID {
			return ModelPoolSecretRotationPreparation{}, controlplane.ErrIdempotencyConflict
		}
		return ModelPoolSecretRotationPreparation{Account: summary, Existing: &summary}, nil
	})
}
