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
	ID                 string
	UserID             string
	Product            controlplane.ProductCode
	Audience           SessionAudience
	DeviceID           string
	AccessTokenHash    string
	RefreshTokenHash   string
	RefreshFamilyID    string
	RefreshGeneration  int
	AccessExpiresAt    time.Time
	RefreshExpiresAt   time.Time
	CreatedAt          time.Time
	RevokedAt          time.Time
	ConsumedAt         time.Time
	RevokedReason      string
	RotatedToSessionID string
}

type SessionAudience string

const (
	SessionAudienceAdmin   SessionAudience = "admin"
	SessionAudienceDesktop SessionAudience = "desktop"
	SessionAudienceLegacy  SessionAudience = "legacy"
)

func (a SessionAudience) ValidForNewSession() bool {
	return a == SessionAudienceAdmin || a == SessionAudienceDesktop
}

var ErrRefreshTokenReplayed = errors.New("refresh token replayed")

// SessionStore 为鉴权层提供可替换的会话持久化边界。
// Rotate 必须在一个数据库事务中撤销旧 Refresh Token 并插入新会话。
type SessionStore interface {
	Create(ctx context.Context, session AuthSession) error
	GetByAccessTokenHash(ctx context.Context, accessTokenHash string) (AuthSession, bool, error)
	GetByRefreshTokenHash(ctx context.Context, refreshTokenHash string) (AuthSession, bool, error)
	Rotate(ctx context.Context, refreshTokenHash string, next AuthSession) (AuthSession, bool, error)
	RevokeByAccessTokenHash(ctx context.Context, accessTokenHash string) error
	RevokeByRefreshTokenHash(ctx context.Context, refreshTokenHash string) error
	RevokeByUserID(ctx context.Context, userID string) error
	RevokeByDeviceID(ctx context.Context, deviceID string) error
	RevokeByDeviceIDForProduct(ctx context.Context, deviceID string, product controlplane.ProductCode) error
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
			id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash,
			token_family_id, generation, access_expires_at, refresh_expires_at, created_at, device_bound_at
		)
		VALUES ($1, $2, $3, $4, NULLIF($5, ''), $6, $7, $8, $9, $10, $11, $12,
			CASE WHEN NULLIF($5, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END)
	`, session.ID, session.UserID, session.Product, session.Audience, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash,
		session.RefreshFamilyID, session.RefreshGeneration, session.AccessExpiresAt.UTC(), session.RefreshExpiresAt.UTC(), session.CreatedAt.UTC())
	return postgresOperationError(ctx, err)
}

func (s *SQLSessionStore) GetByAccessTokenHash(ctx context.Context, accessTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `access_token_hash = $1 AND revoked_at IS NULL`, accessTokenHash)
}

func (s *SQLSessionStore) GetByRefreshTokenHash(ctx context.Context, refreshTokenHash string) (AuthSession, bool, error) {
	return s.get(ctx, `refresh_token_hash = $1`, refreshTokenHash)
}

func (s *SQLSessionStore) get(ctx context.Context, predicate, value string) (AuthSession, bool, error) {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	var session AuthSession
	err := scanAuthSession(s.db.QueryRowContext(ctx, `
		SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash,
			token_family_id, generation, access_expires_at, refresh_expires_at, created_at,
			revoked_at, consumed_at, revoked_reason, rotated_to_session_id
		FROM auth_sessions
		WHERE `+predicate+`
		LIMIT 1
	`, value), &session)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
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

	familyID, found, err := lockRefreshTokenFamily(ctx, tx, refreshTokenHash)
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if !found {
		return AuthSession{}, false, nil
	}

	var old AuthSession
	err = scanAuthSession(tx.QueryRowContext(ctx, `
		SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash,
			token_family_id, generation, access_expires_at, refresh_expires_at, created_at,
			revoked_at, consumed_at, revoked_reason, rotated_to_session_id
		FROM auth_sessions
		WHERE refresh_token_hash = $1
		FOR UPDATE
	`, refreshTokenHash), &old)
	if errors.Is(err, sql.ErrNoRows) {
		return AuthSession{}, false, nil
	}
	if err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if old.RefreshFamilyID != familyID {
		return AuthSession{}, false, errors.New("auth session refresh family changed during rotation")
	}
	if !old.RevokedAt.IsZero() {
		if !old.ConsumedAt.IsZero() && old.RefreshFamilyID != "" {
			if _, err := tx.ExecContext(ctx, `
				UPDATE auth_sessions
				SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP),
					revoked_reason = CASE WHEN revoked_at IS NULL THEN 'refresh_replay' ELSE revoked_reason END
				WHERE token_family_id = $1
			`, old.RefreshFamilyID); err != nil {
				return AuthSession{}, false, postgresOperationError(ctx, err)
			}
			if err := tx.Commit(); err != nil {
				return AuthSession{}, false, postgresCommitError(ctx, "commit refresh replay revocation", err)
			}
			return old, false, ErrRefreshTokenReplayed
		}
		return old, false, nil
	}
	if !old.RefreshExpiresAt.After(s.now().UTC()) {
		return old, false, nil
	}
	if next.UserID != old.UserID || next.Product != old.Product || next.DeviceID != old.DeviceID ||
		next.Audience != old.Audience || next.RefreshFamilyID != old.RefreshFamilyID ||
		next.RefreshGeneration != old.RefreshGeneration+1 || next.RefreshExpiresAt.After(old.RefreshExpiresAt) ||
		next.AccessExpiresAt.After(old.RefreshExpiresAt) {
		return AuthSession{}, false, errors.New("rotated auth session changes identity or exceeds refresh expiry")
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE auth_sessions
		SET revoked_at = CURRENT_TIMESTAMP, consumed_at = CURRENT_TIMESTAMP, last_used_at = CURRENT_TIMESTAMP,
			revoked_reason = 'rotated', rotated_to_session_id = $2
		WHERE id = $1 AND revoked_at IS NULL
	`, old.ID, next.ID); err != nil {
		return AuthSession{}, false, postgresOperationError(ctx, err)
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash,
			token_family_id, generation, access_expires_at, refresh_expires_at, created_at, device_bound_at
		)
		VALUES ($1, $2, $3, $4, NULLIF($5, ''), $6, $7, $8, $9, $10, $11, $12,
			CASE WHEN NULLIF($5, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END)
	`, next.ID, next.UserID, next.Product, next.Audience, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash,
		next.RefreshFamilyID, next.RefreshGeneration, next.AccessExpiresAt.UTC(), next.RefreshExpiresAt.UTC(), next.CreatedAt.UTC()); err != nil {
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

func (s *SQLSessionStore) RevokeByRefreshTokenHash(ctx context.Context, refreshTokenHash string) error {
	if refreshTokenHash == "" {
		return errors.New("refresh token hash is required")
	}
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("begin refresh family revocation: %w", err))
	}
	defer func() { _ = tx.Rollback() }()
	familyID, found, err := lockRefreshTokenFamily(ctx, tx, refreshTokenHash)
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	if !found {
		return nil
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE auth_sessions
		SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP),
			revoked_reason = CASE WHEN revoked_at IS NULL THEN 'logout' ELSE revoked_reason END
		WHERE token_family_id = $1
	`, familyID); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("revoke refresh token family: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(ctx, "commit refresh token family revocation", err)
	}
	return nil
}

