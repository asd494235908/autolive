package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryModelPoolTestUsesTwoShortTransactions(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 17, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow("mpa_1", "openai", "gpt", "https://api.example.test", "model-account/mpa_1", controlplane.ModelAccountStatusActive, 1, 2, 100, nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "test-model-account:mpa_1:key-1").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	expectModelPoolAccountSummary(mock, now, "mpa_1")
	mock.ExpectCommit()

	preparation, err := repository.PrepareModelPoolAccountTest(context.Background(), ModelPoolTestPrepareRecord{
		Scope: "control-plane-state", IdempotencyKey: "test-model-account:mpa_1:key-1", Fingerprint: "fp-1", AccountID: "mpa_1",
	})
	if err != nil {
		t.Fatalf("PrepareModelPoolAccountTest() error = %v", err)
	}
	if preparation.Account.ID != "mpa_1" || preparation.Account.SecretRef != "model-account/mpa_1" || preparation.Cached != nil {
		t.Fatalf("preparation = %+v", preparation)
	}

	result := controlplane.ModelPoolConnectivityTestResult{AccountID: "mpa_1", Provider: "openai", Model: "gpt", Status: "succeeded", TestedAt: now.Format(time.RFC3339), LatencyMS: 12}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow("mpa_1", "openai", "gpt", "https://api.example.test", "model-account/mpa_1", controlplane.ModelAccountStatusActive, 1, 2, 100, nil))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "test-model-account:mpa_1:key-1", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "model_test_1"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_pool_test_results (id, account_id, payload, created_at)")).WithArgs(sqlmock.AnyArg(), "mpa_1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts SET status = $2, cooldown_until = NULL, updated_at = $3 WHERE id = $1")).WithArgs("mpa_1", controlplane.ModelAccountStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	stored, err := repository.RecordModelPoolAccountTest(context.Background(), ModelPoolTestRecord{
		Scope: "control-plane-state", IdempotencyKey: "test-model-account:mpa_1:key-1", Fingerprint: "fp-1", AccountID: "mpa_1", Result: result,
	})
	if err != nil {
		t.Fatalf("RecordModelPoolAccountTest() error = %v", err)
	}
	if stored != result {
		t.Fatalf("stored result = %+v, want %+v", stored, result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
