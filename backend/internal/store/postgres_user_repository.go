package store

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/lib/pq"
)

var _ UserRepository = (*PostgresRepository)(nil)
var _ UserCredentialReader = (*PostgresRepository)(nil)
var _ UserReader = (*PostgresRepository)(nil)

var ErrNormalizedUserCredentialReaderRequired = errors.New("normalized user credential reader is required")
var ErrNormalizedUserReaderRequired = errors.New("normalized user reader is required")
var ErrNormalizedUserRepositoryRequired = errors.New("normalized user repository is required")

func (s *PostgresRepository) GetUserByID(ctx context.Context, userID string) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, ErrNormalizedUserReaderRequired
	}
	if ctx == nil || strings.TrimSpace(userID) == "" {
		return controlplane.UserSummary{}, controlplane.ErrUserNotFound
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	var user controlplane.UserSummary
	var createdAt time.Time
	err := s.db.QueryRowContext(operationCtx, `
		SELECT id, username, role, status, created_at
		FROM users
		WHERE id = $1
		LIMIT 1
	`, strings.TrimSpace(userID)).Scan(&user.ID, &user.Username, &user.Role, &user.Status, &createdAt)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.UserSummary{}, controlplane.ErrUserNotFound
	}
	if err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("read normalized user: %w", err))
	}
	user.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return user, nil
}

// GetUserCredential reads only the normalized user row needed by the login
// boundary. Password comparison remains in the service layer; this method
// does not expose the hash beyond that in-process call and never logs it.
func (s *PostgresRepository) GetUserCredential(ctx context.Context, username string) (controlplane.UserSummary, []byte, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, nil, ErrNormalizedUserCredentialReaderRequired
	}
	if ctx == nil {
		return controlplane.UserSummary{}, nil, controlplane.ErrInvalidRequest
	}
	username = strings.TrimSpace(username)
	if username == "" {
		return controlplane.UserSummary{}, nil, controlplane.ErrUnauthenticated
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	var user controlplane.UserSummary
	var passwordHash []byte
	var createdAt time.Time
	err := s.db.QueryRowContext(operationCtx, `
		SELECT id, username, password_hash, role, status, created_at
		FROM users
		WHERE username = $1
		LIMIT 1
	`, username).Scan(&user.ID, &user.Username, &passwordHash, &user.Role, &user.Status, &createdAt)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.UserSummary{}, nil, controlplane.ErrUnauthenticated
	}
	if err != nil {
		return controlplane.UserSummary{}, nil, postgresOperationError(operationCtx, fmt.Errorf("read normalized user credential: %w", err))
	}
	user.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return user, append([]byte(nil), passwordHash...), nil
}

// CreateUser persists the normalized users and idempotency records in one
// transaction. It is deliberately separate from StateOperation so a
// normalized deployment does not read or rewrite the legacy snapshot.
func (s *PostgresRepository) CreateUser(ctx context.Context, scope, idempotencyKey, fingerprint string, record UserCreateRecord) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, errors.New("normalized user repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserSummary{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	record.Username = strings.TrimSpace(record.Username)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || record.Username == "" || len(record.PasswordHash) == 0 {
		return controlplane.UserSummary{}, errors.New("normalized user create arguments are incomplete")
	}
	if record.Role != controlplane.RoleAdmin && record.Role != controlplane.RoleUser {
		return controlplane.UserSummary{}, errors.New("normalized user create arguments are incomplete")
	}
	createdAt := record.CreatedAt.UTC()
	if createdAt.IsZero() {
		createdAt = s.Now()
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

	userID, err := newRepositoryID("usr")
	if err != nil {
		return controlplane.UserSummary{}, fmt.Errorf("generate user id: %w", err)
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, userID, createdAt)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserByID(operationCtx, tx, storedResourceID)
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, record.Username, record.PasswordHash, record.Role, controlplane.UserStatusActive, createdAt); err != nil {
		var pqErr *pq.Error
		if errors.As(err, &pqErr) && pqErr.Code == "23505" {
			return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
		}
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("insert normalized user: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized user", err)
	}
	return controlplane.UserSummary{
		ID:        userID,
		Username:  record.Username,
		Role:      record.Role,
		Status:    controlplane.UserStatusActive,
		CreatedAt: createdAt.Format(time.RFC3339),
	}, nil
}

