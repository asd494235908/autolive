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

var _ DeviceLifecycleRepository = (*PostgresRepository)(nil)

func (s *PostgresRepository) DisableDevice(ctx context.Context, record DeviceMutationRecord) (controlplane.DeviceSummary, error) {
	return s.mutateDeviceLifecycle(ctx, record, false)
}

func (s *PostgresRepository) UnbindDevice(ctx context.Context, record DeviceMutationRecord) (controlplane.DeviceSummary, error) {
	return s.mutateDeviceLifecycle(ctx, record, true)
}

func (s *PostgresRepository) mutateDeviceLifecycle(ctx context.Context, record DeviceMutationRecord, unbind bool) (controlplane.DeviceSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.DeviceSummary{}, errors.New("normalized device lifecycle requires normalized read source")
	}
	if ctx == nil {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.DeviceID = strings.TrimSpace(record.DeviceID)
	explicitProduct := record.Product != ""
	record.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Product)))
	if record.Product == "" {
		record.Product = controlplane.ProductAutoLive
	}
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.DeviceID == "" || !record.Product.Valid() {
		return controlplane.DeviceSummary{}, errors.New("normalized device lifecycle arguments are incomplete")
	}
	var err error
	record.Audit, err = normalizeOptionalAuditInputForProduct(record.Audit, record.Product)
	if err != nil {
		return controlplane.DeviceSummary{}, err
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

	var device controlplane.DeviceSummary
	var exists bool
	if explicitProduct {
		device, exists, err = s.loadDeviceForUpdateWithProduct(operationCtx, tx, record.DeviceID, record.Product)
	} else {
		device, exists, err = s.loadDeviceForUpdate(operationCtx, tx, record.DeviceID)
	}
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !exists {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	if !explicitProduct && device.Product != record.Product {
		return controlplane.DeviceSummary{}, controlplane.ErrForbidden
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if explicitProduct {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.DeviceID, s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.DeviceID, s.Now())
	}
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
		}
		if unbind {
			if explicitProduct {
				device, err = s.loadDeviceSummaryWithProduct(operationCtx, tx, storedResourceID, record.Product)
			} else {
				device, err = s.loadDeviceSummary(operationCtx, tx, storedResourceID)
			}
			if err != nil {
				return controlplane.DeviceSummary{}, err
			}
		}
		if err := releaseDeviceLeasesForRecord(operationCtx, tx, record.DeviceID, s.Now(), record.Product, explicitProduct); err != nil {
			return controlplane.DeviceSummary{}, err
		}
		if err := revokeDeviceSessionsForRecord(operationCtx, tx, record.DeviceID, record.Product, explicitProduct); err != nil {
			return controlplane.DeviceSummary{}, err
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
				return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue device lifecycle audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.DeviceSummary{}, postgresCommitError(operationCtx, "commit idempotent normalized device lifecycle", err)
		}
		return device, nil
	}

	now := s.Now()
	if err := releaseDeviceLeasesForRecord(operationCtx, tx, record.DeviceID, now, record.Product, explicitProduct); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if err := revokeDeviceSessionsForRecord(operationCtx, tx, record.DeviceID, record.Product, explicitProduct); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if unbind {
		var activationCodeID string
		bindingErr := tx.QueryRowContext(operationCtx, `
			SELECT activation_code_id
			FROM activation_device_bindings
			WHERE device_id = $1 AND product = $2
			FOR UPDATE
		`, record.DeviceID, record.Product).Scan(&activationCodeID)
		if bindingErr != nil && !errors.Is(bindingErr, sql.ErrNoRows) {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("lock activation device binding for unbind: %w", bindingErr))
		}
		if activationCodeID != "" {
			if _, err := tx.ExecContext(operationCtx, `DELETE FROM activation_device_bindings WHERE device_id = $1`, record.DeviceID); err != nil {
				return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("delete activation device binding: %w", err))
			}
			if _, err := tx.ExecContext(operationCtx, `
				UPDATE activation_codes
				SET bound_devices = GREATEST(bound_devices - 1, 0),
				    status = CASE WHEN status = $2 AND expires_at > $3 THEN $4 ELSE status END
				WHERE id = $1 AND product = $5
			`, activationCodeID, controlplane.ActivationCodeStatusUsed, now, controlplane.ActivationCodeStatusActive, record.Product); err != nil {
				return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("release activation device capacity: %w", err))
			}
		}
		updateQuery := `UPDATE devices SET user_id = NULL, status = $2 WHERE id = $1`
		updateArgs := []any{record.DeviceID, controlplane.DeviceStatusPendingActivation}
		if explicitProduct {
			updateQuery += " AND product = $3"
			updateArgs = append(updateArgs, record.Product)
		}
		if _, err := tx.ExecContext(operationCtx, updateQuery, updateArgs...); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("unbind normalized device: %w", err))
		}
		device.UserID = ""
		device.Status = controlplane.DeviceStatusPendingActivation
	} else {
		updateQuery := `UPDATE devices SET status = $2 WHERE id = $1`
		updateArgs := []any{record.DeviceID, controlplane.DeviceStatusDisabled}
		if explicitProduct {
			updateQuery += " AND product = $3"
			updateArgs = append(updateArgs, record.Product)
		}
		if _, err := tx.ExecContext(operationCtx, updateQuery, updateArgs...); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("disable normalized device: %w", err))
		}
		device.Status = controlplane.DeviceStatusDisabled
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, now); err != nil {
			return controlplane.DeviceSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue device lifecycle audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.DeviceSummary{}, postgresCommitError(operationCtx, "commit normalized device lifecycle", err)
	}
	return device, nil
}

