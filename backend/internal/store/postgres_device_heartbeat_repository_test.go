package store

import (
	"context"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryRecordHeartbeatWithSessionBindingCommitsTogether(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductAutoLive), nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "heartbeat:usr_1:dev_1:heartbeat-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE devices SET disk_free_bytes = $2")).WithArgs("dev_1", int64(1024), int64(4096), int64(2048), 8, "windows", "11", "kernel", "demo.mp4", "playing", now, controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	result, err := repository.RecordHeartbeatWithSessionBinding(context.Background(), DeviceHeartbeatRecord{
		Scope: "control-plane-state", IdempotencyKey: "heartbeat:usr_1:dev_1:heartbeat-key", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1",
		Product: controlplane.ProductAutoLive, Input: controlplane.HeartbeatInput{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", SentAt: now, Status: controlplane.HeartbeatStatus{DiskFreeBytes: 1024, MemoryTotalBytes: 4096, MemoryAvailableBytes: 2048, CPULogicalCores: 8, OSName: "windows", OSVersion: "11", KernelVersion: "kernel", CurrentMediaName: "demo.mp4", PlaybackState: "playing"}},
		Audit: controlplane.AuditLogInput{ActorUserID: "usr_1", DeviceID: "dev_1", Action: "POST /api/v1/client/heartbeat", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-1"},
	})
	if err != nil {
		t.Fatalf("RecordHeartbeatWithSessionBinding() error = %v", err)
	}
	if result.AcceptedAt != now.Format(time.RFC3339) || result.DeviceStatus != controlplane.DeviceStatusActive {
		t.Fatalf("heartbeat result = %+v", result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryHeartbeatRejectsSessionProductMismatchBeforeIdempotency(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").
		WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductDouyinDesktop), nil))
	mock.ExpectRollback()

	_, err = repository.RecordHeartbeatWithSessionBinding(context.Background(), DeviceHeartbeatRecord{
		Scope: "control-plane-state", IdempotencyKey: "heartbeat-product-mismatch", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1", Product: controlplane.ProductAutoLive,
		Input: controlplane.HeartbeatInput{Product: controlplane.ProductAutoLive, DeviceID: "dev_00000001", SentAt: now, Status: controlplane.HeartbeatStatus{DiskFreeBytes: 1024}},
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordHeartbeatWithSessionBinding() error = %v, want forbidden", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
