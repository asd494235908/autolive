package store

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ TransactionalHeartbeatRecorder = (*PostgresRepository)(nil)

// RecordHeartbeatWithSessionBinding updates the normalized device row and
// binds the authenticated session atomically. The caller supplies the already
// canonicalized fingerprint; the heartbeat payload is never written to an
// idempotency response or an audit payload.
func (s *PostgresRepository) RecordHeartbeatWithSessionBinding(ctx context.Context, record DeviceHeartbeatRecord) (controlplane.HeartbeatResult, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.HeartbeatResult{}, errors.New("normalized heartbeat requires normalized read source")
	}
	if ctx == nil {
		return controlplane.HeartbeatResult{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccessTokenHash = strings.TrimSpace(record.AccessTokenHash)
	record.UserID = strings.TrimSpace(record.UserID)
	record.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Product)))
	record.Input.DeviceID = strings.TrimSpace(record.Input.DeviceID)
	record.Input.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Input.Product)))
	if !record.Product.Valid() || !record.Input.Product.Valid() {
		return controlplane.HeartbeatResult{}, controlplane.ErrInvalidRequest
	}
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.AccessTokenHash == "" || record.UserID == "" || record.Input.DeviceID == "" {
		return controlplane.HeartbeatResult{}, errors.New("normalized heartbeat arguments are incomplete")
	}
	if record.Product != record.Input.Product {
		return controlplane.HeartbeatResult{}, controlplane.ErrForbidden
	}
	if record.Audit.Product == "" {
		record.Audit.Product = record.Product
	} else if !record.Audit.Product.Valid() {
		return controlplane.HeartbeatResult{}, controlplane.ErrInvalidRequest
	} else if record.Audit.Product != record.Product {
		return controlplane.HeartbeatResult{}, controlplane.ErrForbidden
	}
	if err := ctx.Err(); err != nil {
		return controlplane.HeartbeatResult{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.HeartbeatResult{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.HeartbeatResult{}, err
	}

	currentDeviceID, sessionProduct, err := s.lockAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.UserID)
	if err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if sessionProduct != record.Product {
		return controlplane.HeartbeatResult{}, controlplane.ErrForbidden
	}
	if currentDeviceID != "" && currentDeviceID != record.Input.DeviceID {
		return controlplane.HeartbeatResult{}, ErrSessionDeviceBindingConflict
	}

	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.HeartbeatResult{}, controlplane.ErrUserDisabled
	}
	device, exists, err := s.loadDeviceForUpdate(operationCtx, tx, record.Input.DeviceID)
	if err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if !exists || device.UserID != record.UserID {
		return controlplane.HeartbeatResult{}, controlplane.ErrDeviceNotFound
	}
	if device.Product != record.Product {
		return controlplane.HeartbeatResult{}, controlplane.ErrForbidden
	}
	if device.Status != controlplane.DeviceStatusActive {
		return controlplane.HeartbeatResult{}, controlplane.ErrDeviceDisabled
	}

	var storedFingerprint, storedResourceID string
	var inserted bool
	if record.Product != controlplane.ProductAutoLive {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Input.DeviceID, s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Input.DeviceID, s.Now())
	}
	if err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyConflict
		}
		if storedResourceID != record.Input.DeviceID {
			return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyConflict
		}
		if currentDeviceID == "" {
			if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.Input.DeviceID, record.Product); err != nil {
				return controlplane.HeartbeatResult{}, err
			}
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, s.Now()); err != nil {
				return controlplane.HeartbeatResult{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue heartbeat audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.HeartbeatResult{}, postgresCommitError(operationCtx, "commit idempotent normalized heartbeat", err)
		}
		return controlplane.HeartbeatResult{AcceptedAt: device.LastSeenAt, DeviceStatus: device.Status}, nil
	}

	acceptedAt := s.Now()
	status := record.Input.Status
	result, err := tx.ExecContext(operationCtx, `
		UPDATE devices
		SET disk_free_bytes = $2,
		    memory_total_bytes = $3,
		    memory_available_bytes = $4,
		    cpu_logical_cores = $5,
		    runtime_os_name = $6,
		    runtime_os_version = $7,
		    kernel_version = $8,
		    current_media_name = $9,
		    playback_state = $10,
		    last_heartbeat_at = $11
		WHERE id = $1 AND product = $12
	`, record.Input.DeviceID, status.DiskFreeBytes, status.MemoryTotalBytes, status.MemoryAvailableBytes, status.CPULogicalCores, status.OSName, status.OSVersion, status.KernelVersion, status.CurrentMediaName, status.PlaybackState, acceptedAt, record.Product)
	if err != nil {
		return controlplane.HeartbeatResult{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized device heartbeat: %w", err))
	}
	rows, err := result.RowsAffected()
	if err != nil {
		return controlplane.HeartbeatResult{}, postgresOperationError(operationCtx, fmt.Errorf("count normalized device heartbeat update: %w", err))
	}
	if rows != 1 {
		return controlplane.HeartbeatResult{}, controlplane.ErrForbidden
	}
	if currentDeviceID == "" {
		if err := bindAuthenticatedSession(operationCtx, tx, record.AccessTokenHash, record.Input.DeviceID, record.Product); err != nil {
			return controlplane.HeartbeatResult{}, err
		}
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, record.Audit, acceptedAt); err != nil {
			return controlplane.HeartbeatResult{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue heartbeat audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.HeartbeatResult{}, postgresCommitError(operationCtx, "commit normalized device heartbeat", err)
	}
	return controlplane.HeartbeatResult{AcceptedAt: acceptedAt.Format(time.RFC3339), DeviceStatus: device.Status}, nil
}
