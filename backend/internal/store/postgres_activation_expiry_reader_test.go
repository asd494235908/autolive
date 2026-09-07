package store

import (
	"context"
	"database/sql"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryGetActivationExpiryReadsBoundCodeExpiry(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	expiresAt := now.Add(24 * time.Hour)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery("(?s)"+regexp.QuoteMeta("SELECT ac.expires_at")+".*"+regexp.QuoteMeta("ac.status IN ('active', 'used')")).WithArgs("usr_1", "dev_1").
		WillReturnRows(sqlmock.NewRows([]string{"expires_at"}).AddRow(expiresAt))
	mock.ExpectCommit()

	actual, err := repository.GetActivationExpiry(context.Background(), "usr_1", "dev_1")
	if err != nil {
		t.Fatalf("GetActivationExpiry() error = %v", err)
	}
	if actual == nil || !actual.Equal(expiresAt) {
		t.Fatalf("activation expiry = %v, want %v", actual, expiresAt)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetActivationExpiryRejectsRevokedBinding(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery("(?s)"+regexp.QuoteMeta("SELECT ac.expires_at")+".*"+regexp.QuoteMeta("ac.status IN ('active', 'used')")).WithArgs("usr_1", "dev_1").WillReturnError(sql.ErrNoRows)
	mock.ExpectCommit()

	actual, err := repository.GetActivationExpiry(context.Background(), "usr_1", "dev_1")
	if err != nil || actual != nil {
		t.Fatalf("GetActivationExpiry() = %v, %v; want nil, nil", actual, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