func (s *PostgresRepository) UpdateUser(ctx context.Context, scope, idempotencyKey, fingerprint string, record UserUpdateRecord) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, errors.New("normalized user repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserSummary{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || record.UserID == "" || (record.Username == nil && record.Role == nil && record.Status == nil) {
		return controlplane.UserSummary{}, errors.New("normalized user update arguments are incomplete")
	}
	if record.Username != nil {
		username := strings.TrimSpace(*record.Username)
		record.Username = &username
		if username == "" {
			return controlplane.UserSummary{}, errors.New("normalized user username must not be empty")
		}
	}
	if record.Role != nil && *record.Role != controlplane.RoleAdmin && *record.Role != controlplane.RoleUser {
		return controlplane.UserSummary{}, errors.New("normalized user role is invalid")
	}
	if record.Status != nil && *record.Status != controlplane.UserStatusActive && *record.Status != controlplane.UserStatusDisabled {
		return controlplane.UserSummary{}, errors.New("normalized user status is invalid")
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
	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if user.ID == "usr_local_admin" {
		return controlplane.UserSummary{}, controlplane.ErrCannotModifyLocalAdmin
	}
	createdAt := s.Now()
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, record.UserID, createdAt)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserByID(operationCtx, tx, storedResourceID)
	}

	candidate := user
	if record.Username != nil {
		candidate.Username = *record.Username
	}
	if record.Role != nil {
		candidate.Role = *record.Role
	}
	if record.Status != nil {
		candidate.Status = *record.Status
	}
	if candidate.Username != user.Username {
		var duplicate bool
		if err := tx.QueryRowContext(operationCtx, `
			SELECT EXISTS(SELECT 1 FROM users WHERE username = $1 AND id <> $2)
		`, candidate.Username, user.ID).Scan(&duplicate); err != nil {
			return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("check normalized username uniqueness: %w", err))
		}
		if duplicate {
			return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
		}
	}
	if candidate.Role != controlplane.RoleAdmin || candidate.Status != controlplane.UserStatusActive {
		var activeAdmins int
		if err := tx.QueryRowContext(operationCtx, `
			SELECT COUNT(*) FROM users WHERE role = $1 AND status = $2 AND id <> $3
		`, controlplane.RoleAdmin, controlplane.UserStatusActive, user.ID).Scan(&activeAdmins); err != nil {
			return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("count normalized active admins: %w", err))
		}
		if activeAdmins == 0 {
			return controlplane.UserSummary{}, controlplane.ErrLastActiveAdmin
		}
	}
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE users SET username = $2, role = $3, status = $4 WHERE id = $1
	`, user.ID, candidate.Username, candidate.Role, candidate.Status); err != nil {
		var pqErr *pq.Error
		if errors.As(err, &pqErr) && pqErr.Code == "23505" {
			return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
		}
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized user: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized user update", err)
	}
	return candidate, nil
}

func (s *PostgresRepository) DisableUser(ctx context.Context, scope, idempotencyKey, fingerprint, userID string) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, errors.New("normalized user repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserSummary{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	userID = strings.TrimSpace(userID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || userID == "" {
		return controlplane.UserSummary{}, errors.New("normalized user disable arguments are incomplete")
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
	user, err := s.loadUserForUpdate(operationCtx, tx, userID)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if user.ID == "usr_local_admin" {
		return controlplane.UserSummary{}, controlplane.ErrCannotDisableLocalAdmin
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, userID, s.Now())
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserByID(operationCtx, tx, storedResourceID)
	}
	if user.Status == controlplane.UserStatusDisabled {
		if err := tx.Commit(); err != nil {
			return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized disabled user replay", err)
		}
		return user, nil
	}
	var activeAdmins int
	if err := tx.QueryRowContext(operationCtx, `
		SELECT COUNT(*) FROM users WHERE role = $1 AND status = $2 AND id <> $3
	`, controlplane.RoleAdmin, controlplane.UserStatusActive, user.ID).Scan(&activeAdmins); err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("count normalized active admins: %w", err))
	}
	if activeAdmins == 0 {
		return controlplane.UserSummary{}, controlplane.ErrLastActiveAdmin
	}
	user.Status = controlplane.UserStatusDisabled
	if _, err := tx.ExecContext(operationCtx, `UPDATE users SET status = $2 WHERE id = $1`, user.ID, controlplane.UserStatusDisabled); err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("disable normalized user: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized user disable", err)
	}
	return user, nil
}

func (s *PostgresRepository) ResetUserPassword(ctx context.Context, scope, idempotencyKey, fingerprint, userID string, passwordHash []byte) (controlplane.UserSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserSummary{}, errors.New("normalized user repository requires normalized read source")
	}
	if err := ctx.Err(); err != nil {
		return controlplane.UserSummary{}, err
	}
	scope = strings.TrimSpace(scope)
	idempotencyKey = strings.TrimSpace(idempotencyKey)
	fingerprint = strings.TrimSpace(fingerprint)
	userID = strings.TrimSpace(userID)
	if scope == "" || idempotencyKey == "" || fingerprint == "" || userID == "" || len(passwordHash) == 0 {
		return controlplane.UserSummary{}, errors.New("normalized password reset arguments are incomplete")
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
	user, err := s.loadUserForUpdate(operationCtx, tx, userID)
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if user.ID == "usr_local_admin" {
		return controlplane.UserSummary{}, controlplane.ErrCannotModifyLocalAdmin
	}
	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, scope, idempotencyKey, fingerprint, userID, s.Now())
	if err != nil {
		return controlplane.UserSummary{}, err
	}
	if !inserted {
		if storedFingerprint != fingerprint {
			return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadUserByID(operationCtx, tx, storedResourceID)
	}
	if _, err := tx.ExecContext(operationCtx, `UPDATE users SET password_hash = $2 WHERE id = $1`, userID, passwordHash); err != nil {
		return controlplane.UserSummary{}, postgresOperationError(operationCtx, fmt.Errorf("reset normalized user password: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserSummary{}, postgresCommitError(operationCtx, "commit normalized password reset", err)
	}
	return user, nil
}

func (s *PostgresRepository) reserveUserIdempotency(ctx context.Context, tx *sql.Tx, scope, idempotencyKey, fingerprint, resourceID string, createdAt time.Time) (string, string, bool, error) {
	var storedFingerprint, storedResourceID string
	err := tx.QueryRowContext(ctx, `
		INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)
		VALUES ($1, $2, $3, $4, $5)
		ON CONFLICT (scope, idempotency_key) DO NOTHING
		RETURNING fingerprint, resource_id
	`, scope, idempotencyKey, fingerprint, resourceID, createdAt).Scan(&storedFingerprint, &storedResourceID)
	if err == nil {
		return storedFingerprint, storedResourceID, true, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return "", "", false, postgresOperationError(ctx, fmt.Errorf("insert user idempotency record: %w", err))
	}
	if err := tx.QueryRowContext(ctx, `
		SELECT fingerprint, resource_id
		FROM idempotency_records
		WHERE scope = $1 AND idempotency_key = $2
		FOR UPDATE
	`, scope, idempotencyKey).Scan(&storedFingerprint, &storedResourceID); err != nil {
		return "", "", false, postgresOperationError(ctx, fmt.Errorf("load existing user idempotency record: %w", err))
	}
	return storedFingerprint, storedResourceID, false, nil
}

func (s *PostgresRepository) loadUserByID(ctx context.Context, tx *sql.Tx, userID string) (controlplane.UserSummary, error) {
	var user controlplane.UserSummary
	var createdAt time.Time
	if err := tx.QueryRowContext(ctx, `
		SELECT id, username, role, status, created_at
		FROM users
		WHERE id = $1
	`, userID).Scan(&user.ID, &user.Username, &user.Role, &user.Status, &createdAt); err != nil {
		return controlplane.UserSummary{}, postgresOperationError(ctx, fmt.Errorf("load idempotent normalized user: %w", err))
	}
	user.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return user, nil
}

func (s *PostgresRepository) loadUserForUpdate(ctx context.Context, tx *sql.Tx, userID string) (controlplane.UserSummary, error) {
	var user controlplane.UserSummary
	var createdAt time.Time
	if err := tx.QueryRowContext(ctx, `
		SELECT id, username, role, status, created_at
		FROM users
		WHERE id = $1
		FOR UPDATE
	`, userID).Scan(&user.ID, &user.Username, &user.Role, &user.Status, &createdAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		return controlplane.UserSummary{}, postgresOperationError(ctx, fmt.Errorf("lock normalized user: %w", err))
	}
	user.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return user, nil
}

func newRepositoryID(prefix string) (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	return prefix + "_" + hex.EncodeToString(raw[:]), nil
}
