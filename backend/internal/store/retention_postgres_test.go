package store

import (
	"context"
	"errors"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRetentionCleanupUsesBoundedParameterizedDeletes(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	cutoff := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	request := RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 17}
	tests := []struct {
		name  string
		run   func(context.Context, RetentionCleanupRequest) (int64, error)
		query string
	}{
		{name: "idempotency", query: cleanupIdempotencyRecordsSQL, run: repository.CleanupIdempotencyRecords},
		{name: "model tests", query: cleanupModelPoolTestResultsSQL, run: repository.CleanupModelPoolTestResults},
		{name: "audit logs", query: cleanupAuditLogsSQL, run: repository.CleanupAuditLogs},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			mock.ExpectBegin()
			mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(0, 1))
			mock.ExpectQuery(regexp.QuoteMeta(test.query)).
				WithArgs(cutoff, request.BatchSize).
				WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow("old"))
			mock.ExpectCommit()
			deleted, err := test.run(context.Background(), request)
			if err != nil {
				t.Fatalf("cleanup error = %v", err)
			}
			if deleted != 1 {
				t.Fatalf("deleted = %d, want 1", deleted)
			}
		})
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreRetentionCleanupUsesExpiryAndRevocationCutoff(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("new session store: %v", err)
	}
	cutoff := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	mock.ExpectExec(regexp.QuoteMeta(cleanupAuthSessionsSQL)).
		WithArgs(cutoff, 23).
		WillReturnResult(sqlmock.NewResult(0, 4))
	deleted, err := sessions.CleanupAuthSessions(context.Background(), RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 23})
	if err != nil {
		t.Fatalf("CleanupAuthSessions() error = %v", err)
	}
	if deleted != 4 {
		t.Fatalf("deleted = %d, want 4", deleted)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestSQLSessionStoreOrphanBindingCleanupUsesBoundedParameterizedUpdate(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	sessions, err := NewSQLSessionStore(database, time.Now)
	if err != nil {
		t.Fatalf("new session store: %v", err)
	}
	cutoff := time.Date(2026, 8, 21, 11, 55, 0, 0, time.UTC)
	mock.ExpectExec(regexp.QuoteMeta(cleanupOrphanedDeviceBindingsSQL)).
		WithArgs(cutoff, 11).
		WillReturnResult(sqlmock.NewResult(0, 2))
	deleted, err := sessions.CleanupOrphanedDeviceBindings(context.Background(), RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 11})
	if err != nil {
		t.Fatalf("CleanupOrphanedDeviceBindings() error = %v", err)
	}
	if deleted != 2 {
		t.Fatalf("deleted = %d, want 2", deleted)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRetentionCleanupRequiresNormalizedSource(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	deleted, err := repository.CleanupAuditLogs(context.Background(), RetentionCleanupRequest{Cutoff: time.Now(), BatchSize: 10})
	if !errors.Is(err, ErrNormalizedRetentionCleanupRequired) || deleted != 0 {
		t.Fatalf("CleanupAuditLogs() = (%d, %v), want normalized-source error", deleted, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("unexpected SQL: %v", err)
	}
}

func TestPostgresRetentionCleanupRejectsIncompleteNormalizedBackfill(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	request := RetentionCleanupRequest{Cutoff: time.Now().UTC(), BatchSize: 10}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()")).
		WillReturnError(errors.New("normalized backfill is not complete (status \"pending\")"))
	mock.ExpectRollback()
	if deleted, err := repository.CleanupAuditLogs(context.Background(), request); err == nil || deleted != 0 || !strings.Contains(err.Error(), "backfill") {
		t.Fatalf("incomplete-backfill cleanup = (%d, %v), want gate error", deleted, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRetentionCleanupPropagatesCancellationAndSQLError(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	request := RetentionCleanupRequest{Cutoff: time.Now().UTC(), BatchSize: 10}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if deleted, err := repository.CleanupAuditLogs(ctx, request); !errors.Is(err, context.Canceled) || deleted != 0 {
		t.Fatalf("canceled cleanup = (%d, %v)", deleted, err)
	}

	databaseErr := errors.New("database unavailable")
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta(cleanupAuditLogsSQL)).
		WithArgs(request.Cutoff, request.BatchSize).
		WillReturnError(databaseErr)
	mock.ExpectRollback()
	if deleted, err := repository.CleanupAuditLogs(context.Background(), request); !errors.Is(err, databaseErr) || deleted != 0 {
		t.Fatalf("failed cleanup = (%d, %v), want database error", deleted, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