func releaseDeviceLeases(ctx context.Context, tx *sql.Tx, deviceID string, now time.Time) error {
	query := `
		UPDATE model_leases
		SET status = $2, released_at = $3
		WHERE device_id = $1 AND status = $4
	`
	args := []any{deviceID, controlplane.ModelLeaseStatusReleased, now, controlplane.ModelLeaseStatusActive}
	if _, err := tx.ExecContext(ctx, query, args...); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("release normalized device leases: %w", err))
	}
	return nil
}

func revokeDeviceSessions(ctx context.Context, tx *sql.Tx, deviceID string) error {
	if _, err := tx.ExecContext(ctx, `
		UPDATE auth_sessions
		SET revoked_at = CURRENT_TIMESTAMP
		WHERE device_id = $1 AND revoked_at IS NULL
	`, deviceID); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("revoke normalized device sessions: %w", err))
	}
	return nil
}

func releaseDeviceLeasesForRecord(ctx context.Context, tx *sql.Tx, deviceID string, now time.Time, product controlplane.ProductCode, strict bool) error {
	if !strict {
		return releaseDeviceLeases(ctx, tx, deviceID, now)
	}
	condition, productArgs := normalizedProductFilter("product", product, 5)
	query := `
		UPDATE model_leases
		SET status = $2, released_at = $3
		WHERE device_id = $1 AND status = $4 AND ` + condition
	args := append([]any{deviceID, controlplane.ModelLeaseStatusReleased, now, controlplane.ModelLeaseStatusActive}, productArgs...)
	if _, err := tx.ExecContext(ctx, query, args...); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("release normalized product device leases: %w", err))
	}
	return nil
}

func revokeDeviceSessionsForRecord(ctx context.Context, tx *sql.Tx, deviceID string, product controlplane.ProductCode, strict bool) error {
	if !strict {
		return revokeDeviceSessions(ctx, tx, deviceID)
	}
	condition, productArgs := normalizedProductFilter("product", product, 2)
	query := `
		UPDATE auth_sessions
		SET revoked_at = CURRENT_TIMESTAMP
		WHERE device_id = $1 AND revoked_at IS NULL AND ` + condition
	args := append([]any{deviceID}, productArgs...)
	if _, err := tx.ExecContext(ctx, query, args...); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("revoke normalized product device sessions: %w", err))
	}
	return nil
}
