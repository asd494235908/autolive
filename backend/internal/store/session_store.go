package store

import (
	"context"
	"database/sql"
	"errors"
	"time"
)

// AuthSession 是鉴权会话的持久化形状，只包含 Token 哈希，不包含 Token 原文。
type AuthSession struct {
	ID               string
	UserID           string
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
	UpdateDeviceID(ctx context.Context, accessTokenHash, deviceID string) error
}

// SQLSessionStore 是 PostgreSQL 会话实现。它不缓存会话，进程重启后可直接从数据库恢复。
type SQLSessionStore struct {
	db  *sql.DB
	now func() time.Time
}

func NewSQLSessionStore(db *sql.DB, now func() time.Time) (*SQLSessionStore, error) {
	if db == nil {
		return nil, errors.New("session store database must not be nil")
	}
	if now == nil {
		now = time.Now
	}
	return &SQLSessionStore{db: db, now: now}, nil
}

func (s *SQLSessionStore) Create(ctx context.Context, session AuthSession) error {
	if err := validateAuthSession(session); err != nil {
		return err
	}
	if session.CreatedAt.IsZero() {
		session.CreatedAt = s.now().UTC()
	}
	_, err := s.db.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		)
		VALUES ($1, $2, NULLIF($3, ''), $4, $5, $6, $7, $8)
	`, session.ID, session.UserID, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash,
		session.AccessExpiresAt.UTC(), session.RefreshExpiresAt.UTC(), session.CreatedAt.UTC())
	return err
}

func (s *SQLSessionStore) GetByAccessTokenHash(ctx context.Context, accessTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `access_token_hash = $1`, accessTokenHash)
}

func (s *SQLSessionStore) GetByRefreshTokenHash(ctx context.Context, refreshTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `refresh_token_hash = $1`, refreshTokenHash)
}

func (s *SQLSessionStore) get(ctx context.Context, predicate, value string) (AuthSession, bool, error) {
	var session AuthSession
	var deviceID sql.NullString
	err := s.db.QueryRowContext(ctx, `
		SELECT id, user_id, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		FROM auth_sessions
		WHERE `+predicate+` AND revoked_at IS NULL
		LIMIT 1
	`, value).Scan(
		&session.ID, &session.UserID, &deviceID, &session.AccessTokenHash, &session.RefreshTokenHash,
		&session.AccessExpiresAt, &session.RefreshExpiresAt, &session.CreatedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
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
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return AuthSession{}, false, err
	}
	defer func() { _ = tx.Rollback() }()

	var old AuthSession
	var deviceID sql.NullString
	err = tx.QueryRowContext(ctx, `
		SELECT id, user_id, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		FROM auth_sessions
		WHERE refresh_token_hash = $1
		  AND revoked_at IS NULL
		  AND refresh_expires_at > CURRENT_TIMESTAMP
		FOR UPDATE
	`, refreshTokenHash).Scan(
		&old.ID, &old.UserID, &deviceID, &old.AccessTokenHash, &old.RefreshTokenHash,
		&old.AccessExpiresAt, &old.RefreshExpiresAt, &old.CreatedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
		return AuthSession{}, false, err
	}
	if deviceID.Valid {
		old.DeviceID = deviceID.String
	}
	if next.UserID != old.UserID || next.DeviceID != old.DeviceID ||
		next.RefreshExpiresAt.After(old.RefreshExpiresAt) || next.AccessExpiresAt.After(old.RefreshExpiresAt) {
		return AuthSession{}, false, errors.New("rotated auth session changes identity or exceeds refresh expiry")
	}
	if _, err := tx.ExecContext(ctx, `UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1`, old.ID); err != nil {
		return AuthSession{}, false, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		)
		VALUES ($1, $2, NULLIF($3, ''), $4, $5, $6, $7, $8)
	`, next.ID, next.UserID, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash,
		next.AccessExpiresAt.UTC(), next.RefreshExpiresAt.UTC(), next.CreatedAt.UTC()); err != nil {
		return AuthSession{}, false, err
	}
	if err := tx.Commit(); err != nil {
		return AuthSession{}, false, err
	}
	return old, true, nil
}

func (s *SQLSessionStore) RevokeByAccessTokenHash(ctx context.Context, accessTokenHash string) error {
	_, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP
		WHERE access_token_hash = $1 AND revoked_at IS NULL
	`, accessTokenHash)
	return err
}

func (s *SQLSessionStore) UpdateDeviceID(ctx context.Context, accessTokenHash, deviceID string) error {
	_, err := s.db.ExecContext(ctx, `
		UPDATE auth_sessions SET device_id = NULLIF($2, '')
		WHERE access_token_hash = $1
		  AND revoked_at IS NULL
		  AND (device_id IS NULL OR device_id = $2)
	`, accessTokenHash, deviceID)
	return err
}

func validateAuthSession(session AuthSession) error {
	if session.ID == "" || session.UserID == "" || session.AccessTokenHash == "" || session.RefreshTokenHash == "" ||
		session.AccessExpiresAt.IsZero() || session.RefreshExpiresAt.IsZero() || !session.RefreshExpiresAt.After(session.AccessExpiresAt) {
		return errors.New("invalid auth session")
	}
	return nil
}
