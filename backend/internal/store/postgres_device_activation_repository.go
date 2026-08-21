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

var _ TransactionalDeviceActivator = (*PostgresRepository)(nil)

// ActivateDeviceWithSessionBinding keeps the capacity-limited activation,
// device registration and authenticated-session binding in one normalized SQL
// transaction. The legacy snapshot path remains behind the existing runner.
func (s *PostgresRepository) ActivateDeviceWithSessionBinding(ctx context.Context, record DeviceActivationRecord) (controlplane.DeviceSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.DeviceSummary{}, errors.New("normalized device activation requires normalized read source")
	}
	if ctx == nil {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccessTokenHash = strings.TrimSpace(record.AccessTokenHash)
	record.UserID = strings.TrimSpace(record.UserID)
	record.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Product)))
	record.ActivationCodeHash = strings.TrimSpace(record.ActivationCodeHash)
	record.Device.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Device.Product)))
	record.Device.DeviceID = strings.TrimSpace(record.Device.DeviceID)
	record.Device.DeviceName = strings.TrimSpace(record.Device.DeviceName)
	record.Device.Platform = strings.TrimSpace(record.Device.Platform)
	record.Device.AppVersion = strings.TrimSpace(record.Device.AppVersion)
	if !record.Product.Valid() || !record.Device.Product.Valid() {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	}
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccessTokenHash == "" || record.UserID == "" || record.ActivationCodeHash == "" || record.Device.DeviceID == "" || record.Device.DeviceName == "" || record.Device.Platform == "" || record.Device.AppVersion == "" {
		return controlplane.DeviceSummary{}, errors.New("normalized device activation arguments are incomplete")
	}
	if record.Product != record.Device.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}
	if record.Audit.Product == "" {
		record.Audit.Product = record.Product
	} else if !record.Audit.Product.Valid() {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	} else if record.Audit.Product != record.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}
	if err := ctx.Err(); err != nil {
		return controlplane.DeviceSummary{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.DeviceSummary{}, err
	}

	currentDeviceID, sessionProduct, err := s.lockAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.UserID)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if sessionProduct != record.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}
	if currentDeviceID != "" && currentDeviceID != record.Device.DeviceID {
		return controlplane.DeviceSummary{}, ErrSessionDeviceBindingConflict
	}

	existingDevice, exists, err := s.loadDeviceForUpdateWithProduct(operationCtx, tx, record.Device.DeviceID, record.Product)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if exists && existingDevice.Product != record.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}

	var codeID, codeStatus string
	var codeProduct sql.NullString
	var expiresAt sql.NullTime
	var maxDevices, boundDevices int
	codeQuery := `
		SELECT id, product, status, expires_at, max_devices, bound_devices
		FROM activation_codes
		WHERE code_hash = $1 AND `
	productCondition, productArgs := normalizedProductFilter("product", record.Product, 2)
	codeQuery += productCondition + "\n\t\tFOR UPDATE\n\t"
	codeArgs := append([]any{record.ActivationCodeHash}, productArgs...)
	if err := tx.QueryRowContext(operationCtx, codeQuery, codeArgs...).Scan(&codeID, &codeProduct, &codeStatus, &expiresAt, &maxDevices, &boundDevices); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("lock normalized activation code by hash: %w", err))
	}
	storedCodeProduct, err := normalizedStoredProduct(codeProduct)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if storedCodeProduct != record.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}

	createdAt := s.Now()
	var storedFingerprint, storedResourceID string
	var inserted bool
	if record.Product != controlplane.ProductAutoLive {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Device.DeviceID, createdAt, record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Device.DeviceID, createdAt)
	}
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
		}
		device, err := s.loadDeviceSummaryWithProduct(operationCtx, tx, storedResourceID, record.Product)
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		if currentDeviceID == "" {
			if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.Device.DeviceID, record.Product); err != nil {
				return controlplane.DeviceSummary{}, err
			}
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
				return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue activation audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.DeviceSummary{}, postgresCommitError(operationCtx, "commit idempotent device activation", err)
		}
		return device, nil
	}

	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.DeviceSummary{}, controlplane.ErrUserDisabled
	}

	if exists {
		if !(existingDevice.Status == controlplane.DeviceStatusPendingActivation && existingDevice.UserID == "") {
			if existingDevice.UserID != record.UserID || existingDevice.Status == controlplane.DeviceStatusDisabled {
				return controlplane.DeviceSummary{}, controlplane.ErrForbidden
			}
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingConflict
		}
	}

	now := s.Now()
	if !expiresAt.Valid || !now.Before(expiresAt.Time) {
		return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeExpired
	}
	switch codeStatus {
	case controlplane.ActivationCodeStatusRevoked:
		return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeRevoked
	case controlplane.ActivationCodeStatusUsed:
		return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeAlreadyUsed
	}
	if maxDevices < 1 || boundDevices >= maxDevices {
		return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeAlreadyUsed
	}

	device := controlplane.DeviceSummary{
		ID:         record.Device.DeviceID,
		UserID:     record.UserID,
		Product:    record.Product,
		DeviceName: record.Device.DeviceName,
		Platform:   record.Device.Platform,
		AppVersion: record.Device.AppVersion,
		Status:     controlplane.DeviceStatusActive,
		LastSeenAt: now.Format(time.RFC3339),
	}
	activationExpiresAt := expiresAt.Time.UTC().Format(time.RFC3339)
	device.ActivationExpiresAt = &activationExpiresAt
	if exists {
		if _, err := tx.ExecContext(operationCtx, `
			UPDATE devices
			SET user_id = $2, product = $3, device_name = $4, platform = $5, client_version = $6, status = $7, last_heartbeat_at = $8
			WHERE id = $1 AND product = $3
		`, device.ID, device.UserID, device.Product, device.DeviceName, device.Platform, device.AppVersion, device.Status, now); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized activated device: %w", err))
		}
	} else if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, last_heartbeat_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
	`, device.ID, device.UserID, device.Product, "state-device/"+device.ID, device.DeviceName, device.Platform, device.AppVersion, device.Status, now); err != nil {
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("insert normalized activated device: %w", err))
	}
	newBoundDevices := boundDevices + 1
	newStatus := controlplane.ActivationCodeStatusActive
	if newBoundDevices >= maxDevices {
		newStatus = controlplane.ActivationCodeStatusUsed
	}
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE activation_codes
		SET status = $2, bound_devices = $3, used_at = COALESCE(used_at, $4), used_by_user_id = COALESCE(used_by_user_id, $5), used_by_device_id = COALESCE(used_by_device_id, $6)
		WHERE id = $1 AND product = $7
	`, codeID, newStatus, newBoundDevices, now, record.UserID, device.ID, record.Product); err != nil {
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("redeem normalized activation code: %w", err))
	}
	if currentDeviceID == "" {
		if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, device.ID, record.Product); err != nil {
			return controlplane.DeviceSummary{}, err
		}
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, now); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue activation audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.DeviceSummary{}, postgresCommitError(operationCtx, "commit normalized device activation", err)
	}
	return device, nil
}

