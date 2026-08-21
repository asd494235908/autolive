package store

import (
	"context"
	"encoding/json"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryRotateModelPoolAccountSecretUsesSharedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 18, 0, 0, 0, time.UTC)
	secretWriter := &transactionalSecretWriterStub{MemorySecretStore: NewMemorySecretStore()}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretWriter, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	probe := controlplane.ModelPoolConnectivityTestResult{
		AccountID: "mpa_1", Provider: "openai", Model: "gpt", Status: "succeeded", TestedAt: now.Format(time.RFC3339), HTTPStatus: 200,
	}
	payload, err := json.Marshal(probe)
	if err != nil {
		t.Fatalf("json.Marshal() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow("mpa_1", "openai", "gpt", "https://api.example.test", "model-account/mpa_1", controlplane.ModelAccountStatusActive, 2, 2, 0, nil))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "rotate-model-account-secret:mpa_1:rotate-key", "fp-1", "mpa_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "mpa_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts\n\t\tSET secret_ref = $2, updated_at = $3")).WithArgs("mpa_1", sqlmock.AnyArg(), now, "model-account/mpa_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_pool_test_results (id, account_id, payload, created_at)")).WithArgs(sqlmock.AnyArg(), "mpa_1", payload, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM model_account_secrets")).WithArgs("model-account/mpa_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases")).WithArgs("mpa_1", controlplane.ModelLeaseStatusActive, now).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	dayStart := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COALESCE(SUM(total_tokens), 0)")).WithArgs("mpa_1", dayStart, dayStart.Add(24*time.Hour)).WillReturnRows(sqlmock.NewRows([]string{"coalesce"}).AddRow(0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT payload, created_at")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"payload", "created_at"}).AddRow(payload, now))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-rotate-account", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	summary, err := repository.RotateModelPoolAccountSecret(context.Background(), ModelPoolSecretRotationRecord{
		Scope: "control-plane-state", IdempotencyKey: "rotate-model-account-secret:mpa_1:rotate-key", Fingerprint: "fp-1", AccountID: "mpa_1", ExpectedSecretRef: "model-account/mpa_1", APIKey: "sk-rotated-secret", Probe: probe,
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_local_admin", Action: "POST /api/v1/admin/model-pool/{account_id}/rotate-secret", TargetType: "model_account", TargetID: "mpa_1", Outcome: "success", StatusCode: 200, RequestID: "req-rotate-account"},
	})
	if err != nil {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v", err)
	}
	if summary.ID != "mpa_1" || !summary.SecretConfigured || summary.LastTestStatus != "succeeded" {
		t.Fatalf("rotated account = %+v", summary)
	}
	if secretWriter.value != "sk-rotated-secret" || secretWriter.reference == "" {
		t.Fatalf("transactional secret writer = ref=%q value=%q", secretWriter.reference, secretWriter.value)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
