package store

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestNewSQLSessionStoreRejectsNilDatabase(t *testing.T) {
	if _, err := NewSQLSessionStore(nil, time.Now); err == nil {
		t.Fatal("NewSQLSessionStore(nil) error = nil")
	}
}

func TestNewSQLSessionStoreRejectsNonPositiveOperationTimeout(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	if _, err := NewSQLSessionStoreWithTimeout(database, time.Now, 0); err == nil {
		t.Fatal("NewSQLSessionStoreWithTimeout() error = nil, want invalid timeout")
	}
}

func TestSQLSessionStoreCreateHonorsOperationTimeout(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStoreWithTimeout(database, time.Now, 10*time.Millisecond)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	session := AuthSession{ID: "session_timeout", UserID: "user_1", Product: controlplane.ProductAutoLive, Audience: SessionAudienceAdmin, AccessTokenHash: "access_hash", RefreshTokenHash: "refresh_hash", RefreshFamilyID: "family_1", AccessExpiresAt: now.Add(time.Hour), RefreshExpiresAt: now.Add(24 * time.Hour), CreatedAt: now}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(session.ID, session.UserID, session.Product, session.Audience, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash, session.RefreshFamilyID, session.RefreshGeneration, session.AccessExpiresAt, session.RefreshExpiresAt, session.CreatedAt).
		WillDelayFor(50 * time.Millisecond).
		WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.Create(context.Background(), session); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Create() error = %v, want context deadline exceeded", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestValidateAuthSessionRequiresHashesAndOrderedExpiry(t *testing.T) {
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	valid := AuthSession{
		ID:               "session_1",
		UserID:           "user_1",
		Product:          controlplane.ProductAutoLive,
		Audience:         SessionAudienceAdmin,
		AccessTokenHash:  "access_hash",
		RefreshTokenHash: "refresh_hash",
		RefreshFamilyID:  "family_1",
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
	}
	if err := validateAuthSession(valid); err != nil {
		t.Fatalf("validateAuthSession(valid) error = %v", err)
	}
	equalExpiry := valid
	equalExpiry.RefreshExpiresAt = equalExpiry.AccessExpiresAt
	if err := validateAuthSession(equalExpiry); err != nil {
		t.Fatalf("validateAuthSession(equal expiry) error = %v", err)
	}
	invalid := valid
	invalid.RefreshExpiresAt = invalid.AccessExpiresAt.Add(-time.Nanosecond)
	if err := validateAuthSession(invalid); err == nil {
		t.Fatal("validateAuthSession(refresh before access) error = nil")
	}
}