func (s *PostgresRepository) lockAuthenticatedSession(ctx context.Context, tx *sql.Tx, accessTokenHash, userID string) (string, controlplane.ProductCode, error) {
	var sessionUserID string
	var sessionProduct sql.NullString
	var currentDeviceID sql.NullString
	err := tx.QueryRowContext(ctx, `
		SELECT user_id, product, device_id
		FROM auth_sessions
		WHERE access_token_hash = $1 AND revoked_at IS NULL
		FOR UPDATE
	`, accessTokenHash).Scan(&sessionUserID, &sessionProduct, &currentDeviceID)
	if errors.Is(err, sql.ErrNoRows) {
		return "", "", controlplane.ErrUnauthenticated
	}
	if err != nil {
		return "", "", postgresOperationError(ctx, fmt.Errorf("lock auth session for normalized activation: %w", err))
	}
	if sessionUserID != userID {
		return "", "", controlplane.ErrForbidden
	}
	product, err := normalizedStoredProduct(sessionProduct)
	if err != nil {
		return "", "", err
	}
	if currentDeviceID.Valid {
		return currentDeviceID.String, product, nil
	}
	return "", product, nil
}

func bindAuthenticatedSession(ctx context.Context, tx *sql.Tx, accessTokenHash, deviceID string, product controlplane.ProductCode) error {
	result, err := tx.ExecContext(ctx, `
		UPDATE auth_sessions
		SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP
		WHERE access_token_hash = $1 AND product = $3 AND revoked_at IS NULL AND device_id IS NULL
	`, accessTokenHash, deviceID, product)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("bind normalized auth session: %w", err))
	}
	rows, err := result.RowsAffected()
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("count normalized auth session binding: %w", err))
	}
	if rows != 1 {
		return ErrSessionDeviceBindingConflict
	}
	return nil
}

