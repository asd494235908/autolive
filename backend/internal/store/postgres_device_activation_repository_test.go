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
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:activate-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT activation_code_id FROM activation_device_bindings")).WithArgs("dev_1", controlplane.ProductAutoLive, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"activation_code_id"}))
	mock.ExpectQuery("(?s)"+regexp.QuoteMeta("SELECT id, status, expires_at, max_devices, bound_devices FROM activation_codes")+".*"+regexp.QuoteMeta("ORDER BY expires_at ASC, id ASC")).WithArgs("usr_1", controlplane.ProductAutoLive, controlplane.ActivationCodeStatusRevoked).WillReturnRows(sqlmock.NewRows([]string{"id", "status", "expires_at", "max_devices", "bound_devices"}).AddRow("ac_1", controlplane.ActivationCodeStatusActive, expiresAt, 2, 0))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, last_heartbeat_at)")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive, "state-device/dev_1", "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO activation_device_bindings (activation_code_id, device_id, product, user_id, bound_at)")).WithArgs("ac_1", "dev_1", controlplane.ProductAutoLive, "usr_1", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE activation_codes SET status = $2, bound_devices = $3, used_at = COALESCE(used_at, $4), used_by_user_id = COALESCE(used_by_user_id, $5), used_by_device_id = COALESCE(used_by_device_id, $6)")).WithArgs("ac_1", controlplane.ActivationCodeStatusActive, 1, now, "usr_1", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	expectNormalizedAuditTargetProduct(mock, "devices", "dev_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	device, err := repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:activate-key", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1",
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
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:activate-key", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT activation_code_id FROM activation_device_bindings")).WithArgs("dev_1", controlplane.ProductAutoLive, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"activation_code_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, status, expires_at, max_devices, bound_devices FROM activation_codes")).WithArgs("usr_1", controlplane.ProductAutoLive, controlplane.ActivationCodeStatusRevoked).WillReturnRows(sqlmock.NewRows([]string{"id", "status", "expires_at", "max_devices", "bound_devices"}).AddRow("ac_1", controlplane.ActivationCodeStatusActive, expiresAt, 1, 0))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, last_heartbeat_at)")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive, "state-device/dev_1", "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO activation_device_bindings (activation_code_id, device_id, product, user_id, bound_at)")).WithArgs("ac_1", "dev_1", controlplane.ProductAutoLive, "usr_1", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE activation_codes SET status = $2, bound_devices = $3, used_at = COALESCE(used_at, $4), used_by_user_id = COALESCE(used_by_user_id, $5), used_by_device_id = COALESCE(used_by_device_id, $6)")).WithArgs("ac_1", controlplane.ActivationCodeStatusUsed, 1, now, "usr_1", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	expectNormalizedAuditTargetProduct(mock, "devices", "dev_1", controlplane.ProductAutoLive)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-fail", sqlmock.AnyArg(), now).WillReturnError(errors.New("outbox unavailable"))
	mock.ExpectRollback()

	_, err = repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:activate-key", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1",
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
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_00000001", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("ActivateDeviceWithSessionBinding() error = %v, want forbidden", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryActivateDeviceReturnsStableExistingDeviceErrors(t *testing.T) {
	tests := []struct {
		name        string
		actorUserID string
		ownerUserID string
		status      string
		want        error
	}{
		{name: "disabled same account", actorUserID: "usr_1", ownerUserID: "usr_1", status: controlplane.DeviceStatusDisabled, want: controlplane.ErrDeviceDisabled},
		{name: "same product other account", actorUserID: "usr_2", ownerUserID: "usr_1", status: controlplane.DeviceStatusActive, want: controlplane.ErrDeviceBindingConflict},
	}
	for _, testCase := range tests {
		t.Run(testCase.name, func(t *testing.T) {
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
			mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow(testCase.actorUserID, string(controlplane.ProductAutoLive), nil))
			mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", testCase.ownerUserID, string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", testCase.status, now))
			mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "stable-error", "fp-1", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
			mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs(testCase.actorUserID).WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow(testCase.actorUserID, "actor", controlplane.RoleUser, controlplane.UserStatusActive, now))
			mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs(testCase.actorUserID, controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
			mock.ExpectRollback()
			_, err = repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
				Scope: "control-plane-state", IdempotencyKey: "stable-error", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: testCase.actorUserID, Product: controlplane.ProductAutoLive,
				Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
			})
			if !errors.Is(err, testCase.want) {
				t.Fatalf("ActivateDeviceWithSessionBinding() error = %v, want %v", err, testCase.want)
			}
			if err := mock.ExpectationsWereMet(); err != nil {
				t.Fatalf("sql expectations: %v", err)
			}
		})
	}
}

