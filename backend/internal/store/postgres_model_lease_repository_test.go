package store

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryCreateModelLeaseUsesNormalizedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	secretStore := NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/mpa_1", "secret-value"); err != nil {
		t.Fatalf("secretStore.Put() error = %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	dayStart := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-model-lease:usr_1:dev_1:create-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "lease_created"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases")).WithArgs(controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts\n\t\tSET status = $4")).WithArgs("openai", "gpt", now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts a")).WithArgs("openai", "gpt", now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusExhausted, dayStart, dayEnd).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT allowed_models, daily_token_limit")).WithArgs("usr_1").WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT a.id\n\t\tFROM model_accounts a")).WithArgs("openai", "gpt", controlplane.ModelAccountStatusActive, now, dayStart, dayEnd).WillReturnRows(sqlmock.NewRows([]string{"id"}).AddRow("mpa_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority,")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow("mpa_1", "openai", "gpt", "https://api.example.test", "model-account/mpa_1", controlplane.ModelAccountStatusActive, 2, 2, 0, nil))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, released_at, provider, model, proxy_mode, concurrency_limit)")).WithArgs(sqlmock.AnyArg(), "mpa_1", "usr_1", "dev_1", "chat", controlplane.ModelLeaseStatusActive, now.Add(5*time.Minute), now, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-create-lease", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	lease, err := repository.CreateModelLease(context.Background(), ModelLeaseCreateRecord{
		Scope: "control-plane-state", IdempotencyKey: "create-model-lease:usr_1:dev_1:create-key", Fingerprint: "fp-1",
		UserID: "usr_1", DeviceID: "dev_1", Provider: "openai", Model: "gpt", Purpose: "chat", MaxDurationSeconds: 300,
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/model-leases", TargetType: "model_lease", Outcome: "success", StatusCode: 200, RequestID: "req-create-lease"},
	})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	if lease.AccountID != "mpa_1" || lease.Provider != "openai" || lease.Model != "gpt" || lease.Status != controlplane.ModelLeaseStatusActive || lease.ExpiresAt != now.Add(5*time.Minute).Format(time.RFC3339) {
		t.Fatalf("created lease = %+v", lease)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateModelLeaseHonorsCancellation(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, NewMemorySecretStore(), ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err = repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: "control-plane-state", IdempotencyKey: "create-key", Fingerprint: "fp-1",
		UserID: "usr_1", DeviceID: "dev_1", Provider: "openai", Model: "gpt", Purpose: "chat", MaxDurationSeconds: 300,
	})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("CreateModelLease() error = %v, want context.Canceled", err)
	}
}

