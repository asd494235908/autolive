package store

import (
	"context"
	"database/sql"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryRecordDirectLLMCallUsesNormalizedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 16, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at,")).WithArgs("lease_1").WillReturnRows(sqlmock.NewRows([]string{"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit"}).AddRow("lease_1", "mpa_1", "usr_1", "dev_1", "chat", controlplane.ModelLeaseStatusActive, now.Add(time.Hour), now.Add(-time.Minute), nil, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "record-direct-llm-call:usr_1:dev_1:usage-key", "fp-1", "", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", ""))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, lease_id, client_call_id, request_id, provider, model,")).WithArgs("lease_1", "call_0001").WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT daily_token_limit")).WithArgs("usr_1").WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT provider, model, base_url, status, concurrency_limit, daily_token_limit")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"provider", "model", "base_url", "status", "concurrency_limit", "daily_token_limit"}).AddRow("openai", "gpt", "https://api.example.test", controlplane.ModelAccountStatusActive, 2, 0))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_usage_records (")).WithArgs(sqlmock.AnyArg(), "mpa_1", "lease_1", "usr_1", "dev_1", "openai", "gpt", 2, 3, 5, int64(120), "req-1", "call_0001", "client_reported", "succeeded", "", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE idempotency_records SET resource_id = $3")).WithArgs("control-plane-state", "record-direct-llm-call:usr_1:dev_1:usage-key", sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), "audit-request:req-1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	usage, err := repository.RecordDirectLLMCall(context.Background(), ModelUsageWriteRecord{
		Scope: "control-plane-state", IdempotencyKey: "record-direct-llm-call:usr_1:dev_1:usage-key", Fingerprint: "fp-1", UserID: "usr_1", DeviceID: "dev_1", RequestID: "req-1",
		Input: controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_0001", LeaseID: "lease_1", Provider: "openai", Model: "gpt", InputTokens: 2, OutputTokens: 3, TotalTokens: 5, LatencyMS: 120, Status: "succeeded", UsageSource: "client_reported"},
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/llm/call-records", TargetType: "model_usage", Outcome: "success", StatusCode: 200, RequestID: "req-1"},
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() error = %v", err)
	}
	if usage.ID == "" || usage.LeaseID != "lease_1" || usage.TotalTokens != 5 || usage.CreatedAt != now.Format(time.RFC3339) {
		t.Fatalf("usage = %+v", usage)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRecordDirectLLMCallHonorsCancellation(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err = repository.RecordDirectLLMCall(ctx, ModelUsageWriteRecord{
		Scope: "control-plane-state", IdempotencyKey: "usage-key", Fingerprint: "fp-1", UserID: "usr_1", DeviceID: "dev_1",
		Input: controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_0001", LeaseID: "lease_1", Provider: "openai", Model: "gpt", Status: "succeeded", UsageSource: "client_reported"},
	})
	if err != context.Canceled {
		t.Fatalf("RecordDirectLLMCall() error = %v, want context.Canceled", err)
	}
}