func TestSQLSessionStoreCreatePersistsOnlySessionHashesAndMetadata(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	store, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	session := AuthSession{
		ID:               "session_1",
		UserID:           "user_1",
		Product:          controlplane.ProductAutoLive,
		Audience:         SessionAudienceAdmin,
		AccessTokenHash:  "access_hash",
		RefreshTokenHash: "refresh_hash",
		RefreshFamilyID:  "family_1",
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(session.ID, session.UserID, session.Product, session.Audience, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash, session.RefreshFamilyID, session.RefreshGeneration, session.AccessExpiresAt, session.RefreshExpiresAt, session.CreatedAt).
		WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.Create(context.Background(), session); err != nil {
		t.Fatalf("Create() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreGetRestoresProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	store, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash",
			"token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at",
			"revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id",
		}).AddRow("session_1", "user_1", "douyin_desktop", "desktop", nil, "access_hash", "refresh_hash", "family_1", 0, now.Add(time.Hour), now.Add(24*time.Hour), now, nil, nil, nil, nil))
	session, found, err := store.GetByAccessTokenHash(context.Background(), "access_hash")
	if err != nil || !found {
		t.Fatalf("GetByAccessTokenHash() = (%+v, %t, %v), want found session", session, found, err)
	}
	if session.Product != "douyin_desktop" {
		t.Fatalf("session product = %q, want douyin_desktop", session.Product)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreGetRejectsNullProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access-null-product").
		WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash", "token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at", "revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id"}).
			AddRow("session-1", "user-1", nil, "admin", nil, "access-null-product", "refresh-1", "family-1", 0, now.Add(time.Hour), now.Add(24*time.Hour), now, nil, nil, nil, nil))

	if _, found, err := sessions.GetByAccessTokenHash(context.Background(), "access-null-product"); err == nil || found {
		t.Fatalf("GetByAccessTokenHash() = found %t, error %v; want fail closed", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreGetRejectsInvalidProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access-invalid-product").
		WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash", "token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at", "revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id"}).
			AddRow("session-1", "user-1", "unknown", "admin", nil, "access-invalid-product", "refresh-1", "family-1", 0, now.Add(time.Hour), now.Add(24*time.Hour), now, nil, nil, nil, nil))

	if _, found, err := sessions.GetByAccessTokenHash(context.Background(), "access-invalid-product"); err == nil || found {
		t.Fatalf("GetByAccessTokenHash() = found %t, error %v; want fail closed", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRevokeByUserID(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id FROM auth_sessions WHERE user_id = $1 ORDER BY token_family_id, generation FOR UPDATE")).
		WithArgs("user_1").WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow("session_1").AddRow("session_2"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions")).
		WithArgs("user_1").WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectCommit()
	if err := store.RevokeByUserID(context.Background(), "user_1"); err != nil {
		t.Fatalf("RevokeByUserID() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRevokeByDeviceID(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error: %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error: %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id FROM auth_sessions WHERE device_id = $1 ORDER BY token_family_id, generation FOR UPDATE")).
		WithArgs("device_1").WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow("session_1").AddRow("session_2"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions")).
		WithArgs("device_1").WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectCommit()
	if err := store.RevokeByDeviceID(context.Background(), "device_1"); err != nil {
		t.Fatalf("RevokeByDeviceID() error: %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRevokeByDeviceIDForProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error: %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id FROM auth_sessions WHERE device_id = $1 AND product = $2 ORDER BY token_family_id, generation FOR UPDATE")).
		WithArgs("device_1", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow("session_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions")).
		WithArgs("device_1", controlplane.ProductDouyinDesktop).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	if err := store.RevokeByDeviceIDForProduct(context.Background(), "device_1", controlplane.ProductDouyinDesktop); err != nil {
		t.Fatalf("RevokeByDeviceIDForProduct() error: %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreDeviceBindingChecksAffectedRowsAndSupportsCompensation(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = NULLIF($2, '')")).
		WithArgs("access_hash", "device_1").WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.UpdateDeviceID(context.Background(), "access_hash", "device_1"); err != nil {
		t.Fatalf("UpdateDeviceID() error = %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = NULL")).
		WithArgs("access_hash", "device_1").WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.ClearDeviceID(context.Background(), "access_hash", "device_1"); err != nil {
		t.Fatalf("ClearDeviceID() error = %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = NULLIF($2, '')")).
		WithArgs("access_hash", "device_2").WillReturnResult(sqlmock.NewResult(1, 0))
	if err := store.UpdateDeviceID(context.Background(), "access_hash", "device_2"); err == nil {
		t.Fatal("UpdateDeviceID() error = nil, want affected-row failure")
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = NULL")).
		WithArgs("access_hash", "device_2").WillReturnResult(sqlmock.NewResult(1, 0))
	if err := store.ClearDeviceID(context.Background(), "access_hash", "device_2"); err == nil {
		t.Fatal("ClearDeviceID() error = nil, want affected-row failure")
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreDeviceBindingWritesRecoveryMarker(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("new session store: %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("device_bound_at = CASE WHEN NULLIF($2, '') IS NULL THEN NULL ELSE CURRENT_TIMESTAMP END")).
		WithArgs("access_hash", "device_1").WillReturnResult(sqlmock.NewResult(0, 1))
	if err := sessions.UpdateDeviceID(context.Background(), "access_hash", "device_1"); err != nil {
		t.Fatalf("UpdateDeviceID() error = %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = NULL, device_bound_at = NULL")).
		WithArgs("access_hash", "device_1").WillReturnResult(sqlmock.NewResult(0, 1))
	if err := sessions.ClearDeviceID(context.Background(), "access_hash", "device_1"); err != nil {
		t.Fatalf("ClearDeviceID() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRotateRollsBackWhenNewSessionInsertFails(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	store, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	next := AuthSession{
		ID:                "session_2",
		UserID:            "user_1",
		Product:           controlplane.ProductAutoLive,
		Audience:          SessionAudienceAdmin,
		AccessTokenHash:   "new_access_hash",
		RefreshTokenHash:  "new_refresh_hash",
		RefreshFamilyID:   "family_1",
		RefreshGeneration: 1,
		AccessExpiresAt:   now.Add(2 * time.Hour),
		RefreshExpiresAt:  now.Add(24 * time.Hour),
		CreatedAt:         now,
	}
	mock.ExpectBegin()
	expectSessionFamilyRotationLock(mock, "refresh_hash", "family_1", "session_1")
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash",
			"token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at",
			"revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id",
		}).AddRow("session_1", "user_1", "autolive", "admin", nil, "access_hash", "refresh_hash", "family_1", 0, now.Add(time.Hour), now.Add(24*time.Hour), now, nil, nil, nil, nil))
	mock.ExpectExec(regexp.QuoteMeta("SET revoked_at = CURRENT_TIMESTAMP, consumed_at = CURRENT_TIMESTAMP, last_used_at = CURRENT_TIMESTAMP")).
		WithArgs("session_1", next.ID).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(next.ID, next.UserID, next.Product, next.Audience, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash, next.RefreshFamilyID, next.RefreshGeneration, next.AccessExpiresAt, next.RefreshExpiresAt, next.CreatedAt).
		WillReturnError(errors.New("insert failed"))
	mock.ExpectRollback()
	if _, found, err := store.Rotate(context.Background(), "refresh_hash", next); err == nil || found {
		t.Fatalf("Rotate() = found=%t error=%v, want rollback error", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRotateRejectsIdentityOrExpiryExpansion(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	store, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	next := AuthSession{
		ID: "session_2", UserID: "other_user", Product: controlplane.ProductAutoLive, Audience: SessionAudienceAdmin, DeviceID: "device_2",
		AccessTokenHash: "new_access_hash", RefreshTokenHash: "new_refresh_hash",
		RefreshFamilyID: "family_1", RefreshGeneration: 1,
		AccessExpiresAt: now.Add(2 * time.Hour), RefreshExpiresAt: now.Add(48 * time.Hour), CreatedAt: now,
	}
	mock.ExpectBegin()
	expectSessionFamilyRotationLock(mock, "refresh_hash", "family_1", "session_1")
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash",
			"token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at",
			"revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id",
		}).AddRow("session_1", "user_1", "autolive", "admin", "device_1", "access_hash", "refresh_hash", "family_1", 0, now.Add(time.Hour), now.Add(24*time.Hour), now, nil, nil, nil, nil))
	mock.ExpectRollback()
	if _, found, err := store.Rotate(context.Background(), "refresh_hash", next); err == nil || found {
		t.Fatalf("Rotate() = found=%t error=%v, want validation failure", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRotateDetectsReplayAndRevokesTokenFamily(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 26, 0, 0, 0, 0, time.UTC)
	sessions, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	next := AuthSession{
		ID: "session_3", UserID: "user_1", Product: controlplane.ProductAutoLive, Audience: SessionAudienceAdmin,
		AccessTokenHash: "access_3", RefreshTokenHash: "refresh_3", RefreshFamilyID: "family_1", RefreshGeneration: 2,
		AccessExpiresAt: now.Add(15 * time.Minute), RefreshExpiresAt: now.Add(24 * time.Hour), CreatedAt: now,
	}
	mock.ExpectBegin()
	expectSessionFamilyRotationLock(mock, "refresh_1", "family_1", "session_1", "session_2")
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, audience, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_1").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "audience", "device_id", "access_token_hash", "refresh_token_hash",
			"token_family_id", "generation", "access_expires_at", "refresh_expires_at", "created_at",
			"revoked_at", "consumed_at", "revoked_reason", "rotated_to_session_id",
		}).AddRow("session_1", "user_1", "autolive", "admin", nil, "access_1", "refresh_1", "family_1", 0,
			now.Add(-time.Hour), now.Add(24*time.Hour), now.Add(-2*time.Hour), now.Add(-time.Hour), now.Add(-time.Hour), "rotated", "session_2"))
	mock.ExpectExec(regexp.QuoteMeta("WHERE token_family_id = $1")).
		WithArgs("family_1").WillReturnResult(sqlmock.NewResult(0, 2))
	mock.ExpectCommit()
	if _, found, err := sessions.Rotate(context.Background(), "refresh_1", next); !errors.Is(err, ErrRefreshTokenReplayed) || found {
		t.Fatalf("Rotate(replay) = found %t, error %v; want replay error", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func expectSessionFamilyRotationLock(mock sqlmock.Sqlmock, refreshHash, familyID string, sessionIDs ...string) {
	mock.ExpectQuery(regexp.QuoteMeta("SELECT token_family_id FROM auth_sessions WHERE refresh_token_hash = $1")).
		WithArgs(refreshHash).
		WillReturnRows(sqlmock.NewRows([]string{"token_family_id"}).AddRow(familyID))
	rows := sqlmock.NewRows([]string{"id"})
	for _, sessionID := range sessionIDs {
		rows.AddRow(sessionID)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id FROM auth_sessions WHERE token_family_id = $1 ORDER BY generation FOR UPDATE")).
		WithArgs(familyID).
		WillReturnRows(rows)
}

func TestSQLSessionStoreLogoutRevokesRefreshTokenFamilyIdempotently(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	mock.ExpectBegin()
	expectSessionFamilyRotationLock(mock, "refresh_hash", "family_1", "session_1", "session_2")
	mock.ExpectExec(regexp.QuoteMeta("WHERE token_family_id = $1")).
		WithArgs("family_1").WillReturnResult(sqlmock.NewResult(0, 2))
	mock.ExpectCommit()
	if err := sessions.RevokeByRefreshTokenHash(context.Background(), "refresh_hash"); err != nil {
		t.Fatalf("RevokeByRefreshTokenHash() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT token_family_id FROM auth_sessions WHERE refresh_token_hash = $1")).
		WithArgs("unknown_refresh_hash").WillReturnError(sql.ErrNoRows)
	mock.ExpectRollback()
	if err := sessions.RevokeByRefreshTokenHash(context.Background(), "unknown_refresh_hash"); err != nil {
		t.Fatalf("RevokeByRefreshTokenHash(unknown) error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