func lockRefreshTokenFamily(ctx context.Context, tx *sql.Tx, refreshTokenHash string) (string, bool, error) {
	var familyID string
	err := tx.QueryRowContext(ctx, `
		SELECT token_family_id FROM auth_sessions WHERE refresh_token_hash = $1
	`, refreshTokenHash).Scan(&familyID)
	if errors.Is(err, sql.ErrNoRows) {
		return "", false, nil
	}
	if err != nil {
		return "", false, fmt.Errorf("find refresh token family: %w", err)
	}
	rows, err := tx.QueryContext(ctx, `
		SELECT id FROM auth_sessions WHERE token_family_id = $1 ORDER BY generation FOR UPDATE
	`, familyID)
	if err != nil {
		return "", false, fmt.Errorf("lock refresh token family: %w", err)
	}
	defer rows.Close()
	for rows.Next() {
		var sessionID string
		if err := rows.Scan(&sessionID); err != nil {
			return "", false, fmt.Errorf("scan refresh token family lock: %w", err)
		}
	}
	if err := rows.Err(); err != nil {
		return "", false, fmt.Errorf("iterate refresh token family lock: %w", err)
	}
	if err := rows.Close(); err != nil {
		return "", false, fmt.Errorf("close refresh token family lock: %w", err)
	}
	return familyID, true, nil
}

func (s *SQLSessionStore) RevokeByUserID(ctx context.Context, userID string) error {
	if userID == "" {
		return errors.New("user id is required")
	}
	return s.lockAndRevokeSessions(ctx,
		`SELECT id FROM auth_sessions WHERE user_id = $1 ORDER BY token_family_id, generation FOR UPDATE`,
		`UPDATE auth_sessions
		 SET revoked_at = CURRENT_TIMESTAMP, revoked_reason = COALESCE(revoked_reason, 'user_sessions_revoked')
		 WHERE user_id = $1 AND revoked_at IS NULL`,
		userID,
	)
}

