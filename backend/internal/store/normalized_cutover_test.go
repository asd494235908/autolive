package store

import (
	"context"
	"errors"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestVerifyNormalizedCutoverRequiresNormalizedSource(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceSnapshot)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	if _, err := repository.VerifyNormalizedCutover(context.Background()); !errors.Is(err, ErrNormalizedCutoverRequiresNormalizedSource) {
		t.Fatalf("VerifyNormalizedCutover() error = %v, want normalized source error", err)
	}
}

func TestVerifyNormalizedCutoverReadOnlyChecksTablesAndSecretReferences(t *testing.T) {
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
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status, COALESCE(completed_at, TIMESTAMPTZ 'epoch')")).WillReturnRows(
		sqlmock.NewRows([]string{"status", "coalesce"}).AddRow("completed", now),
	)
	for _, table := range normalizedCutoverTables {
		mock.ExpectQuery(regexp.QuoteMeta("SELECT to_regclass($1)")).WithArgs("public." + table).WillReturnRows(
			sqlmock.NewRows([]string{"to_regclass"}).AddRow(table),
		)
		mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM public." + table)).WillReturnRows(
			sqlmock.NewRows([]string{"count"}).AddRow(int64(0)),
		)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*)")).WillReturnRows(
		sqlmock.NewRows([]string{"count"}).AddRow(int64(0)),
	)
	mock.ExpectRollback()

	report, err := repository.VerifyNormalizedCutover(context.Background())
	if err != nil {
		t.Fatalf("VerifyNormalizedCutover() error = %v", err)
	}
	if report.BackfillStatus != "completed" || !report.BackfillCompletedAt.Equal(now) || len(report.Tables) != len(normalizedCutoverTables) || report.MissingSecretReferences != 0 {
		t.Fatalf("report = %+v", report)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestVerifyNormalizedCutoverRejectsPendingBackfill(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status, COALESCE(completed_at, TIMESTAMPTZ 'epoch')")).WillReturnRows(
		sqlmock.NewRows([]string{"status", "coalesce"}).AddRow("pending", time.Unix(0, 0).UTC()),
	)
	mock.ExpectRollback()
	if _, err := repository.VerifyNormalizedCutover(context.Background()); err == nil {
		t.Fatal("VerifyNormalizedCutover() error = nil, want pending backfill rejection")
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
