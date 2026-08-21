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

// ActivateDeviceWithSessionBinding keeps the one-time activation, device
// registration and authenticated-session binding in one normalized SQL
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
	record.ActivationCodeHash = strings.TrimSpace(record.ActivationCodeHash)
	record.Device.DeviceID = strings.TrimSpace(record.Device.DeviceID)
	record.Device.DeviceName = strings.TrimSpace(record.Device.DeviceName)
	record.Device.Platform = strings.TrimSpace(record.Device.Platform)
	record.Device.AppVersion = strings.TrimSpace(record.Device.AppVersion)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccessTokenHash == "" || record.UserID == "" || record.ActivationCodeHash == "" || record.Device.DeviceID == "" || record.Device.DeviceName == "" || record.Device.Platform == "" || record.Device.AppVersion == "" {
		return controlplane.DeviceSummary{}, errors.New("normalized device activation arguments are incomplete")
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

	currentDeviceID, err := s.lockAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.UserID)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if currentDeviceID != "" && currentDeviceID != record.Device.DeviceID {
		return controlplane.DeviceSummary{}, ErrSessionDeviceBindingConflict
	}

	createdAt := s.Now()
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Device.DeviceID, createdAt)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
		}
		device, err := s.loadDeviceSummary(operationCtx, tx, storedResourceID)
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		if currentDeviceID == "" {
			if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.Device.DeviceID); err != nil {
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

	existingDevice, exists, err := s.loadDeviceForUpdate(operationCtx, tx, record.Device.DeviceID)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if exists {
		if !(existingDevice.Status == controlplane.DeviceStatusPendingActivation && existingDevice.UserID == "") {
			if existingDevice.UserID != record.UserID || existingDevice.Status == controlplane.DeviceStatusDisabled {
				return controlplane.DeviceSummary{}, controlplane.ErrForbidden
			}
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingConflict
		}
	}

	var codeID, codeStatus string
	var expiresAt sql.NullTime
	if err := tx.QueryRowContext(operationCtx, `
		SELECT id, status, expires_at
		FROM activation_codes
		WHERE code_hash = $1
		FOR UPDATE
	`, record.ActivationCodeHash).Scan(&codeID, &codeStatus, &expiresAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeNotFound
		}
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("lock normalized activation code by hash: %w", err))
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

	device := controlplane.DeviceSummary{
		ID:         record.Device.DeviceID,
		UserID:     record.UserID,
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
			SET user_id = $2, device_name = $3, platform = $4, client_version = $5, status = $6, last_heartbeat_at = $7
			WHERE id = $1
		`, device.ID, device.UserID, device.DeviceName, device.Platform, device.AppVersion, device.Status, now); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized activated device: %w", err))
		}
	} else if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO devices (id, user_id, device_key, device_name, platform, client_version, status, last_heartbeat_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
	`, device.ID, device.UserID, "state-device/"+device.ID, device.DeviceName, device.Platform, device.AppVersion, device.Status, now); err != nil {
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("insert normalized activated device: %w", err))
	}
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE activation_codes
		SET status = $2, used_at = $3, used_by_user_id = $4, used_by_device_id = $5
		WHERE id = $1
	`, codeID, controlplane.ActivationCodeStatusUsed, now, record.UserID, device.ID); err != nil {
		return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("redeem normalized activation code: %w", err))
	}
	if currentDeviceID == "" {
		if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, device.ID); err != nil {
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

func (s *PostgresRepository) lockAuthenticatedSession(ctx context.Context, tx *sql.Tx, accessTokenHash, userID string) (string, error) {
	var sessionUserID string
	var currentDeviceID sql.NullString
	err := tx.QueryRowContext(ctx, `
		SELECT user_id, device_id
		FROM auth_sessions
		WHERE access_token_hash = $1 AND revoked_at IS NULL
		FOR UPDATE
	`, accessTokenHash).Scan(&sessionUserID, &currentDeviceID)
	if errors.Is(err, sql.ErrNoRows) {
		return "", controlplane.ErrUnauthenticated
	}
	if err != nil {
		return "", postgresOperationError(ctx, fmt.Errorf("lock auth session for normalized activation: %w", err))
	}
	if sessionUserID != userID {
		return "", controlplane.ErrForbidden
	}
	if currentDeviceID.Valid {
		return currentDeviceID.String, nil
	}
	return "", nil
}

func bindAuthenticatedSession(ctx context.Context, tx *sql.Tx, accessTokenHash, deviceID string) error {
	result, err := tx.ExecContext(ctx, `
		UPDATE auth_sessions
		SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP
		WHERE access_token_hash = $1 AND revoked_at IS NULL AND device_id IS NULL
	`, accessTokenHash, deviceID)
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
	var lastSeenAt sql.NullTime
	err := tx.QueryRowContext(ctx, `
		SELECT id, user_id, device_name, platform, client_version, status, last_heartbeat_at
		FROM devices
		WHERE id = $1
		FOR UPDATE
	`, deviceID).Scan(&device.ID, &userID, &device.DeviceName, &device.Platform, &device.AppVersion, &device.Status, &lastSeenAt)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.DeviceSummary{}, false, nil
	}
	if err != nil {
		return controlplane.DeviceSummary{}, false, postgresOperationError(ctx, fmt.Errorf("lock normalized device: %w", err))
	}
	device.UserID = userID.String
	if lastSeenAt.Valid {
		device.LastSeenAt = lastSeenAt.Time.UTC().Format(time.RFC3339)
	}
	return device, true, nil
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
