package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryEnsureConfiguredAdminCreatesNormalizedRow(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, password_hash, role, status, created_at")).WillReturnRows(sqlmock.NewRows([]string{"id", "username", "password_hash", "role", "status", "created_at"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id FROM users WHERE username = $1 LIMIT 1")).WithArgs("admin").WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO users (id, username, password_hash, role, status, created_at)")).WithArgs("admin", []byte("hash"), controlplane.RoleAdmin, controlplane.UserStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	if err := repository.EnsureConfiguredAdmin(context.Background(), "admin", []byte("hash")); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCheckAdminReadyUsesNormalizedUsers(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS(")).WithArgs(controlplane.RoleAdmin, controlplane.UserStatusActive).WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	if err := repository.CheckAdminReady(context.Background()); err != nil {
		t.Fatalf("CheckAdminReady() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryChangeLocalAdminPasswordUsesIdempotentTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at")).WithArgs("usr_local_admin").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_local_admin", "admin", controlplane.RoleAdmin, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "change-local-admin-password:key-1", "fp-1", "usr_local_admin", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "usr_local_admin"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE users SET password_hash = $2 WHERE id = $1")).WithArgs("usr_local_admin", []byte("new-hash")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	user, err := repository.ChangeLocalAdminPassword(context.Background(), "control-plane-state", "change-local-admin-password:key-1", "fp-1", []byte("new-hash"))
	if err != nil {
		t.Fatalf("ChangeLocalAdminPassword() error = %v", err)
	}
	if user.ID != "usr_local_admin" {
		t.Fatalf("changed user = %+v", user)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
