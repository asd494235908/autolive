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
var _ ProductActivationRepository = (*PostgresRepository)(nil)

// CreateActivationCode persists only the digest of the one-time code. The
// plaintext is returned for the first response and is never stored in the
// normalized table or the idempotency record.
func (s *PostgresRepository) CreateActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint string, record ActivationCodeCreateRecord) (controlplane.ActivationCode, error) {
	return s.createActivationCode(ctx, scope, idempotencyKey, fingerprint, record, controlplane.ProductAutoLive, false)
}

func (s *PostgresRepository) CreateActivationCodeForProduct(ctx context.Context, scope, idempotencyKey, fingerprint string, record ActivationCodeCreateRecord, product controlplane.ProductCode) (controlplane.ActivationCode, error) {
	return s.createActivationCode(ctx, scope, idempotencyKey, fingerprint, record, product, true)
}

func (s *PostgresRepository) createActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint string, record ActivationCodeCreateRecord, product controlplane.ProductCode, strictProduct bool) (controlplane.ActivationCode, error) {
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
	record.UserID = strings.TrimSpace(record.UserID)
	record.CodeHash = strings.TrimSpace(record.CodeHash)
	record.CodePrefix = strings.TrimSpace(record.CodePrefix)
	product = controlplane.ProductCode(strings.TrimSpace(string(product)))
	if strictProduct && !product.Valid() {
		return controlplane.ActivationCode{}, controlplane.ErrInvalidRequest
	}
	if record.Product != "" && controlplane.ProductCode(strings.TrimSpace(string(record.Product))) != product {
		return controlplane.ActivationCode{}, controlplane.ErrForbidden
	}
	record.Product = product
	if scope == "" || idempotencyKey == "" || fingerprint == "" || record.UserID == "" || record.PlainCode == "" || record.CodeHash == "" || record.CodePrefix == "" || record.ExpiresAt.IsZero() || record.MaxDevices < 1 || record.MaxDevices > controlplane.MaxActivationCodeDevices {
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
	var storedFingerprint, storedResourceID string
	var inserted bool
	if strictProduct {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, createdAt, product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, createdAt)
	}
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
		}
		if strictProduct {
			return s.loadActivationCodeForProduct(operationCtx, tx, storedResourceID, product, false)
		}
		return s.loadActivationCode(operationCtx, tx, storedResourceID)
	}
	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.ActivationCode{}, controlplane.ErrUserDisabled
	}
	var membershipStatus string
	if err := tx.QueryRowContext(operationCtx, `
		SELECT status
		FROM user_products
		WHERE user_id = $1 AND product = $2
		FOR KEY SHARE
	`, record.UserID, product).Scan(&membershipStatus); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrForbidden
		}
		return controlplane.ActivationCode{}, postgresOperationError(operationCtx, fmt.Errorf("read activation user product membership: %w", err))
	}
	if membershipStatus != "active" {
		return controlplane.ActivationCode{}, controlplane.ErrForbidden
	}
	insertQuery := `
		INSERT INTO activation_codes (id, product, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, NULL, NULL, $9, $10)`
	insertArgs := []any{codeID, product, record.UserID, record.CodeHash, record.CodePrefix, controlplane.ActivationCodeStatusActive, createdAt, record.ExpiresAt.UTC(), record.MaxDevices, 0}
	if !strictProduct {
		insertQuery = `
			INSERT INTO activation_codes (id, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices)
			VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, NULL, NULL, $8, $9)`
		insertArgs = []any{codeID, record.UserID, record.CodeHash, record.CodePrefix, controlplane.ActivationCodeStatusActive, createdAt, record.ExpiresAt.UTC(), record.MaxDevices, 0}
	}
	if _, err := tx.ExecContext(operationCtx, insertQuery, insertArgs...); err != nil {
		return controlplane.ActivationCode{}, postgresOperationError(operationCtx, fmt.Errorf("write normalized activation code: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ActivationCode{}, postgresCommitError(operationCtx, "commit normalized activation code", err)
	}
	plainCode := record.PlainCode
	return controlplane.ActivationCode{
		ID: codeID, Product: product, UserID: record.UserID, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: record.ExpiresAt.UTC().Format(time.RFC3339), MaxDevices: record.MaxDevices, BoundDevices: 0,
		CodePrefix: record.CodePrefix, PlainCode: &plainCode,
	}, nil
}

func (s *PostgresRepository) RevokeActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint, codeID string) (controlplane.ActivationCode, error) {
	return s.revokeActivationCode(ctx, scope, idempotencyKey, fingerprint, codeID, controlplane.ProductAutoLive, false)
}

