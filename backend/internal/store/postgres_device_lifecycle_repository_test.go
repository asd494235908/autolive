package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryDisableDeviceCommitsLifecycle(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 14, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "disable-device:dev_1:disable-key", "fp-1", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = $3")).WithArgs("dev_1", controlplane.ModelLeaseStatusReleased, now, controlplane.ModelLeaseStatusActive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP")).WithArgs("dev_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE devices SET status = $2 WHERE id = $1")).WithArgs("dev_1", controlplane.DeviceStatusDisabled).WillReturnResult(sqlmock.NewResult(1, 1))
	expectNormalizedAuditTargetProduct(mock, "devices", "dev_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-disable", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	device, err := repository.DisableDevice(context.Background(), DeviceMutationRecord{Scope: "control-plane-state", IdempotencyKey: "disable-device:dev_1:disable-key", Fingerprint: "fp-1", DeviceID: "dev_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_admin", DeviceID: "dev_1", Action: "POST /api/v1/admin/devices/dev_1/disable", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-disable"}})
	if err != nil {
		t.Fatalf("DisableDevice() error = %v", err)
	}
	if device.ID != "dev_1" || device.Status != controlplane.DeviceStatusDisabled || device.UserID != "usr_1" {
		t.Fatalf("disabled device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryUnbindDeviceCommitsLifecycle(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 14, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "unbind-device:dev_1:unbind-key", "fp-1", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE model_leases SET status = $2, released_at = $3")).WithArgs("dev_1", controlplane.ModelLeaseStatusReleased, now, controlplane.ModelLeaseStatusActive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET revoked_at = CURRENT_TIMESTAMP")).WithArgs("dev_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE devices SET user_id = NULL, status = $2 WHERE id = $1")).WithArgs("dev_1", controlplane.DeviceStatusPendingActivation).WillReturnResult(sqlmock.NewResult(1, 1))
	expectNormalizedAuditTargetProduct(mock, "devices", "dev_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-unbind", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	device, err := repository.UnbindDevice(context.Background(), DeviceMutationRecord{Scope: "control-plane-state", IdempotencyKey: "unbind-device:dev_1:unbind-key", Fingerprint: "fp-1", DeviceID: "dev_1", Audit: controlplane.AuditLogInput{ActorUserID: "usr_admin", DeviceID: "dev_1", Action: "POST /api/v1/admin/devices/dev_1/unbind", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-unbind"}})
	if err != nil {
		t.Fatalf("UnbindDevice() error = %v", err)
	}
	if device.ID != "dev_1" || device.Status != controlplane.DeviceStatusPendingActivation || device.UserID != "" {
		t.Fatalf("unbound device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
