package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryPreparesNormalizedModelSecretRotationWithoutSnapshot(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("completed"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{
		"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until",
	}).AddRow("mpa_1", "openai-compatible", "rewrite-model", "https://api.example.test/v1", "model-account/mpa_1", controlplane.ModelAccountStatusActive, 1, 1, 0, nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases")).WithArgs("mpa_1", controlplane.ModelLeaseStatusActive, now).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	dayStart := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COALESCE(SUM(total_tokens), 0)")).WithArgs("mpa_1", dayStart, dayStart.Add(24*time.Hour)).WillReturnRows(sqlmock.NewRows([]string{"coalesce"}).AddRow(0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT payload, created_at")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"payload", "created_at"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "rotate-model-account-secret:mpa_1:rotate-key").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectCommit()

	preparation, err := repository.PrepareModelPoolAccountSecretRotation(context.Background(), "control-plane-state", "rotate-model-account-secret:mpa_1:rotate-key", "fp-1", "mpa_1")
	if err != nil {
		t.Fatalf("PrepareModelPoolAccountSecretRotation() error = %v", err)
	}
	if preparation.Account.ID != "mpa_1" || preparation.Account.SecretRef != "model-account/mpa_1" || preparation.Existing != nil {
		t.Fatalf("preparation = %+v", preparation)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