func TestPostgresRepositoryCreateModelLeaseEnforcesNormalizedModelAllowlist(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	dayStart := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, NewMemorySecretStore(), ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "lease_created"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases")).WithArgs(controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts\n\t\tSET status = $4")).WithArgs("openai", "gpt", now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_accounts a")).WithArgs("openai", "gpt", now, controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusExhausted, dayStart, dayStart.Add(24*time.Hour)).WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT allowed_models, daily_token_limit")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"allowed_models", "daily_token_limit"}).AddRow([]byte(`["other/gpt"]`), 0))
	mock.ExpectRollback()

	_, err = repository.CreateModelLease(context.Background(), ModelLeaseCreateRecord{
		Scope: "control-plane-state", IdempotencyKey: "create-key", Fingerprint: "fp-1",
		UserID: "usr_1", DeviceID: "dev_1", Provider: "openai", Model: "gpt", Purpose: "chat", MaxDurationSeconds: 300,
	})
	if !controlplane.IsErrorCode(err, controlplane.ErrUserModelNotAuthorized.Code) {
		t.Fatalf("CreateModelLease() error = %v, want USER_MODEL_NOT_AUTHORIZED", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRenewModelLeaseUsesNormalizedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at,")).WithArgs("lease_1").WillReturnRows(sqlmock.NewRows([]string{"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit"}).AddRow("lease_1", "mpa_1", "usr_1", "dev_1", "chat", controlplane.ModelLeaseStatusActive, now.Add(5*time.Minute), now.Add(-time.Minute), nil, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "renew-model-lease:lease_1:renew-key", "fp-1", "lease_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "lease_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT provider, model, base_url, status, concurrency_limit, daily_token_limit")).WithArgs("mpa_1").WillReturnRows(sqlmock.NewRows([]string{"provider", "model", "base_url", "status", "concurrency_limit", "daily_token_limit"}).AddRow("openai", "gpt", "https://api.example.test", controlplane.ModelAccountStatusActive, 2, 0))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET expires_at = $2, provider = $3, model = $4, proxy_mode = $5, concurrency_limit = $6")).WithArgs("lease_1", now.Add(10*time.Minute), "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-renew-lease", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	lease, err := repository.RenewModelLease(context.Background(), ModelLeaseRenewRecord{Scope: "control-plane-state", IdempotencyKey: "renew-model-lease:lease_1:renew-key", Fingerprint: "fp-1", UserID: "usr_1", DeviceID: "dev_1", LeaseID: "lease_1", ExtendSeconds: 300, Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/model-leases/lease_1/renew", TargetType: "model_lease", TargetID: "lease_1", Outcome: "success", StatusCode: 200, RequestID: "req-renew-lease"}})
	if err != nil {
		t.Fatalf("RenewModelLease() error = %v", err)
	}
	if lease.ID != "lease_1" || lease.ExpiresAt != now.Add(10*time.Minute).Format(time.RFC3339) || lease.DirectBaseURL != "https://api.example.test" {
		t.Fatalf("renewed lease = %+v", lease)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryReleaseModelLeaseUsesNormalizedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at,")).WithArgs("lease_1").WillReturnRows(sqlmock.NewRows([]string{"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit"}).AddRow("lease_1", "mpa_1", "usr_1", "dev_1", "chat", controlplane.ModelLeaseStatusActive, now.Add(time.Hour), now.Add(-time.Minute), nil, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "release-model-lease:lease_1:release-key", "fp-1", "lease_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "lease_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = $3 WHERE id = $1")).WithArgs("lease_1", controlplane.ModelLeaseStatusReleased, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-release-lease", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	result, err := repository.ReleaseModelLease(context.Background(), ModelLeaseReleaseRecord{Scope: "control-plane-state", IdempotencyKey: "release-model-lease:lease_1:release-key", Fingerprint: "fp-1", UserID: "usr_1", DeviceID: "dev_1", LeaseID: "lease_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/model-leases/lease_1/release", TargetType: "model_lease", TargetID: "lease_1", Outcome: "success", StatusCode: 200, RequestID: "req-release-lease"}})
	if err != nil {
		t.Fatalf("ReleaseModelLease() error = %v", err)
	}
	if result.LeaseID != "lease_1" || !result.Released {
		t.Fatalf("release result = %+v", result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryReclaimModelLeaseUsesNormalizedTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at,")).WithArgs("lease_1").WillReturnRows(sqlmock.NewRows([]string{"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit"}).AddRow("lease_1", "mpa_1", "usr_1", "dev_1", "chat", controlplane.ModelLeaseStatusActive, now.Add(time.Hour), now.Add(-time.Minute), nil, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "admin-reclaim-model-lease:lease_1:reclaim-key", "fp-1", "lease_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "lease_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = $3 WHERE id = $1")).WithArgs("lease_1", controlplane.ModelLeaseStatusReleased, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-reclaim-lease", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	result, err := repository.ReclaimModelLease(context.Background(), ModelLeaseReclaimRecord{Scope: "control-plane-state", IdempotencyKey: "admin-reclaim-model-lease:lease_1:reclaim-key", Fingerprint: "fp-1", LeaseID: "lease_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_admin", Action: "POST /api/v1/admin/model-leases/lease_1/reclaim", TargetType: "model_lease", TargetID: "lease_1", Outcome: "success", StatusCode: 200, RequestID: "req-reclaim-lease"}})
	if err != nil {
		t.Fatalf("ReclaimModelLease() error = %v", err)
	}
	if result.LeaseID != "lease_1" || !result.Released {
		t.Fatalf("reclaim result = %+v", result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRenewModelLeaseHonorsCancellation(t *testing.T) {
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
	_, err = repository.RenewModelLease(ctx, ModelLeaseRenewRecord{
		Scope: "control-plane-state", IdempotencyKey: "renew-key", Fingerprint: "fp-1",
		UserID: "usr_1", DeviceID: "dev_1", LeaseID: "lease_1", ExtendSeconds: 300,
	})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("RenewModelLease() error = %v, want context.Canceled", err)
	}
}