func TestPostgresRepositoryActivateDeviceReusesRevokedExistingBinding(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Old", "windows", "0.9.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "revoked-relogin", "fp-1", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT activation_code_id FROM activation_device_bindings")).WithArgs("dev_1", controlplane.ProductAutoLive, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"activation_code_id"}).AddRow("ac_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT bound_user_id, product, status, expires_at, max_devices, bound_devices FROM activation_codes")).WithArgs("ac_1").WillReturnRows(sqlmock.NewRows([]string{"bound_user_id", "product", "status", "expires_at", "max_devices", "bound_devices"}).AddRow("usr_1", string(controlplane.ProductAutoLive), controlplane.ActivationCodeStatusRevoked, expiresAt, 1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE devices SET user_id = $2, product = $3, device_name = $4")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive, "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_sessions SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP")).WithArgs("access-hash", "dev_1", controlplane.ProductAutoLive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	device, err := repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "revoked-relogin", Fingerprint: "fp-1", AccessTokenHash: "access-hash", UserID: "usr_1", Product: controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if err != nil || device.ActivationExpiresAt == nil || *device.ActivationExpiresAt != expiresAt.Format(time.RFC3339) {
		t.Fatalf("revoked existing binding activation = %+v, error = %v", device, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryActivateDeviceIdempotentReplaySkipsExpiredGrantAndCapacity(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	expiresAt := now.Add(-time.Hour)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductAutoLive), "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:replay", "fp-replay", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "activate-device:usr_1:replay").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-replay", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT ac.expires_at FROM activation_device_bindings AS binding")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"expires_at"}).AddRow(expiresAt))
	mock.ExpectCommit()

	device, err := repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:replay", Fingerprint: "fp-replay", AccessTokenHash: "access-hash", UserID: "usr_1", Product: controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDeviceWithSessionBinding() replay error = %v", err)
	}
	if device.ID != "dev_1" || device.ActivationExpiresAt == nil || *device.ActivationExpiresAt != expiresAt.Format(time.RFC3339) {
		t.Fatalf("idempotent replay device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryActivateDeviceReplayRejectsBindingNewSessionToExpiredGrant(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	expiresAt := now.Add(-time.Hour)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, product, device_id FROM auth_sessions")).WithArgs("new-access-hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "product", "device_id"}).AddRow("usr_1", string(controlplane.ProductAutoLive), nil))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status, last_heartbeat_at FROM devices")).WithArgs("dev_1").WillReturnRows(sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "last_heartbeat_at"}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Demo", "windows", "1.0.0", controlplane.DeviceStatusActive, now.Add(-time.Minute)))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "activate-device:usr_1:replay", "fp-replay", "dev_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "activate-device:usr_1:replay").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-replay", "dev_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT ac.expires_at FROM activation_device_bindings AS binding")).WithArgs("dev_1", "usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"expires_at"}).AddRow(expiresAt))
	mock.ExpectRollback()

	_, err = repository.ActivateDeviceWithSessionBinding(context.Background(), DeviceActivationRecord{
		Scope: "control-plane-state", IdempotencyKey: "activate-device:usr_1:replay", Fingerprint: "fp-replay", AccessTokenHash: "new-access-hash", UserID: "usr_1", Product: controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_1", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if !errors.Is(err, controlplane.ErrAccountActivationExpired) {
		t.Fatalf("new-session replay error = %v, want account activation expired", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
