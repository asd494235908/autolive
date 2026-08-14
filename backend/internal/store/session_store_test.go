package store

import (
	"context"
	"errors"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestNewSQLSessionStoreRejectsNilDatabase(t *testing.T) {
	if _, err := NewSQLSessionStore(nil, time.Now); err == nil {
		t.Fatal("NewSQLSessionStore(nil) error = nil")
	}
}

func TestValidateAuthSessionRequiresHashesAndOrderedExpiry(t *testing.T) {
	now := time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC)
	valid := AuthSession{
		ID:               "session_1",
		UserID:           "user_1",
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
		AccessTokenHash:  "access_hash",
		RefreshTokenHash: "refresh_hash",
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(session.ID, session.UserID, session.DeviceID, session.AccessTokenHash, session.RefreshTokenHash, session.AccessExpiresAt, session.RefreshExpiresAt, session.CreatedAt).
		WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.Create(context.Background(), session); err != nil {
		t.Fatalf("Create() error = %v", err)
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
		AccessTokenHash:  "new_access_hash",
		RefreshTokenHash: "new_refresh_hash",
		AccessExpiresAt:  now.Add(2 * time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "device_id", "access_token_hash", "refresh_token_hash",
			"access_expires_at", "refresh_expires_at", "created_at",
		}).AddRow("session_1", "user_1", nil, "access_hash", "refresh_hash", now.Add(time.Hour), now.Add(24*time.Hour), now))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1")).
		WithArgs("session_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO auth_sessions (")).
		WithArgs(next.ID, next.UserID, next.DeviceID, next.AccessTokenHash, next.RefreshTokenHash, next.AccessExpiresAt, next.RefreshExpiresAt, next.CreatedAt).
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
		ID: "session_2", UserID: "other_user", DeviceID: "device_2",
		AccessTokenHash: "new_access_hash", RefreshTokenHash: "new_refresh_hash",
		AccessExpiresAt: now.Add(2 * time.Hour), RefreshExpiresAt: now.Add(48 * time.Hour), CreatedAt: now,
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, device_id, access_token_hash, refresh_token_hash")).
		WithArgs("refresh_hash").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "device_id", "access_token_hash", "refresh_token_hash",
			"access_expires_at", "refresh_expires_at", "created_at",
		}).AddRow("session_1", "user_1", "device_1", "access_hash", "refresh_hash", now.Add(time.Hour), now.Add(24*time.Hour), now))
	mock.ExpectRollback()
	if _, found, err := store.Rotate(context.Background(), "refresh_hash", next); err == nil || found {
		t.Fatalf("Rotate() = found=%t error=%v, want validation failure", found, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
