package store

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

type transactionalSecretWriterStub struct {
	*MemorySecretStore
	reference string
	value     string
}

func (s *transactionalSecretWriterStub) PutTx(_ context.Context, tx *sql.Tx, reference, value string) error {
	if tx == nil {
		return errors.New("missing transaction")
	}
	s.reference, s.value = reference, value
	return nil
}

func TestPostgresRepositoryCreateModelPoolAccountUsesSharedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 17, 0, 0, 0, time.UTC)
	secretWriter := &transactionalSecretWriterStub{MemorySecretStore: NewMemorySecretStore()}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretWriter, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-model-account:create-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "mpa_created"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, active_requests, daily_reserved_tokens, created_at, updated_at)")).WithArgs(sqlmock.AnyArg(), "openai", "gpt", "https://api.example.test", sqlmock.AnyArg(), controlplane.ModelAccountStatusActive, 2, 3, 100, now).WillReturnResult(sqlmock.NewResult(1, 1))
	expectModelPoolAccountSummary(mock, now, sqlmock.AnyArg())
	expectNormalizedAuditTargetProduct(mock, "model_accounts", sqlmock.AnyArg(), controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-create-account", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	summary, err := repository.CreateModelPoolAccount(context.Background(), ModelPoolAccountCreateRecord{
		Scope: "control-plane-state", IdempotencyKey: "create-model-account:create-key", Fingerprint: "fp-1",
		Provider: "openai", Model: "gpt", BaseURL: "https://api.example.test", APIKey: "sk-test-secret",
		Status: controlplane.ModelAccountStatusActive, Priority: 2, DailyLimit: 100, ConcurrencyLimit: 3,
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_local_admin", Action: "POST /api/v1/admin/model-pool", TargetType: "model_account", Outcome: "success", StatusCode: 201, RequestID: "req-create-account"},
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	if summary.Provider != "openai" || summary.Model != "gpt" || !summary.SecretConfigured {
		t.Fatalf("created account = %+v", summary)
	}
	if secretWriter.value != "sk-test-secret" || !strings.HasPrefix(secretWriter.reference, "model-account/") {
		t.Fatalf("secret writer = ref=%q value=%q", secretWriter.reference, secretWriter.value)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateModelPoolAccountRequiresTransactionalSecretStore(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 17, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, NewMemorySecretStore(), ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-model-account:create-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "mpa_created"))

	_, err = repository.CreateModelPoolAccount(context.Background(), ModelPoolAccountCreateRecord{
		Scope: "control-plane-state", IdempotencyKey: "create-model-account:create-key", Fingerprint: "fp-1",
		Provider: "openai", Model: "gpt", APIKey: "sk-test-secret", Status: controlplane.ModelAccountStatusActive, ConcurrencyLimit: 1,
	})
	if !errors.Is(err, ErrTransactionalSecretStoreRequired) {
		t.Fatalf("CreateModelPoolAccount() error = %v, want transactional secret store error", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryDisableModelPoolAccountUsesNormalizedTransaction(t *testing.T) {
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
	expectModelPoolAccountMutationPrefix(mock, now, "mpa_1", controlplane.ModelAccountStatusActive)
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts SET status = $2, updated_at = $3 WHERE id = $1")).WithArgs("mpa_1", controlplane.ModelAccountStatusDisabled, now).WillReturnResult(sqlmock.NewResult(1, 1))
	expectModelPoolAccountSummary(mock, now, "mpa_1")
	expectNormalizedAuditTargetProduct(mock, "model_accounts", "mpa_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-disable-account", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	summary, err := repository.DisableModelPoolAccount(context.Background(), ModelPoolAccountMutationRecord{Scope: "control-plane-state", IdempotencyKey: "disable-model-account:disable-key", Fingerprint: "fp-1", AccountID: "mpa_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_local_admin", Action: "POST /api/v1/admin/model-pool/{account_id}/disable", TargetType: "model_account", Outcome: "success", StatusCode: 200, RequestID: "req-disable-account"}})
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() error = %v", err)
	}
	if summary.ID != "mpa_1" || summary.Status != controlplane.ModelAccountStatusDisabled || summary.SecretConfigured != true {
		t.Fatalf("disabled account = %+v", summary)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryUpdateModelPoolAccountUsesNormalizedTransaction(t *testing.T) {
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
	expectModelPoolAccountMutationPrefix(mock, now, "mpa_1", controlplane.ModelAccountStatusActive)
	newBaseURL := "https://new.example.test"
	newPriority := 3
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts SET base_url = $2, priority = $3, updated_at = $4 WHERE id = $1")).WithArgs("mpa_1", newBaseURL, newPriority, now).WillReturnResult(sqlmock.NewResult(1, 1))
	expectModelPoolAccountSummary(mock, now, "mpa_1")
	expectNormalizedAuditTargetProduct(mock, "model_accounts", "mpa_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-update-account", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	summary, err := repository.UpdateModelPoolAccount(context.Background(), ModelPoolAccountUpdateRecord{
		ModelPoolAccountMutationRecord: ModelPoolAccountMutationRecord{Scope: "control-plane-state", IdempotencyKey: "update-model-account:update-key", Fingerprint: "fp-1", AccountID: "mpa_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_local_admin", Action: "PATCH /api/v1/admin/model-pool/{account_id}", TargetType: "model_account", Outcome: "success", StatusCode: 200, RequestID: "req-update-account"}},
		Input:                          controlplane.UpdateModelPoolAccountInput{BaseURL: &newBaseURL, Priority: &newPriority},
	})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() error = %v", err)
	}
	if summary.ID != "mpa_1" || summary.BaseURL != newBaseURL || summary.Priority != newPriority {
		t.Fatalf("updated account = %+v", summary)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func expectModelPoolAccountMutationPrefix(mock sqlmock.Sqlmock, now time.Time, accountID, status string) {
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs(accountID).WillReturnRows(sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow(accountID, "openai", "gpt", "https://api.example.test", "model-account/mpa_1", status, 1, 2, 0, nil))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", sqlmock.AnyArg(), "fp-1", accountID, now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", accountID))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = COALESCE(released_at, $3)")).WithArgs(accountID, controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases")).WithArgs(accountID, controlplane.ModelLeaseStatusActive, now).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
}

func expectModelPoolAccountSummary(mock sqlmock.Sqlmock, now time.Time, accountID any) {
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases")).WithArgs(accountID, controlplane.ModelLeaseStatusActive, now).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COALESCE(SUM(total_tokens), 0)")).WithArgs(accountID, dayStart, dayStart.Add(24*time.Hour)).WillReturnRows(sqlmock.NewRows([]string{"coalesce"}).AddRow(0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT payload, created_at")).WithArgs(accountID).WillReturnError(sql.ErrNoRows)
}