func (s *SQLSessionStore) RevokeByDeviceID(ctx context.Context, deviceID string) error {
	if deviceID == "" {
		return errors.New("device id is required")
	}
	return s.lockAndRevokeSessions(ctx,
		`SELECT id FROM auth_sessions WHERE device_id = $1 ORDER BY token_family_id, generation FOR UPDATE`,
		`UPDATE auth_sessions
		 SET revoked_at = CURRENT_TIMESTAMP, revoked_reason = COALESCE(revoked_reason, 'device_sessions_revoked')
		 WHERE device_id = $1 AND revoked_at IS NULL`,
		deviceID,
	)
}

func (s *SQLSessionStore) RevokeByDeviceIDForProduct(ctx context.Context, deviceID string, product controlplane.ProductCode) error {
	if deviceID == "" || !product.Valid() {
		return errors.New("device id and product are required")
	}
	return s.lockAndRevokeSessions(ctx,
		`SELECT id FROM auth_sessions WHERE device_id = $1 AND product = $2 ORDER BY token_family_id, generation FOR UPDATE`,
		`UPDATE auth_sessions
		 SET revoked_at = CURRENT_TIMESTAMP, revoked_reason = COALESCE(revoked_reason, 'device_sessions_revoked')
		 WHERE device_id = $1 AND product = $2 AND revoked_at IS NULL`,
		deviceID, product,
	)
}

func (s *SQLSessionStore) lockAndRevokeSessions(ctx context.Context, lockQuery, updateQuery string, args ...any) error {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("begin auth session revocation: %w", err))
	}
	defer func() { _ = tx.Rollback() }()
	rows, err := tx.QueryContext(ctx, lockQuery, args...)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("lock auth sessions for revocation: %w", err))
	}
	for rows.Next() {
		var sessionID string
		if err := rows.Scan(&sessionID); err != nil {
			_ = rows.Close()
			return postgresOperationError(ctx, fmt.Errorf("scan auth session revocation lock: %w", err))
		}
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return postgresOperationError(ctx, fmt.Errorf("iterate auth session revocation locks: %w", err))
	}
	if err := rows.Close(); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("close auth session revocation locks: %w", err))
	}
	if _, err := tx.ExecContext(ctx, updateQuery, args...); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("revoke auth sessions: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(ctx, "commit auth session revocation", err)
	}
	return nil
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
		session.RefreshFamilyID == "" || session.RefreshGeneration < 0 || !session.Audience.ValidForNewSession() ||
		!session.Product.Valid() || session.AccessExpiresAt.IsZero() || session.RefreshExpiresAt.IsZero() || session.RefreshExpiresAt.Before(session.AccessExpiresAt) {
		return errors.New("invalid auth session")
	}
	return nil
}

type authSessionScanner interface {
	Scan(dest ...any) error
}

func scanAuthSession(scanner authSessionScanner, session *AuthSession) error {
	var product, audience, deviceID, revokedReason, rotatedToSessionID sql.NullString
	var revokedAt, consumedAt sql.NullTime
	if err := scanner.Scan(
		&session.ID, &session.UserID, &product, &audience, &deviceID, &session.AccessTokenHash, &session.RefreshTokenHash,
		&session.RefreshFamilyID, &session.RefreshGeneration, &session.AccessExpiresAt, &session.RefreshExpiresAt, &session.CreatedAt,
		&revokedAt, &consumedAt, &revokedReason, &rotatedToSessionID,
	); err != nil {
		return err
	}
	if err := assignSessionProduct(session, product); err != nil {
		return err
	}
	if !audience.Valid {
		return errors.New("auth session audience is missing")
	}
	session.Audience = SessionAudience(audience.String)
	if session.Audience != SessionAudienceAdmin && session.Audience != SessionAudienceDesktop && session.Audience != SessionAudienceLegacy {
		return errors.New("auth session audience is invalid")
	}
	if deviceID.Valid {
		session.DeviceID = deviceID.String
	}
	if revokedAt.Valid {
		session.RevokedAt = revokedAt.Time
	}
	if consumedAt.Valid {
		session.ConsumedAt = consumedAt.Time
	}
	if revokedReason.Valid {
		session.RevokedReason = revokedReason.String
	}
	if rotatedToSessionID.Valid {
		session.RotatedToSessionID = rotatedToSessionID.String
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
