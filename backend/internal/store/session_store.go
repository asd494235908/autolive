package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"

	"autoLive/backend/internal/controlplane"
)

// AuthSession 是鉴权会话的持久化形状，只包含 Token 哈希，不包含 Token 原文。
type AuthSession struct {
	ID               string
	UserID           string
	Product          controlplane.ProductCode
	DeviceID         string
	AccessTokenHash  string
	RefreshTokenHash string
	AccessExpiresAt  time.Time
	RefreshExpiresAt time.Time
	CreatedAt        time.Time
}

// SessionStore 为鉴权层提供可替换的会话持久化边界。
// Rotate 必须在一个数据库事务中撤销旧 Refresh Token 并插入新会话。
type SessionStore interface {
	Create(ctx context.Context, session AuthSession) error
	GetByAccessTokenHash(ctx context.Context, accessTokenHash string) (AuthSession, bool, error)
	GetByRefreshTokenHash(ctx context.Context, refreshTokenHash string) (AuthSession, bool, error)
	Rotate(ctx context.Context, refreshTokenHash string, next AuthSession) (AuthSession, bool, error)
	RevokeByAccessTokenHash(ctx context.Context, accessTokenHash string) error
	RevokeByUserID(ctx context.Context, userID string) error
	RevokeByDeviceID(ctx context.Context, deviceID string) error
	UpdateDeviceID(ctx context.Context, accessTokenHash, deviceID string) error
	ClearDeviceID(ctx context.Context, accessTokenHash, deviceID string) error
}

// SQLSessionStore 是 PostgreSQL 会话实现。它不缓存会话，进程重启后可直接从数据库恢复。
type SQLSessionStore struct {
	db               *sql.DB
	now              func() time.Time
	operationTimeout time.Duration
}

func NewSQLSessionStore(db *sql.DB, now func() time.Time) (*SQLSessionStore, error) {
	return NewSQLSessionStoreWithTimeout(db, now, defaultPostgresOperationTimeout)
}

func NewSQLSessionStoreWithTimeout(db *sql.DB, now func() time.Time, operationTimeout time.Duration) (*SQLSessionStore, error) {
	if db == nil {
		return nil, errors.New("session store database must not be nil")
	}
	if now == nil {
		now = time.Now
	}
	if operationTimeout <= 0 {
		return nil, errors.New("session store operation timeout must be greater than zero")
	}
	return &SQLSessionStore{db: db, now: now, operationTimeout: operationTimeout}, nil
}

func (s *SQLSessionStore) operationContext(ctx context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(ctx, s.operationTimeout)
}

func (s *SQLSessionStore) Create(ctx context.Context, session AuthSession) error {
	if err := validateAuthSession(session); err != nil {
		return err
	}
	if session.CreatedAt.IsZero() {
		session.CreatedAt = s.now().UTC()
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, product, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at, device_bound_at
		)
		VALUES ($1, $2, $3, NULLIF($4, ''), $5, $6, $7, $8, $9,
			CASE WHEN NULLIF($4, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END)
	`, session.ID, session.UserID, session.Product, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash,
		session.AccessExpiresAt.UTC(), session.RefreshExpiresAt.UTC(), session.CreatedAt.UTC())
	return postgresOperationError(ctx, err)
}

func (s *SQLSessionStore) GetByAccessTokenHash(ctx context.Context, accessTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `access_token_hash = $1`, accessTokenHash)
}

func (s *SQLSessionStore) GetByRefreshTokenHash(ctx context.Context, refreshTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `refresh_token_hash = $1`, refreshTokenHash)
}

func (s *SQLSessionStore) get(ctx context.Context, predicate, value string) (AuthSession, bool, error) {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	var session AuthSession
	var product sql.NullString
	var deviceID sql.NullString
	err := s.db.QueryRowContext(ctx, `
		SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		FROM auth_sessions
		WHERE `+predicate+` AND revoked_at IS NULL
		LIMIT 1
	`, value).Scan(
		&session.ID, &session.UserID, &product, &deviceID, &session.AccessTokenHash, &session.RefreshTokenHash,
		&session.AccessExpiresAt, &session.RefreshExpiresAt, &session.CreatedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if err := assignSessionProduct(&session, product); err != nil {
		return AuthSession{}, false, err
	}
	if deviceID.Valid {
		session.DeviceID = deviceID.String
	}
	return session, true, nil
}

func (s *SQLSessionStore) Rotate(ctx context.Context, refreshTokenHash string, next AuthSession) (AuthSession, bool, error) {
	if err := validateAuthSession(next); err != nil {
		return AuthSession{}, false, err
	}
	if next.CreatedAt.IsZero() {
		next.CreatedAt = s.now().UTC()
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	defer func() { _ = tx.Rollback() }()

	var old AuthSession
	var oldProduct sql.NullString
	var deviceID sql.NullString
	err = tx.QueryRowContext(ctx, `
		SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		FROM auth_sessions
		WHERE refresh_token_hash = $1
		  AND revoked_at IS NULL
		  AND refresh_expires_at > CURRENT_TIMESTAMP
		FOR UPDATE
	`, refreshTokenHash).Scan(
		&old.ID, &old.UserID, &oldProduct, &deviceID, &old.AccessTokenHash, &old.RefreshTokenHash,
		&old.AccessExpiresAt, &old.RefreshExpiresAt, &old.CreatedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if err := assignSessionProduct(&old, oldProduct); err != nil {
		return AuthSession{}, false, err
	}
	if deviceID.Valid {
		old.DeviceID = deviceID.String
	}
	if next.UserID != old.UserID || next.Product != old.Product || next.DeviceID != old.DeviceID ||
		next.RefreshExpiresAt.After(old.RefreshExpiresAt) || next.AccessExpiresAt.After(old.RefreshExpiresAt) {
		return AuthSession{}, false, errors.New("rotated auth session changes identity or exceeds refresh expiry")
	}
	if _, err := tx.ExecContext(ctx, `UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1`, old.ID); err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, product, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at, device_bound_at
		)
		VALUES ($1, $2, $3, NULLIF($4, ''), $5, $6, $7, $8, $9,
			CASE WHEN NULLIF($4, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END)
	`, next.ID, next.UserID, next.Product, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash,
		next.AccessExpiresAt.UTC(), next.RefreshExpiresAt.UTC(), next.CreatedAt.UTC()); err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if err := tx.Commit(); err != nil {
		return AuthSession{}, false, postgresCommitError(ctx, "commit auth session rotation", err)
	}
	return old, true, nil
}

