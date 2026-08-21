package store

import (
	"context"
	"errors"
	"regexp"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryActivateDeviceWithSessionBindingCommitsTogether(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	expiresAt := now.Add(time.Hour)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductAutoLive), nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, status, expires_at, max_devices, bound_devices FROM activation_codes")).WithArgs("digest-1").WillReturnRows(sqlmock.NewRows([]string{"id", "product", "status", "expires_at", "max_devices", "bound_devices"}).AddRow("ac_1", string(controlplane.ProductAutoLive), controlplane.ActivationCodeStatusActive, expiresAt, 2, 0))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:activate-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, last_heartbeat_at)")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive, "state-device/dev_1", "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE activation_codes SET status = $2, bound_devices = $3, used_at = COALESCE(used_at, $4), used_by_user_id = COALESCE(used_by_user_id, $5), used_by_device_id = COALESCE(used_by_device_id, $6)")).WithArgs("ac_1", controlplane.ActivationCodeStatusActive, 1, now, "usr_1", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	device, err := repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:activate-key", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1", ActivationCodeHash: "digest-1",
		Product: controlplane.ProductAutoLive, Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/activate", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-1"},
	})
	if err != nil {
		t.Fatalf("ActivateDeviceWithSessionBinding() error = %v", err)
	}
	if device.ID != "dev_1" || device.UserID != "usr_1" || device.Status != controlplane.DeviceStatusActive || device.LastSeenAt != now.Format(time.RFC3339) {
		t.Fatalf("activated device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryActivateDeviceAuditFailureRollsBackBusinessTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	expiresAt := now.Add(time.Hour)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductAutoLive), nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, status, expires_at, max_devices, bound_devices FROM activation_codes")).WithArgs("digest-1").WillReturnRows(sqlmock.NewRows([]string{"id", "product", "status", "expires_at", "max_devices", "bound_devices"}).AddRow("ac_1", string(controlplane.ProductAutoLive), controlplane.ActivationCodeStatusActive, expiresAt, 1, 0))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:activate-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, last_heartbeat_at)")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive, "state-device/dev_1", "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE activation_codes SET status = $2, bound_devices = $3, used_at = COALESCE(used_at, $4), used_by_user_id = COALESCE(used_by_user_id, $5), used_by_device_id = COALESCE(used_by_device_id, $6)")).WithArgs("ac_1", controlplane.ActivationCodeStatusUsed, 1, now, "usr_1", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-fail", sqlmock.AnyArg(), now).WillReturnError(errors.New("outbox unavailable"))
	mock.ExpectRollback()

	_, err = repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:activate-key", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1", ActivationCodeHash: "digest-1",
		Product: controlplane.ProductAutoLive, Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/activate", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-fail"},
	})
	if err == nil || !strings.Contains(err.Error(), "outbox unavailable") {
		t.Fatalf("activation error = %v, want outbox failure", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryActivateRejectsSessionProductMismatchBeforeIdempotency(t *testing.T) {
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
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").
		WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductDouyinDesktop), nil))
	mock.ExpectRollback()

	_, err = repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-product-mismatch", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1", Product: controlplane.ProductAutoLive,
		ActivationCodeHash: "digest-1", Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_00000001", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("ActivateDeviceWithSessionBinding() error = %v, want forbidden", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
