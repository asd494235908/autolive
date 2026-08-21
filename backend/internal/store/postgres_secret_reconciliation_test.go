package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresNormalizedStagedSecretCleanupUsesActiveReferenceInBoundedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	cutoff := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()")).
		WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectExec(regexp.QuoteMeta(cleanupUnreferencedStagedSecretsSQL)).
		WithArgs(cutoff, 25).
		WillReturnResult(sqlmock.NewResult(0, 2))
	mock.ExpectCommit()

	deleted, err := repository.CleanupUnreferencedStagedSecrets(context.Background(), RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 25})
	if err != nil {
		t.Fatalf("CleanupUnreferencedStagedSecrets() error = %v", err)
	}
	if deleted != 2 {
		t.Fatalf("deleted = %d, want 2", deleted)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresNormalizedStagedSecretCleanupRejectsSnapshotSource(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("new repository: %v", err)
	}
	deleted, err := repository.CleanupUnreferencedStagedSecrets(context.Background(), RetentionCleanupRequest{Cutoff: time.Now().UTC(), BatchSize: 1})
	if deleted != 0 || err != ErrNormalizedRetentionCleanupRequired {
		t.Fatalf("snapshot cleanup = (%d, %v), want normalized-source error", deleted, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("unexpected SQL: %v", err)
	}
}