func (s *SQLSessionStore) RevokeByAccessTokenHash(ctx context.Context, accessTokenHash string) error {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP
		WHERE access_token_hash = $1 AND revoked_at IS NULL
	`, accessTokenHash)
	return postgresOperationError(ctx, err)
}

func (s *SQLSessionStore) RevokeByUserID(ctx context.Context, userID string) error {
	if userID == "" {
		return errors.New("user id is required")
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP
		WHERE user_id = $1 AND revoked_at IS NULL
	`, userID)
	return postgresOperationError(ctx, err)
}

func (s *SQLSessionStore) RevokeByDeviceID(ctx context.Context, deviceID string) error {
	if deviceID == "" {
		return errors.New("device id is required")
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP
		WHERE device_id = $1 AND revoked_at IS NULL
	`, deviceID)
	return postgresOperationError(ctx, err)
}

func (s *SQLSessionStore) UpdateDeviceID(ctx context.Context, accessTokenHash, deviceID string) error {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	result, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions
		SET device_id = NULLIF($2, ''),
			device_bound_at = CASE WHEN NULLIF($2, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END
		WHERE access_token_hash = $1
		  AND revoked_at IS NULL
		  AND (device_id IS NULL OR device_id = $2)
	`, accessTokenHash, deviceID)
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	rows, err := result.RowsAffected()
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	if rows != 1 {
		return errors.New("auth session was not updated for device binding")
	}
	return nil
}

func (s *SQLSessionStore) ClearDeviceID(ctx context.Context, accessTokenHash, deviceID string) error {
	if accessTokenHash == "" || deviceID == "" {
		return errors.New("access token hash and device id are required")
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	result, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET device_id = NULL, device_bound_at = NULL
		WHERE access_token_hash = $1
		  AND device_id = $2
		  AND revoked_at IS NULL
	`, accessTokenHash, deviceID)
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	rows, err := result.RowsAffected()
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	if rows != 1 {
		return errors.New("auth session was not cleared for device compensation")
	}
	return nil
}

func validateAuthSession(session AuthSession) error {
	if session.ID == "" || session.UserID == "" || session.AccessTokenHash == "" || session.RefreshTokenHash == "" ||
		!session.Product.Valid() || session.AccessExpiresAt.IsZero() || session.RefreshExpiresAt.IsZero() || !session.RefreshExpiresAt.After(session.AccessExpiresAt) {
		return errors.New("invalid auth session")
	}
	return nil
}

func assignSessionProduct(session *AuthSession, raw sql.NullString) error {
	if session == nil || !raw.Valid {
		return errors.New("auth session product is missing")
	}
	product, err := controlplane.ParseProductCode(raw.String)
	if err != nil {
		return fmt.Errorf("auth session product is invalid: %w", err)
	}
	session.Product = product
	return nil
}
