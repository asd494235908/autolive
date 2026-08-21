package store

import (
	"context"
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
	session := AuthSession{ID: "session_timeout", UserID: "user_1", Product: controlplane.ProductAutoLive, AccessTokenHash: "access_hash", RefreshTokenHash: "refresh_hash", AccessExpiresAt: now.Add(time.Hour), RefreshExpiresAt: now.Add(24 * time.Hour), CreatedAt: now}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(session.ID, session.UserID, session.Product, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash, session.AccessExpiresAt, session.RefreshExpiresAt, session.CreatedAt).
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
		AccessTokenHash:  "access_hash",
		RefreshTokenHash: "refresh_hash",
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
	}
	if err := validateAuthSession(valid); err != nil {
		t.Fatalf("validateAuthSession(valid) error = %v", err)
	}
	invalid := valid
	invalid.RefreshExpiresAt = invalid.AccessExpiresAt
	if err := validateAuthSession(invalid); err == nil {
		t.Fatal("validateAuthSession(equal expiry) error = nil")
	}
}

func TestSQLSessionStoreCreatePersistsOnlySessionHashesAndMetadata(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	session := AuthSession{
		ID:               "session_1",
		UserID:           "user_1",
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  "access_hash",
		RefreshTokenHash: "refresh_hash",
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(session.ID, session.UserID, session.Product, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash, session.AccessExpiresAt, session.RefreshExpiresAt, session.CreatedAt).
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
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_id", "access_token_hash", "refresh_token_hash",
			"access_expires_at", "refresh_expires_at", "created_at",
		}).AddRow("session_1", "user_1", "douyin_desktop", nil, "access_hash", "refresh_hash", now.Add(time.Hour), now.Add(24*time.Hour), now))
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access-null-product").
		WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_id", "access_token_hash", "refresh_token_hash", "access_expires_at", "refresh_expires_at", "created_at"}).
			AddRow("session-1", "user-1", nil, nil, "access-null-product", "refresh-1", now.Add(time.Hour), now.Add(24*time.Hour), now))

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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("access-invalid-product").
		WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_id", "access_token_hash", "refresh_token_hash", "access_expires_at", "refresh_expires_at", "created_at"}).
			AddRow("session-1", "user-1", "unknown", nil, "access-invalid-product", "refresh-1", now.Add(time.Hour), now.Add(24*time.Hour), now))

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
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP")).
		WithArgs("user_1").WillReturnResult(sqlmock.NewResult(1, 2))
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
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP")).
		WithArgs("device_1").WillReturnResult(sqlmock.NewResult(1, 2))
	if err := store.RevokeByDeviceID(context.Background(), "device_1"); err != nil {
		t.Fatalf("RevokeByDeviceID() error: %v", err)
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
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	next := AuthSession{
		ID:               "session_2",
		UserID:           "user_1",
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  "new_access_hash",
		RefreshTokenHash: "new_refresh_hash",
		AccessExpiresAt:  now.Add(2 * time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_id", "access_token_hash", "refresh_token_hash",
			"access_expires_at", "refresh_expires_at", "created_at",
		}).AddRow("session_1", "user_1", "autolive", nil, "access_hash", "refresh_hash", now.Add(time.Hour), now.Add(24*time.Hour), now))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1")).
		WithArgs("session_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(next.ID, next.UserID, next.Product, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash, next.AccessExpiresAt, next.RefreshExpiresAt, next.CreatedAt).
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
	store, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("NewSQLSessionStore() error = %v", err)
	}
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	next := AuthSession{
		ID: "session_2", UserID: "other_user", Product: controlplane.ProductAutoLive, DeviceID: "device_2",
		AccessTokenHash: "new_access_hash", RefreshTokenHash: "new_refresh_hash",
		AccessExpiresAt: now.Add(2 * time.Hour), RefreshExpiresAt: now.Add(48 * time.Hour), CreatedAt: now,
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_id", "access_token_hash", "refresh_token_hash",
			"access_expires_at", "refresh_expires_at", "created_at",
		}).AddRow("session_1", "user_1", "autolive", "device_1", "access_hash", "refresh_hash", now.Add(time.Hour), now.Add(24*time.Hour), now))
	mock.ExpectRollback()
	if _, found, err := store.Rotate(context.Background(), "refresh_hash", next); err == nil || found {
		t.Fatalf("Rotate() = found=%t error=%v, want validation failure", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
