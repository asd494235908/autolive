package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ActivationRepository = (*PostgresRepository)(nil)

// CreateActivationCode persists only the digest of the one-time code. The
// plaintext is returned for the first response and is never stored in the
// normalized table or the idempotency record.
func (s *PostgresRepository) CreateActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint string, record ActivationCodeCreateRecord) (controlplane.ActivationCode, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ActivationCode{}, errors.New("normalized activation repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ActivationCode{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	record.PlainCode = strings.TrimSpace(record.PlainCode)
	record.CodeHash = strings.TrimSpace(record.CodeHash)
	record.CodePrefix = strings.TrimSpace(record.CodePrefix)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || record.PlainCode == "" || record.CodeHash == "" || record.CodePrefix == "" || record.ExpiresAt.IsZero() || record.MaxDevices != 1 {
		return controlplane.ActivationCode{}, errors.New("normalized activation create arguments are invalid")
	}
	createdAt := record.CreatedAt.UTC()
	if createdAt.IsZero() {
		createdAt = s.Now()
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ActivationCode{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	codeID, err := newRepositoryID("ac")
	if err != nil {
		return controlplane.ActivationCode{}, fmt.Errorf("generate activation code id: %w", err)
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, createdAt)
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadActivationCode(operationCtx, tx, storedResourceID)
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO activation_codes (id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id)
		VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, NULL)
	`, codeID, record.CodeHash, record.CodePrefix, controlplane.ActivationCodeStatusActive, createdAt, record.ExpiresAt.UTC()); err != nil {
		return controlplane.ActivationCode{}, postgresOperationError(operationCtx, fmt.Errorf("write normalized activation code: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ActivationCode{}, postgresCommitError(operationCtx, "commit normalized activation code", err)
	}
	plainCode := record.PlainCode
	return controlplane.ActivationCode{
		ID: codeID, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: record.ExpiresAt.UTC().Format(time.RFC3339), MaxDevices: record.MaxDevices,
		CodePrefix: record.CodePrefix, PlainCode: &plainCode,
	}, nil
}

func (s *PostgresRepository) RevokeActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint, codeID string) (controlplane.ActivationCode, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ActivationCode{}, errors.New("normalized activation repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ActivationCode{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	codeID = strings.TrimSpace(codeID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || codeID == "" {
		return controlplane.ActivationCode{}, errors.New("normalized activation revoke arguments are invalid")
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ActivationCode{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	code, err := s.loadActivationCodeForUpdate(operationCtx, tx, codeID)
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, s.Now())
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadActivationCode(operationCtx, tx, storedResourceID)
	}
	if code.Status != controlplane.ActivationCodeStatusActive && code.Status != controlplane.ActivationCodeStatusRevoked {
		return controlplane.ActivationCode{}, controlplane.ErrActivationCodeStateConflict
	}
	if code.Status == controlplane.ActivationCodeStatusActive {
		if _, err := tx.ExecContext(operationCtx, `UPDATE activation_codes SET status = $2 WHERE id = $1`, codeID, controlplane.ActivationCodeStatusRevoked); err != nil {
			return controlplane.ActivationCode{}, postgresOperationError(operationCtx, fmt.Errorf("revoke normalized activation code: %w", err))
		}
		code.Status = controlplane.ActivationCodeStatusRevoked
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ActivationCode{}, postgresCommitError(operationCtx, "commit normalized activation revoke", err)
	}
	code.PlainCode = nil
	return code, nil
}

func (s *PostgresRepository) loadActivationCode(ctx context.Context, tx *sql.Tx, codeID string) (controlplane.ActivationCode, error) {
	var code controlplane.ActivationCode
	var expiresAt, usedAt sql.NullTime
	var usedByUserID, usedByDeviceID sql.NullString
	if err := tx.QueryRowContext(ctx, `
		SELECT id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id
		FROM activation_codes
		WHERE id = $1
	`, codeID).Scan(&code.ID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.ActivationCode{}, postgresOperationError(ctx, fmt.Errorf("load idempotent normalized activation code: %w", err))
	}
	code.MaxDevices = 1
	if expiresAt.Valid {
		code.ExpiresAt = expiresAt.Time.UTC().Format(time.RFC3339)
	}
	if usedAt.Valid {
		code.UsedAt = usedAt.Time.UTC().Format(time.RFC3339)
	}
	code.UsedByUserID = usedByUserID.String
	code.UsedByDeviceID = usedByDeviceID.String
	return code, nil
}

func (s *PostgresRepository) loadActivationCodeForUpdate(ctx context.Context, tx *sql.Tx, codeID string) (controlplane.ActivationCode, error) {
	var code controlplane.ActivationCode
	var expiresAt, usedAt sql.NullTime
	var usedByUserID, usedByDeviceID sql.NullString
	if err := tx.QueryRowContext(ctx, `
		SELECT id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id
		FROM activation_codes
		WHERE id = $1
		FOR UPDATE
	`, codeID).Scan(&code.ID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.ActivationCode{}, postgresOperationError(ctx, fmt.Errorf("lock normalized activation code: %w", err))
	}
	code.MaxDevices = 1
	if expiresAt.Valid {
		code.ExpiresAt = expiresAt.Time.UTC().Format(time.RFC3339)
	}
	if usedAt.Valid {
		code.UsedAt = usedAt.Time.UTC().Format(time.RFC3339)
	}
	code.UsedByUserID = usedByUserID.String
	code.UsedByDeviceID = usedByDeviceID.String
	return code, nil
}
