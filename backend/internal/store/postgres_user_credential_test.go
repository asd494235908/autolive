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

func TestPostgresRepositoryGetUserCredentialUsesNormalizedUserRow(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, password_hash, role, status, created_at")).WithArgs("alice").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "password_hash", "role", "status", "created_at"}).AddRow("usr_1", "alice", []byte("$2a$04$hash"), controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	user, hash, err := repository.GetUserCredential(context.Background(), "alice")
	if err != nil {
		t.Fatalf("GetUserCredential() error = %v", err)
	}
	if user.ID != "usr_1" || user.Username != "alice" || string(hash) != "$2a$04$hash" || user.CreatedAt != now.Format(time.RFC3339) {
		t.Fatalf("credential = user:%+v hash:%q", user, hash)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetUserCredentialMapsMissingUserToUnauthenticated(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, password_hash, role, status, created_at")).WithArgs("missing").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "password_hash", "role", "status", "created_at"}))
	_, _, err = repository.GetUserCredential(context.Background(), "missing")
	if !errors.Is(err, controlplane.ErrUnauthenticated) {
		t.Fatalf("GetUserCredential() error = %v, want unauthenticated", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetUserByIDUsesNormalizedUserRow(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	user, err := repository.GetUserByID(context.Background(), "usr_1")
	if err != nil {
		t.Fatalf("GetUserByID() error = %v", err)
	}
	if user.ID != "usr_1" || user.Username != "alice" || user.CreatedAt != now.Format(time.RFC3339) {
		t.Fatalf("user = %+v", user)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
