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

var _ AdminCredentialRepository = (*PostgresRepository)(nil)

var ErrNormalizedAdminCredentialRepositoryRequired = errors.New("normalized admin credential repository is required")

func (s *PostgresRepository) EnsureConfiguredAdmin(ctx context.Context, username string, passwordHash []byte) error {
	if s.modelReadSource != ModelReadSourceNormalized {
		return ErrNormalizedAdminCredentialRepositoryRequired
	}
	if ctx == nil || strings.TrimSpace(username) == "" || len(passwordHash) == 0 {
		return controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	username = strings.TrimSpace(username)
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return err
	}
	var user controlplane.UserSummary
	var storedHash []byte
	var createdAt time.Time
	err = tx.QueryRowContext(operationCtx, `
		SELECT id, username, password_hash, role, status, created_at
		FROM users
		WHERE id = 'usr_local_admin'
		FOR UPDATE
	`).Scan(&user.ID, &user.Username, &storedHash, &user.Role, &user.Status, &createdAt)
	if errors.Is(err, sql.ErrNoRows) {
		var duplicateID string
		duplicateErr := tx.QueryRowContext(operationCtx, `SELECT id FROM users WHERE username = $1 LIMIT 1`, username).Scan(&duplicateID)
		if duplicateErr == nil {
			return controlplane.ErrUsernameAlreadyExists
		}
		if !errors.Is(duplicateErr, sql.ErrNoRows) {
			return postgresOperationError(operationCtx, fmt.Errorf("check normalized admin username: %w", duplicateErr))
		}
		now := s.Now()
		if _, err := tx.ExecContext(operationCtx, `
			INSERT INTO users (id, username, password_hash, role, status, created_at)
			VALUES ('usr_local_admin', $1, $2, $3, $4, $5)
		`, username, passwordHash, controlplane.RoleAdmin, controlplane.UserStatusActive, now); err != nil {
			return postgresOperationError(operationCtx, fmt.Errorf("insert normalized local admin: %w", err))
		}
	} else if err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("load normalized local admin: %w", err))
	} else {
		if user.Username != username || user.Role != controlplane.RoleAdmin {
			return errors.New("persisted local administrator does not match configured identity")
		}
		if len(storedHash) > 0 {
			if err := tx.Commit(); err != nil {
				return postgresCommitError(operationCtx, "commit existing normalized local admin", err)
			}
			return nil
		}
		if _, err := tx.ExecContext(operationCtx, `UPDATE users SET password_hash = $2 WHERE id = 'usr_local_admin'`, passwordHash); err != nil {
			return postgresOperationError(operationCtx, fmt.Errorf("initialize normalized local admin password: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit normalized local admin", err)
	}
	return nil
}

func (s *PostgresRepository) CheckAdminReady(ctx context.Context) error {
	if s.modelReadSource != ModelReadSourceNormalized {
		return ErrNormalizedAdminCredentialRepositoryRequired
	}
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	var ready bool
	if err := s.db.QueryRowContext(operationCtx, `
		SELECT EXISTS(
			SELECT 1 FROM users
			WHERE id = 'usr_local_admin'
			  AND role = $1
			  AND status = $2
			  AND length(password_hash) > 0
		)
	`, controlplane.RoleAdmin, controlplane.UserStatusActive).Scan(&ready); err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("check normalized local admin readiness: %w", err))
	}
	if !ready {
		return errors.New("local admin credential is not initialized")
	}
	return nil
}

func (s *PostgresRepository) ChangeLocalAdminPassword(ctx context.Context, scope, idempotencyKey, fingerprint string, passwordHash []byte) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, ErrNormalizedAdminCredentialRepositoryRequired
	}
	if ctx == nil || strings.TrimSpace(scope) == "" || strings.TrimSpace(idempotencyKey) == "" || strings.TrimSpace(fingerprint) == "" || len(passwordHash) == 0 {
		return controlplane.UserSummary{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserSummary{}, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.UserSummary{}, err
	}
	user, err := s.loadUserForUpdate(operationCtx, tx, "usr_local_admin")
	if errors.Is(err, controlplane.ErrUserNotFound) {
		return controlplane.UserSummary{}, controlplane.ErrLocalAdminRequired
	}
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if user.Role != controlplane.RoleAdmin {
		return controlplane.UserSummary{}, controlplane.ErrLocalAdminRequired
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.UserSummary{}, controlplane.ErrUserDisabled
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, user.ID, s.Now())
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserByID(operationCtx, tx, storedResourceID)
	}
	if _, err := tx.ExecContext(operationCtx, `UPDATE users SET password_hash = $2 WHERE id = $1`, user.ID, passwordHash); err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("rotate normalized local admin password: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized local admin password", err)
	}
	return user, nil
}
