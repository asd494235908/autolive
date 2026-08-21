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

func TestTask4StrictLeaseAndUsageLookupsBindProductInSQL(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)

	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("WHERE id = $1 AND product = $2")).
		WithArgs("lease_douyin", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "product", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit",
		}).AddRow("lease_douyin", string(controlplane.ProductDouyinDesktop), "account_douyin", "user_1", "device_douyin", "chat", controlplane.ModelLeaseStatusActive, now.Add(time.Hour), now, nil, "openai", "gpt", controlplane.ModelLeaseProxyModeDirectLease, 1))
	mock.ExpectCommit()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	lease, err := repository.loadModelLeaseForUpdateWithProduct(context.Background(), tx, "lease_douyin", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("loadModelLeaseForUpdateWithProduct() error = %v", err)
	}
	if lease.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("lease product = %q", lease.Product)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("lease tx commit error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("WHERE id = $1 AND product = $2")).
		WithArgs("usage_douyin", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "product", "lease_id", "client_call_id", "request_id", "provider", "model", "prompt_tokens", "completion_tokens", "total_tokens", "latency_ms", "status", "usage_source", "error_code", "created_at",
		}).AddRow("usage_douyin", string(controlplane.ProductDouyinDesktop), "lease_douyin", "call_1", "request_1", "openai", "gpt", 1, 2, 3, int64(5), "succeeded", "client_reported", nil, now))
	mock.ExpectCommit()
	tx, err = database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	usage, err := repository.loadModelUsageByIDWithProduct(context.Background(), tx, "usage_douyin", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("loadModelUsageByIDWithProduct() error = %v", err)
	}
	if usage.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("usage product = %q", usage.Product)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("usage tx commit error = %v", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestTask4StrictModelAccountLookupBindsProductInSQL(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("WHERE id = $1 AND product = $2")).
		WithArgs("account_douyin", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "product", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until",
		}).AddRow("account_douyin", string(controlplane.ProductDouyinDesktop), "openai", "gpt", "https://example.test", "model-account/douyin", controlplane.ModelAccountStatusActive, 1, 1, 0, nil))
	mock.ExpectCommit()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	account, err := repository.loadNormalizedModelPoolAccountWithProduct(context.Background(), tx, "account_douyin", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("loadNormalizedModelPoolAccountWithProduct() error = %v", err)
	}
	if account.product != controlplane.ProductDouyinDesktop {
		t.Fatalf("account product = %q", account.product)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("tx commit error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestTask4DeviceLifecycleScopesLeaseReleaseAndSessionRevokeByProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("WHERE id = $1 AND product = $2")).
		WithArgs("device_douyin", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).
			AddRow("device_douyin", "user_1", string(controlplane.ProductDouyinDesktop), "Desktop", "windows", "1.0", controlplane.DeviceStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at, product)")).
		WithArgs("control-plane-state", "disable-device", "fp", "device_douyin", now, controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id", "product"}).AddRow("fp", "device_douyin", string(controlplane.ProductDouyinDesktop)))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases\n\t\tSET status = $2, released_at = $3")).
		WithArgs("device_douyin", controlplane.ModelLeaseStatusReleased, now, controlplane.ModelLeaseStatusActive, controlplane.ProductDouyinDesktop).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions\n\t\tSET revoked_at = CURRENT_TIMESTAMP")).
		WithArgs("device_douyin", controlplane.ProductDouyinDesktop).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE devices SET status = $2 WHERE id = $1 AND product = $3")).
		WithArgs("device_douyin", controlplane.DeviceStatusDisabled, controlplane.ProductDouyinDesktop).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	device, err := repository.DisableDevice(context.Background(), DeviceMutationRecord{
		Scope: "control-plane-state", IdempotencyKey: "disable-device", Fingerprint: "fp", DeviceID: "device_douyin", Product: controlplane.ProductDouyinDesktop,
	})
	if err != nil {
		t.Fatalf("DisableDevice() error = %v", err)
	}
	if device.Product != controlplane.ProductDouyinDesktop || device.Status != controlplane.DeviceStatusDisabled {
		t.Fatalf("device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestTask4LeaseSweepScopesProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	_, err = NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	lease := controlplane.ModelLease{ID: "lease_douyin", Product: controlplane.ProductDouyinDesktop, Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(-time.Minute).Format(time.RFC3339)}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = COALESCE(released_at, $3) WHERE id = $1 AND status = $4")).
		WithArgs("lease_douyin", controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive, controlplane.ProductDouyinDesktop).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	if err := sweepNormalizedModelLeaseForProduct(context.Background(), tx, &lease, now, controlplane.ProductDouyinDesktop); err != nil {
		t.Fatalf("sweepNormalizedModelLeaseForProduct() error = %v", err)
	}
	if lease.Status != controlplane.ModelLeaseStatusExpired {
		t.Fatalf("lease status = %q", lease.Status)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("tx commit error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestTask4ProductIdempotencyDoesNotReadOtherProductRow(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at, product)")).
		WithArgs("scope", "key", "fp", "resource", sqlmock.AnyArg(), controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id", "product"}))
	mock.ExpectQuery(regexp.QuoteMeta("WHERE scope = $1 AND idempotency_key = $2 AND product = $3")).
		WithArgs("scope", "key", controlplane.ProductDouyinDesktop).
		WillReturnError(sql.ErrNoRows)
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	_, _, _, err = repository.reserveUserIdempotencyForProduct(context.Background(), tx, "scope", "key", "fp", "resource", nowForProductContract(), controlplane.ProductDouyinDesktop)
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("reserveUserIdempotencyForProduct() error = %v, want forbidden", err)
	}
	_ = tx.Rollback()
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func nowForProductContract() time.Time {
	return time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
}