func (s *PostgresRepository) RevokeActivationCodeForProduct(ctx context.Context, scope, idempotencyKey, fingerprint, codeID string, product controlplane.ProductCode) (controlplane.ActivationCode, error) {
	return s.revokeActivationCode(ctx, scope, idempotencyKey, fingerprint, codeID, product, true)
}

func (s *PostgresRepository) revokeActivationCode(ctx context.Context, scope, idempotencyKey, fingerprint, codeID string, product controlplane.ProductCode, strictProduct bool) (controlplane.ActivationCode, error) {
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
	product = controlplane.ProductCode(strings.TrimSpace(string(product)))
	if strictProduct && !product.Valid() {
		return controlplane.ActivationCode{}, controlplane.ErrInvalidRequest
	}
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
	var code controlplane.ActivationCode
	if strictProduct {
		code, err = s.loadActivationCodeForProduct(operationCtx, tx, codeID, product, true)
	} else {
		code, err = s.loadActivationCodeForUpdate(operationCtx, tx, codeID)
	}
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if strictProduct {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, s.Now(), product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, codeID, s.Now())
	}
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
		}
		if strictProduct {
			return s.loadActivationCodeForProduct(operationCtx, tx, storedResourceID, product, false)
		}
		return s.loadActivationCode(operationCtx, tx, storedResourceID)
	}
	if code.Status != controlplane.ActivationCodeStatusActive && code.Status != controlplane.ActivationCodeStatusUsed && code.Status != controlplane.ActivationCodeStatusRevoked {
		return controlplane.ActivationCode{}, controlplane.ErrActivationCodeStateConflict
	}
	if code.Status == controlplane.ActivationCodeStatusActive || code.Status == controlplane.ActivationCodeStatusUsed {
		updateQuery := `UPDATE activation_codes SET status = $2 WHERE id = $1`
		updateArgs := []any{codeID, controlplane.ActivationCodeStatusRevoked}
		if strictProduct {
			updateQuery += " AND product = $3"
			updateArgs = append(updateArgs, product)
		}
		if _, err := tx.ExecContext(operationCtx, updateQuery, updateArgs...); err != nil {
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
	var boundUserID, usedByUserID, usedByDeviceID sql.NullString
	var maxDevices, boundDevices int
	if err := tx.QueryRowContext(ctx, `
		SELECT id, bound_user_id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices
		FROM activation_codes
		WHERE id = $1
	`, codeID).Scan(&code.ID, &boundUserID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &maxDevices, &boundDevices); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.ActivationCode{}, postgresOperationError(ctx, fmt.Errorf("load idempotent normalized activation code: %w", err))
	}
	code.MaxDevices = maxDevices
	code.BoundDevices = boundDevices
	code.UserID = boundUserID.String
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
	var boundUserID, usedByUserID, usedByDeviceID sql.NullString
	var maxDevices, boundDevices int
	if err := tx.QueryRowContext(ctx, `
		SELECT id, bound_user_id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices
		FROM activation_codes
		WHERE id = $1
		FOR UPDATE
	`, codeID).Scan(&code.ID, &boundUserID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &maxDevices, &boundDevices); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.ActivationCode{}, postgresOperationError(ctx, fmt.Errorf("lock normalized activation code: %w", err))
	}
	code.MaxDevices = maxDevices
	code.BoundDevices = boundDevices
	code.UserID = boundUserID.String
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

func (s *PostgresRepository) loadActivationCodeForProduct(ctx context.Context, tx *sql.Tx, codeID string, product controlplane.ProductCode, forUpdate bool) (controlplane.ActivationCode, error) {
	var code controlplane.ActivationCode
	var storedProduct string
	var expiresAt, usedAt sql.NullTime
	var boundUserID, usedByUserID, usedByDeviceID sql.NullString
	query := `
		SELECT id, product, bound_user_id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices
		FROM activation_codes
		WHERE id = $1 AND product = $2`
	if forUpdate {
		query += " FOR UPDATE"
	}
	if err := tx.QueryRowContext(ctx, query, codeID, product).Scan(&code.ID, &storedProduct, &boundUserID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &code.MaxDevices, &code.BoundDevices); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.ActivationCode{}, postgresOperationError(ctx, fmt.Errorf("load product-scoped normalized activation code: %w", err))
	}
	parsedProduct, err := controlplane.ParseProductCode(storedProduct)
	if err != nil || parsedProduct != product {
		return controlplane.ActivationCode{}, controlplane.ErrForbidden
	}
	code.Product = parsedProduct
	code.UserID = boundUserID.String
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
