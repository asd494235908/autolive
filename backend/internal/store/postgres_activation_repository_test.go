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

func TestPostgresRepositoryCreateActivationCodeWritesHashOnly(t *testing.T) {
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
	plainCode := "code_0123456789abcdef"
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-activation-code:key-1", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "ac_created"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO activation_codes (id, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices)")).WithArgs(sqlmock.AnyArg(), "usr_1", "digest-1", "code_012345", controlplane.ActivationCodeStatusActive, now, expiresAt, 3, 0).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	code, err := repository.CreateActivationCode(context.Background(), "control-plane-state", "create-activation-code:key-1", "fp-1", ActivationCodeCreateRecord{
		UserID: "usr_1", PlainCode: plainCode, CodeHash: "digest-1", CodePrefix: "code_012345", ExpiresAt: expiresAt, MaxDevices: 3, CreatedAt: now,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	if code.PlainCode == nil || *code.PlainCode != plainCode || code.ID == "" || code.Status != controlplane.ActivationCodeStatusActive || code.CodePrefix != "code_012345" || code.MaxDevices != 3 || code.BoundDevices != 0 {
		t.Fatalf("created activation code = %+v", code)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateActivationCodeClassifiesUnknownCommit(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-activation-code:unknown-commit", "fp-unknown", sqlmock.AnyArg(), now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-unknown", "ac_unknown"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM user_products")).WithArgs("usr_1", controlplane.ProductAutoLive).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("active"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO activation_codes (id, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices)")).WithArgs(sqlmock.AnyArg(), "usr_1", "digest-unknown", "code_012345", controlplane.ActivationCodeStatusActive, now, expiresAt, 1, 0).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit().WillReturnError(errors.New("connection lost after COMMIT"))

	_, err = repository.CreateActivationCode(context.Background(), "control-plane-state", "create-activation-code:unknown-commit", "fp-unknown", ActivationCodeCreateRecord{
		UserID: "usr_1", PlainCode: "code_0123456789abcdef", CodeHash: "digest-unknown", CodePrefix: "code_012345", ExpiresAt: expiresAt, MaxDevices: 1, CreatedAt: now,
	})
	if !errors.Is(err, ErrCommitOutcomeUnknown) {
		t.Fatalf("CreateActivationCode() error = %v, want ErrCommitOutcomeUnknown", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateActivationCodeReplaysWithoutPlaintext(t *testing.T) {
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
	record := ActivationCodeCreateRecord{UserID: "usr_1", PlainCode: "code_unused", CodeHash: "digest-unused", CodePrefix: "code_unused", ExpiresAt: expiresAt, MaxDevices: 1, CreatedAt: now}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-activation-code:key-2", "fp-2", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "create-activation-code:key-2").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-2", "ac_existing"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, bound_user_id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices FROM activation_codes")).WithArgs("ac_existing").WillReturnRows(
		sqlmock.NewRows([]string{"id", "bound_user_id", "code_prefix", "status", "expires_at", "used_at", "used_by_user_id", "used_by_device_id", "max_devices", "bound_devices"}).AddRow("ac_existing", "usr_1", "code_012345", controlplane.ActivationCodeStatusActive, expiresAt, nil, nil, nil, 1, 0),
	)
	mock.ExpectRollback()
	code, err := repository.CreateActivationCode(context.Background(), "control-plane-state", "create-activation-code:key-2", "fp-2", record)
	if err != nil || code.ID != "ac_existing" || code.PlainCode != nil {
		t.Fatalf("idempotent activation replay = %+v, error %v", code, err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-activation-code:key-3", "fp-3", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "create-activation-code:key-3").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("different", "ac_existing"))
	mock.ExpectRollback()
	if _, err := repository.CreateActivationCode(context.Background(), "control-plane-state", "create-activation-code:key-3", "fp-3", record); !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("idempotency conflict error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRevokeUsedActivationCodeWritesNormalizedDomain(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, bound_user_id, code_prefix, status, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices FROM activation_codes")).WithArgs("ac_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "bound_user_id", "code_prefix", "status", "expires_at", "used_at", "used_by_user_id", "used_by_device_id", "max_devices", "bound_devices"}).AddRow("ac_1", "usr_1", "code_012345", controlplane.ActivationCodeStatusUsed, expiresAt, now, "usr_1", "dev_1", 1, 1),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "revoke-activation-code:key-6", "fp-6", "ac_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-6", "ac_1"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE activation_codes SET status = $2 WHERE id = $1")).WithArgs("ac_1", controlplane.ActivationCodeStatusRevoked).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	code, err := repository.RevokeActivationCode(context.Background(), "control-plane-state", "revoke-activation-code:key-6", "fp-6", "ac_1")
	if err != nil || code.Status != controlplane.ActivationCodeStatusRevoked || code.PlainCode != nil {
		t.Fatalf("RevokeActivationCode() = %+v, error %v", code, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryProductActivationMethodsRejectInvalidProductBeforeDatabase(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	_, err = repository.CreateActivationCodeForProduct(context.Background(), "scope", "key", "fp", ActivationCodeCreateRecord{
		PlainCode: "code_0123456789", CodeHash: "digest", CodePrefix: "code_012345", ExpiresAt: time.Now().Add(time.Hour), MaxDevices: 1,
	}, controlplane.ProductCode("invalid"))
	if !errors.Is(err, controlplane.ErrInvalidRequest) {
		t.Fatalf("CreateActivationCodeForProduct() error = %v, want invalid request", err)
	}
	_, err = repository.RevokeActivationCodeForProduct(context.Background(), "scope", "key", "fp", "ac_1", controlplane.ProductCode("invalid"))
	if !errors.Is(err, controlplane.ErrInvalidRequest) {
		t.Fatalf("RevokeActivationCodeForProduct() error = %v, want invalid request", err)
	}
}