func (s *PostgresRepository) loadDeviceForUpdate(ctx context.Context, tx *sql.Tx, deviceID string) (controlplane.DeviceSummary, bool, error) {
	var device controlplane.DeviceSummary
	var userID sql.NullString
	var product sql.NullString
	var lastSeenAt sql.NullTime
	err := tx.QueryRowContext(ctx, `
		SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at
		FROM devices
		WHERE id = $1
		FOR UPDATE
	`, deviceID).Scan(&device.ID, &userID, &product, &device.DeviceName, &device.Platform, &device.AppVersion, &device.Status, &lastSeenAt)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.DeviceSummary{}, false, nil
	}
	if err != nil {
		return controlplane.DeviceSummary{}, false, postgresOperationError(ctx, fmt.Errorf("lock normalized device: %w", err))
	}
	device.UserID = userID.String
	device.Product, err = normalizedStoredProduct(product)
	if err != nil {
		return controlplane.DeviceSummary{}, false, err
	}
	if lastSeenAt.Valid {
		device.LastSeenAt = lastSeenAt.Time.UTC().Format(time.RFC3339)
	}
	return device, true, nil
}

func (s *PostgresRepository) loadDeviceForUpdateWithProduct(ctx context.Context, tx *sql.Tx, deviceID string, product controlplane.ProductCode) (controlplane.DeviceSummary, bool, error) {
	var device controlplane.DeviceSummary
	var userID sql.NullString
	var storedProduct sql.NullString
	var lastSeenAt sql.NullTime
	condition, productArgs := normalizedProductFilter("product", product, 2)
	query := `
		SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at
		FROM devices
		WHERE id = $1 AND ` + condition + `
		FOR UPDATE
	`
	args := append([]any{deviceID}, productArgs...)
	err := tx.QueryRowContext(ctx, query, args...).Scan(&device.ID, &userID, &storedProduct, &device.DeviceName, &device.Platform, &device.AppVersion, &device.Status, &lastSeenAt)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.DeviceSummary{}, false, nil
	}
	if err != nil {
		return controlplane.DeviceSummary{}, false, postgresOperationError(ctx, fmt.Errorf("lock normalized product device: %w", err))
	}
	device.UserID = userID.String
	device.Product, err = normalizedStoredProduct(storedProduct)
	if err != nil {
		return controlplane.DeviceSummary{}, false, err
	}
	if lastSeenAt.Valid {
		device.LastSeenAt = lastSeenAt.Time.UTC().Format(time.RFC3339)
	}
	return device, true, nil
}

func normalizedStoredProduct(raw sql.NullString) (controlplane.ProductCode, error) {
	if !raw.Valid {
		return "", controlplane.ErrForbidden
	}
	product, err := controlplane.ParseProductCode(raw.String)
	if err != nil {
		return "", controlplane.ErrForbidden
	}
	return product, nil
}

func normalizedAuditProduct(raw sql.NullString) (controlplane.ProductCode, error) {
	if !raw.Valid || strings.TrimSpace(raw.String) == "" {
		return controlplane.ProductAutoLive, nil
	}
	product, err := controlplane.ParseProductCode(raw.String)
	if err != nil {
		return "", controlplane.ErrForbidden
	}
	return product, nil
}

func (s *PostgresRepository) loadDeviceSummary(ctx context.Context, tx *sql.Tx, deviceID string) (controlplane.DeviceSummary, error) {
	device, exists, err := s.loadDeviceForUpdate(ctx, tx, deviceID)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !exists {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return device, nil
}

func (s *PostgresRepository) loadDeviceSummaryWithProduct(ctx context.Context, tx *sql.Tx, deviceID string, product controlplane.ProductCode) (controlplane.DeviceSummary, error) {
	device, exists, err := s.loadDeviceForUpdateWithProduct(ctx, tx, deviceID, product)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !exists {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return device, nil
}
