package store

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
	"github.com/lib/pq"
)

func TestPostgresRepositoryCreateUserWritesNormalizedDomainAndIdempotency(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-user:key-1", "fp-1", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "usr_generated"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO users (id, username, password_hash, role, status, created_at)")).WithArgs(sqlmock.AnyArg(), "alice", []byte("$2a$10$hash"), controlplane.RoleUser, controlplane.UserStatusActive, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	user, err := repository.CreateUser(context.Background(), "control-plane-state", "create-user:key-1", "fp-1", UserCreateRecord{
		Username: "alice", Role: controlplane.RoleUser, PasswordHash: []byte("$2a$10$hash"), CreatedAt: now,
	})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if !strings.HasPrefix(user.ID, "usr_") || user.Username != "alice" || user.Role != controlplane.RoleUser || user.Status != controlplane.UserStatusActive || user.CreatedAt != now.Format(time.RFC3339) {
		t.Fatalf("created user = %+v", user)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateUserReplaysAndRejectsIdempotency(t *testing.T) {
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
	create := UserCreateRecord{Username: "alice", Role: controlplane.RoleUser, PasswordHash: []byte("hash"), CreatedAt: now}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-user:key-2", "fp-2", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "create-user:key-2").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-2", "usr_existing"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_existing").WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_existing", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectRollback()
	user, err := repository.CreateUser(context.Background(), "control-plane-state", "create-user:key-2", "fp-2", create)
	if err != nil || user.ID != "usr_existing" {
		t.Fatalf("idempotent replay = %+v, error %v", user, err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-user:key-3", "fp-3", sqlmock.AnyArg(), now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "create-user:key-3").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-other", "usr_other"))
	mock.ExpectRollback()
	if _, err := repository.CreateUser(context.Background(), "control-plane-state", "create-user:key-3", "fp-3", create); !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("idempotency conflict error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCreateUserMapsUsernameConflict(t *testing.T) {
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
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "create-user:key-4", "fp-4", sqlmock.AnyArg(), sqlmock.AnyArg()).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-4", "usr_generated"))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO users (id, username, password_hash, role, status, created_at)")).WillReturnError(&pq.Error{Code: "23505"})
	mock.ExpectRollback()
	if _, err := repository.CreateUser(context.Background(), "control-plane-state", "create-user:key-4", "fp-4", UserCreateRecord{Username: "alice", Role: controlplane.RoleUser, PasswordHash: []byte("hash")}); !errors.Is(err, controlplane.ErrUsernameAlreadyExists) {
		t.Fatalf("username conflict error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryUpdateUserWritesNormalizedDomain(t *testing.T) {
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
	username := "alice-renamed"
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "update-user:usr_1:key-1", "fp-1", "usr_1", now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "usr_1"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS(SELECT 1 FROM users WHERE username = $1 AND id <> $2)")).WithArgs(username, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(false))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM users WHERE role = $1 AND status = $2 AND id <> $3")).WithArgs(controlplane.RoleAdmin, controlplane.UserStatusActive, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE users SET username = $2, role = $3, status = $4 WHERE id = $1")).WithArgs("usr_1", username, controlplane.RoleUser, controlplane.UserStatusActive).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	user, err := repository.UpdateUser(context.Background(), "control-plane-state", "update-user:usr_1:key-1", "fp-1", UserUpdateRecord{UserID: "usr_1", Username: &username})
	if err != nil {
		t.Fatalf("UpdateUser() error = %v", err)
	}
	if user.ID != "usr_1" || user.Username != username || user.Role != controlplane.RoleUser || user.Status != controlplane.UserStatusActive {
		t.Fatalf("updated user = %+v", user)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryUpdateUserProtectsLastActiveAdmin(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	status := controlplane.UserStatusDisabled
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_admin").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_admin", "admin", controlplane.RoleAdmin, controlplane.UserStatusActive, time.Now()),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "update-user:usr_admin:key-2", "fp-2", "usr_admin", sqlmock.AnyArg()).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-2", "usr_admin"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM users WHERE role = $1 AND status = $2 AND id <> $3")).WithArgs(controlplane.RoleAdmin, controlplane.UserStatusActive, "usr_admin").WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	mock.ExpectRollback()
	if _, err := repository.UpdateUser(context.Background(), "control-plane-state", "update-user:usr_admin:key-2", "fp-2", UserUpdateRecord{UserID: "usr_admin", Status: &status}); !errors.Is(err, controlplane.ErrLastActiveAdmin) {
		t.Fatalf("last admin error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryResetUserPasswordWritesNormalizedDomain(t *testing.T) {
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
	hash := []byte("$2a$10$hash")
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "reset-user-password:usr_1:key-3", "fp-3", "usr_1", now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-3", "usr_1"),
	)
	mock.ExpectExec(regexp.QuoteMeta("UPDATE users SET password_hash = $2 WHERE id = $1")).WithArgs("usr_1", hash).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	user, err := repository.ResetUserPassword(context.Background(), "control-plane-state", "reset-user-password:usr_1:key-3", "fp-3", "usr_1", hash)
	if err != nil || user.ID != "usr_1" {
		t.Fatalf("ResetUserPassword() = %+v, error %v", user, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryDisableUserWritesNormalizedDomain(t *testing.T) {
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
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "disable-user:usr_1:key-4", "fp-4", "usr_1", now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-4", "usr_1"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM users WHERE role = $1 AND status = $2 AND id <> $3")).WithArgs(controlplane.RoleAdmin, controlplane.UserStatusActive, "usr_1").WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE users SET status = $2 WHERE id = $1")).WithArgs("usr_1", controlplane.UserStatusDisabled).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	user, err := repository.DisableUser(context.Background(), "control-plane-state", "disable-user:usr_1:key-4", "fp-4", "usr_1")
	if err != nil || user.Status != controlplane.UserStatusDisabled {
		t.Fatalf("DisableUser() = %+v, error %v", user, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryDisableUserProtectsLocalAdmin(t *testing.T) {
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
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_local_admin").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_local_admin", "admin", controlplane.RoleAdmin, controlplane.UserStatusActive, time.Now()),
	)
	mock.ExpectRollback()
	if _, err := repository.DisableUser(context.Background(), "control-plane-state", "disable-user:usr_local_admin:key-5", "fp-5", "usr_local_admin"); !errors.Is(err, controlplane.ErrCannotDisableLocalAdmin) {
		t.Fatalf("local admin disable error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestStateSnapshotRedactsPlainActivationCodeAndSecretRef(t *testing.T) {
	state := NewState()
	queuedAt := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	state.PendingSecretCleanup["rotation-old-ref"] = queuedAt
	plainCode := "code_sensitive_value"
	state.ActivationCodes["ac_1"] = ActivationCodeRecord{
		ActivationCode: controlplane.ActivationCode{ID: "ac_1", PlainCode: &plainCode, ExpiresAt: "2026-08-14T00:00:00Z"},
		PlainCode:      plainCode, CodePrefix: "code_sensitive",
	}
	state.ActivationCodeIndex["digest"] = "ac_1"
	state.ModelPoolAccounts["mpa_1"] = controlplane.ModelPoolAccountSummary{
		ID: "mpa_1", Provider: "openai", Model: "rewrite", SecretConfigured: true, SecretRef: "model-account/mpa_1",
	}
	payload, err := marshalStateSnapshot(state)
	if err != nil {
		t.Fatalf("marshalStateSnapshot() error = %v", err)
	}
	if bytes.Contains(payload, []byte(plainCode)) || bytes.Contains(payload, []byte("model-account/mpa_1")) {
		t.Fatalf("snapshot contains sensitive value: %s", payload)
	}
	var decoded stateSnapshot
	if err := json.Unmarshal(payload, &decoded); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}
	if decoded.State.ModelPoolAccounts["mpa_1"].SecretRef != "" {
		t.Fatalf("snapshot account still contains secret ref")
	}
	restored, err := unmarshalStateSnapshot(payload)
	if err != nil {
		t.Fatalf("unmarshalStateSnapshot() error = %v", err)
	}
	if restored.ModelPoolAccounts["mpa_1"].SecretRef != "model-account/mpa_1" || restored.ActivationCodes["ac_1"].PlainCode != "" {
		t.Fatalf("restored sensitive fields = %+v / %+v", restored.ModelPoolAccounts["mpa_1"], restored.ActivationCodes["ac_1"])
	}
	if restored.PendingSecretCleanup["rotation-old-ref"] != queuedAt {
		t.Fatalf("restored pending secret cleanup = %+v", restored.PendingSecretCleanup)
	}
}

func TestPostgresRepositoryTryAdvisoryLockKeepsSessionUntilRelease(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	const key int64 = 0x6175746f6c697665
	mock.ExpectQuery(regexp.QuoteMeta("SELECT pg_try_advisory_lock($1)")).
		WithArgs(key).
		WillReturnRows(sqlmock.NewRows([]string{"pg_try_advisory_lock"}).AddRow(true))
	release, acquired, err := repository.TryAdvisoryLock(context.Background(), key)
	if err != nil || !acquired || release == nil {
		t.Fatalf("TryAdvisoryLock() = release %v, acquired %t, error %v; want acquired lock", release != nil, acquired, err)
	}
	mock.ExpectQuery(regexp.QuoteMeta("SELECT pg_advisory_unlock($1)")).
		WithArgs(key).
		WillReturnRows(sqlmock.NewRows([]string{"pg_advisory_unlock"}).AddRow(true))
	releaseCtx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := release(releaseCtx); err != nil {
		t.Fatalf("release() error = %v", err)
	}
	if err := release(context.Background()); err != nil {
		t.Fatalf("second release() error = %v, want idempotent release", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sqlmock expectations: %v", err)
	}
}

func TestPostgresRepositoryTryAdvisoryLockReportsBusyWithoutUnlock(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	const key int64 = 0x6175746f6c697665
	mock.ExpectQuery(regexp.QuoteMeta("SELECT pg_try_advisory_lock($1)")).
		WithArgs(key).
		WillReturnRows(sqlmock.NewRows([]string{"pg_try_advisory_lock"}).AddRow(false))
	release, acquired, err := repository.TryAdvisoryLock(context.Background(), key)
	if err != nil || acquired || release != nil {
		t.Fatalf("TryAdvisoryLock() = release %v, acquired %t, error %v; want busy lock", release != nil, acquired, err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sqlmock expectations: %v", err)
	}
}

func TestPostgresRepositoryRunCommitsCurrentStateInOneTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, func() time.Time { return time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC) })
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).
		WillReturnRows(sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, secret_ref FROM model_accounts")).
		WillReturnRows(sqlmock.NewRows([]string{"id", "secret_ref"}))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO users (")).
		WithArgs("user_1", "user", "!configured-outside-control-plane!", "user", "active", "2026-08-13T00:00:00Z").
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE control_plane_state")).
		WithArgs(sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	err = repository.Run(context.Background(), func(state *State) error {
		state.Users["user_1"] = controlplane.UserSummary{ID: "user_1", Username: "user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-13T00:00:00Z"}
		return nil
	})
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRunWithSessionBindingCommitsBindingAndStateTogether(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta(`
			SELECT user_id, device_id
			FROM auth_sessions
			WHERE access_token_hash = $1 AND revoked_at IS NULL
			FOR UPDATE
		`)).WithArgs("access_hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "device_id"}).AddRow("user_1", nil))
	mock.ExpectExec(regexp.QuoteMeta(`
				UPDATE auth_sessions
				SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP
				WHERE access_token_hash = $1 AND revoked_at IS NULL
			`)).WithArgs("access_hash", "device_1").WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).
		WillReturnRows(sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, secret_ref FROM model_accounts")).
		WillReturnRows(sqlmock.NewRows([]string{"id", "secret_ref"}))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE control_plane_state")).
		WithArgs(sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	if err := repository.RunWithSessionBinding(context.Background(), "access_hash", "user_1", "device_1", func(state *State) error {
		if state == nil {
			t.Fatal("state is nil")
		}
		return nil
	}); err != nil {
		t.Fatalf("RunWithSessionBinding() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRunWithSessionBindingRejectsConflictingDevice(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta(`
			SELECT user_id, device_id
			FROM auth_sessions
			WHERE access_token_hash = $1 AND revoked_at IS NULL
			FOR UPDATE
		`)).WithArgs("access_hash").WillReturnRows(sqlmock.NewRows([]string{"user_id", "device_id"}).AddRow("user_1", "device_other"))
	mock.ExpectRollback()

	err = repository.RunWithSessionBinding(context.Background(), "access_hash", "user_1", "device_1", func(*State) error {
		t.Fatal("state operation must not run after binding conflict")
		return nil
	})
	if !errors.Is(err, ErrSessionDeviceBindingConflict) {
		t.Fatalf("RunWithSessionBinding() error = %v, want binding conflict", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRejectsMissingOperationTimeout(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	if _, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, time.Now, nil, ModelReadSourceSnapshot, 0); err == nil {
		t.Fatal("constructor error = nil, want invalid operation timeout")
	}
}

func expectNormalizedPageCoverage(mock sqlmock.Sqlmock) {
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).
		WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("completed"))
}

func TestPostgresRepositoryNormalizedUserPageUsesBoundedQuery(t *testing.T) {
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
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM users")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(3))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at")).
		WithArgs(2, 2).
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_00000003", "charlie", controlplane.RoleUser, controlplane.UserStatusActive, time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)))
	mock.ExpectCommit()

	page, err := repository.ListUsersPage(context.Background(), 2, 2)
	if err != nil {
		t.Fatalf("ListUsersPage() error = %v", err)
	}
	if page.Total != 3 || len(page.Items) != 1 || page.Items[0].ID != "usr_00000003" {
		t.Fatalf("users page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedUserDevicePageChecksOwnershipAndBoundsSQL(t *testing.T) {
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
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")).
		WithArgs("usr_00000001").
		WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM devices WHERE user_id = $1")).
		WithArgs("usr_00000001").
		WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).
		WithArgs("usr_00000001", 20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_name", "platform", "client_version", "status",
			"disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores",
			"runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at",
		}).AddRow("dev_00000001", "usr_00000001", string(controlplane.ProductAutoLive), "Studio", "windows", "1.2.3", controlplane.DeviceStatusActive,
			int64(100), int64(200), int64(150), 8, "Windows", "11", "kernel", "demo.mp4", "playing", time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)))
	mock.ExpectCommit()

	page, err := repository.ListDevicesForUserPage(context.Background(), "usr_00000001", 0, 20)
	if err != nil {
		t.Fatalf("ListDevicesForUserPage() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].ID != "dev_00000001" || page.Items[0].CurrentMediaName != "demo.mp4" {
		t.Fatalf("devices page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedUsagePageUsesStableOrdering(t *testing.T) {
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
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_usage_records")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, lease_id, client_call_id, request_id, provider, model")).
		WithArgs(20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "lease_id", "client_call_id", "request_id", "provider", "model",
			"prompt_tokens", "completion_tokens", "total_tokens", "latency_ms", "status", "usage_source", "error_code", "created_at",
		}).AddRow("usage_00000001", "lease_00000001", "call_00000001", "req_00000001", "openai", "rewrite", 4, 6, 10, int64(25), "succeeded", "client_reported", nil, time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)))
	mock.ExpectCommit()

	page, err := repository.ListModelUsagePage(context.Background(), 0, 20)
	if err != nil {
		t.Fatalf("ListModelUsagePage() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].TotalTokens != 10 || page.Items[0].UsageSource != "client_reported" {
		t.Fatalf("usage page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedAuditPageKeepsStructuredFields(t *testing.T) {
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
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM audit_logs")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, actor_user_id, device_id, action, resource_type")).
		WithArgs(20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "product", "actor_user_id", "device_id", "action", "resource_type", "resource_id", "request_id", "outcome", "status_code", "error_code", "created_at",
		}).AddRow("audit_00000001", string(controlplane.ProductAutoLive), "usr_00000001", "dev_00000001", "update", "user", "usr_00000001", "req_00000001", "failure", 409, "CONFLICT", time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)))
	mock.ExpectCommit()

	page, err := repository.ListAuditLogsPage(context.Background(), 0, 20)
	if err != nil {
		t.Fatalf("ListAuditLogsPage() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].Outcome != "failure" || page.Items[0].StatusCode != 409 || page.Items[0].ErrorCode != "CONFLICT" {
		t.Fatalf("audit page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedAuditFilteredPageUsesWhitelist(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	createdAfter := time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)
	createdBefore := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	countExpectation := mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM audit_logs WHERE actor_user_id = $1 AND outcome = $2 AND created_at >= $3 AND created_at <= $4"))
	countExpectation.WithArgs("usr_00000001", "failure", createdAfter, createdBefore).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	pageExpectation := mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, actor_user_id, device_id, action, resource_type"))
	pageExpectation.WithArgs("usr_00000001", "failure", createdAfter, createdBefore, 10, 1).WillReturnRows(sqlmock.NewRows([]string{
		"id", "product", "actor_user_id", "device_id", "action", "resource_type", "resource_id", "request_id", "outcome", "status_code", "error_code", "created_at",
	}).AddRow("audit_00000001", string(controlplane.ProductAutoLive), "usr_00000001", "dev_00000001", "PATCH /users", "user", "usr_00000001", "req_00000001", "failure", 409, "CONFLICT", createdBefore))
	mock.ExpectCommit()

	page, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{
		Offset: 1, Limit: 10, ActorUserID: "usr_00000001", Outcome: "failure", CreatedAfter: &createdAfter, CreatedBefore: &createdBefore, Sort: AuditLogSortCreatedAsc,
	})
	if err != nil {
		t.Fatalf("ListAuditLogsPageWithOptions() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].RequestID != "req_00000001" {
		t.Fatalf("filtered audit page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModelPoolPageUsesBoundedDerivedQuery(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 20, 0, 4, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	payload, err := json.Marshal(controlplane.ModelPoolConnectivityTestResult{
		AccountID: "mpa_00000001", Provider: "openai", Model: "rewrite", Status: "failed", TestedAt: "2026-08-19T23:59:00Z",
	})
	if err != nil {
		t.Fatalf("json.Marshal() error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_accounts")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT a.id, a.provider, a.model, a.base_url, a.secret_ref, a.status")).
		WithArgs(sqlmock.AnyArg(), sqlmock.AnyArg(), sqlmock.AnyArg(), 20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until",
			"active_leases", "daily_used_tokens", "payload", "created_at",
		}).AddRow("mpa_00000001", "openai", "rewrite", "https://example.com/v1", "model-account/1", "active", 10, 2, 1000, nil, 1, 42, payload, now.Add(-time.Minute)))
	mock.ExpectCommit()

	page, err := repository.ListModelPoolAccountsPage(context.Background(), 0, 20)
	if err != nil {
		t.Fatalf("ListModelPoolAccountsPage() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 {
		t.Fatalf("model pool page = %+v", page)
	}
	item := page.Items[0]
	if item.ID != "mpa_00000001" || !item.SecretConfigured || item.ActiveLeases != 1 || item.DailyUsedTokens != 42 || item.LastTestStatus != "failed" || item.LastTestedAt != "2026-08-19T23:59:00Z" {
		t.Fatalf("model pool item = %+v", item)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModelPoolHealthPageFiltersBeforeLimit(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 20, 0, 4, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT a.id, a.provider, a.model, a.base_url, a.secret_ref, a.status")).
		WithArgs(now, dayStart, dayStart.Add(24*time.Hour), 1).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until",
			"active_leases", "daily_used_tokens", "payload", "created_at",
		}).AddRow("mpa_ready", "openai", "rewrite", "https://example.com/v1", "model-account/ready", "active", 10, 2, 1000, nil, 0, 42, nil, now))
	mock.ExpectCommit()

	items, err := repository.ListModelPoolHealthAccounts(context.Background(), 1)
	if err != nil {
		t.Fatalf("ListModelPoolHealthAccounts() error = %v", err)
	}
	if len(items) != 1 || items[0].ID != "mpa_ready" || items[0].DailyUsedTokens != 42 {
		t.Fatalf("health accounts = %+v", items)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModelLeasePageUsesBoundedQuery(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	expiresAt := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at")).
		WithArgs(20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "provider", "model", "proxy_mode", "concurrency_limit",
		}).AddRow("lease_00000001", "mpa_00000001", "usr_00000001", "dev_00000001", "client", "active", expiresAt, "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectCommit()

	page, err := repository.ListModelLeasesPage(context.Background(), 0, 20)
	if err != nil {
		t.Fatalf("ListModelLeasesPage() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].AccountID != "mpa_00000001" || page.Items[0].ProxyMode != controlplane.ModelLeaseProxyModeDirectLease {
		t.Fatalf("model lease page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModelLeaseFilteredPageUsesWhitelist(t *testing.T) {
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
	expectNormalizedPageCoverage(mock)
	countExpectation := mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases WHERE status = 'active' AND expires_at > $1 AND provider = $2"))
	countExpectation.WithArgs(sqlmock.AnyArg(), "openai").WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	pageExpectation := mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at"))
	pageExpectation.WithArgs(sqlmock.AnyArg(), "openai", 5, 2).WillReturnRows(sqlmock.NewRows([]string{
		"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "provider", "model", "proxy_mode", "concurrency_limit",
	}).AddRow("lease_00000001", "mpa_00000001", "usr_00000001", "dev_00000001", "client", "active", time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC), "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectCommit()

	page, err := repository.ListModelLeasesPageWithOptions(context.Background(), ModelLeasePageOptions{
		Offset: 2, Limit: 5, Status: controlplane.ModelLeaseStatusActive, Provider: "openai", Sort: ModelLeaseSortProviderModel,
	})
	if err != nil {
		t.Fatalf("ListModelLeasesPageWithOptions() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].Provider != "openai" {
		t.Fatalf("filtered model lease page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRunHonorsOperationTimeout(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, time.Now, nil, ModelReadSourceSnapshot, 10*time.Millisecond)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).
		WillDelayFor(50 * time.Millisecond).
		WillReturnRows(sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)))

	err = repository.Run(context.Background(), func(state *State) error {
		t.Fatal("state callback should not run after statement timeout")
		return nil
	})
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Run() error = %v, want context deadline exceeded", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryCompensatesSecretAfterSnapshotFailure(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	secretStore := NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/new", "secret-value"); err != nil {
		t.Fatalf("secretStore.Put() error = %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStore(database, time.Now, secretStore)
	if err != nil {
		t.Fatalf("NewPostgresRepositoryWithSecretStore() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).
		WillReturnRows(sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, secret_ref FROM model_accounts")).
		WillReturnRows(sqlmock.NewRows([]string{"id", "secret_ref"}))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_accounts (")).
		WithArgs("mpa_new", "openai", "rewrite", "https://example.com", "model-account/new", "active", 0, 1, 0, nil, 0, 0).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE control_plane_state")).
		WithArgs(sqlmock.AnyArg()).WillReturnError(errors.New("snapshot write failed"))
	mock.ExpectRollback()

	err = repository.Run(context.Background(), func(state *State) error {
		state.ModelPoolAccounts["mpa_new"] = controlplane.ModelPoolAccountSummary{ID: "mpa_new", Provider: "openai", Model: "rewrite", BaseURL: "https://example.com", Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/new", ConcurrencyLimit: 1}
		return nil
	})
	if err == nil {
		t.Fatal("Run() error = nil, want snapshot failure")
	}
	if _, err := secretStore.Get(context.Background(), "model-account/new"); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("compensated secret lookup error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryPreservesSecretAfterUnknownCommitOutcome(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	secretStore := NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/new", "secret-value"); err != nil {
		t.Fatalf("secretStore.Put() error = %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStore(database, time.Now, secretStore)
	if err != nil {
		t.Fatalf("NewPostgresRepositoryWithSecretStore() error = %v", err)
	}
	commitErr := errors.New("connection reset after COMMIT")
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).
		WillReturnRows(sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, secret_ref FROM model_accounts")).
		WillReturnRows(sqlmock.NewRows([]string{"id", "secret_ref"}))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_accounts (")).
		WithArgs("mpa_new", "openai", "rewrite", "https://example.com", "model-account/new", "active", 0, 1, 0, nil, 0, 0).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE control_plane_state")).
		WithArgs(sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit().WillReturnError(commitErr)

	err = repository.Run(context.Background(), func(state *State) error {
		state.ModelPoolAccounts["mpa_new"] = controlplane.ModelPoolAccountSummary{
			ID: "mpa_new", Provider: "openai", Model: "rewrite", BaseURL: "https://example.com",
			Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/new", ConcurrencyLimit: 1,
		}
		return nil
	})
	if !errors.Is(err, ErrCommitOutcomeUnknown) {
		t.Fatalf("Run() error = %v, want ErrCommitOutcomeUnknown", err)
	}
	if _, err := secretStore.Get(context.Background(), "model-account/new"); err != nil {
		t.Fatalf("staged secret after unknown commit outcome = %v, want preserved secret", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresCommitErrorPreservesContextCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := postgresCommitError(ctx, "test commit", errors.New("driver failure")); !errors.Is(err, context.Canceled) {
		t.Fatalf("postgresCommitError() = %v, want context.Canceled", err)
	}
	if errors.Is(postgresCommitError(context.Background(), "test commit", context.Canceled), ErrCommitOutcomeUnknown) {
		t.Fatalf("context cancellation should not be classified as unknown commit outcome")
	}
}

func TestPostgresRepositoryNormalizedModeLoadsDomainTablesUnderAdvisoryLock(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(
		database,
		func() time.Time { return time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC) },
		nil,
		ModelReadSourceNormalized,
	)
	if err != nil {
		t.Fatalf("NewPostgresRepositoryWithSecretStoreAndModelReadSource() error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, password_hash, role, status, created_at")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, allowed_models, daily_token_limit, updated_at")).WillReturnRows(sqlmock.NewRows([]string{"user_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, code_hash, code_prefix, status, expires_at, used_at")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, lease_id, client_call_id, request_id, provider, model")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, payload FROM model_pool_test_results")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT idempotency_key, fingerprint, resource_id")).WillReturnRows(sqlmock.NewRows([]string{"idempotency_key"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, actor_user_id, device_id, action, resource_type")).WillReturnRows(sqlmock.NewRows([]string{"id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("completed"))
	mock.ExpectCommit()

	if err := repository.Run(context.Background(), func(state *State) error {
		if len(state.Users) != 0 || len(state.ModelPoolAccounts) != 0 {
			t.Fatalf("normalized state should start empty for empty domain tables: %+v", state)
		}
		return nil
	}); err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModeRestoresDomainFields(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	createdAt := now.Add(-time.Hour)
	usedAt := now.Add(-30 * time.Minute)
	payload, err := json.Marshal(controlplane.ModelPoolConnectivityTestResult{
		AccountID: "mpa_00000007", Provider: "openai", Model: "rewrite", Status: "ok", TestedAt: now.Format(time.RFC3339),
	})
	if err != nil {
		t.Fatalf("marshal test result: %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, password_hash, role, status, created_at")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "password_hash", "role", "status", "created_at"}).AddRow("usr_00000007", "alice", "$2a$hash", "user", "active", createdAt),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, allowed_models, daily_token_limit, updated_at")).WillReturnRows(
		sqlmock.NewRows([]string{"user_id", "allowed_models", "daily_token_limit", "updated_at"}).AddRow("usr_00000007", []byte(`["openai/rewrite"]`), 100, createdAt),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "user_id", "product", "device_name", "platform", "client_version", "status", "disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores", "runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at"}).AddRow("dev_00000007", "usr_00000007", string(controlplane.ProductAutoLive), "Studio", "windows", "1.2.3", "active", int64(10), int64(20), int64(15), 8, "Windows", "11", "kernel", "demo.mp4", "playing", now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, code_hash, code_prefix, status, expires_at, used_at")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "code_hash", "code_prefix", "status", "expires_at", "used_at", "used_by_user_id", "used_by_device_id", "max_devices", "bound_devices"}).AddRow("ac_00000007", "digest", "AUTO-ABCD", "used", now.Add(time.Hour), usedAt, "usr_00000007", "dev_00000007", 1, 1),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, provider, model, base_url, secret_ref, status, priority")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "provider", "model", "base_url", "secret_ref", "status", "priority", "concurrency_limit", "daily_token_limit", "cooldown_until"}).AddRow("mpa_00000007", "openai", "rewrite", "https://example.com/v1", "model-account/7", "active", 2, 3, 1000, nil),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, expires_at")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "created_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit"}).AddRow("lease_00000007", "mpa_00000007", "usr_00000007", "dev_00000007", "client", "active", now.Add(time.Hour), createdAt, nil, "openai", "rewrite", "direct_lease", 3),
	)
	usageRows := sqlmock.NewRows([]string{"id", "lease_id", "client_call_id", "request_id", "provider", "model", "prompt_tokens", "completion_tokens", "total_tokens", "latency_ms", "status", "usage_source", "error_code", "created_at"})
	usageRows.AddRow("usage_00000007", "lease_00000007", "call-7", "req-7", "openai", "rewrite", 4, 6, 10, int64(25), "succeeded", "client_reported", "", createdAt)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, lease_id, client_call_id, request_id, provider, model")).WillReturnRows(usageRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, payload FROM model_pool_test_results")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "payload"}).AddRow("model_test_00000007", payload),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT idempotency_key, fingerprint, resource_id")).WillReturnRows(
		sqlmock.NewRows([]string{"idempotency_key", "fingerprint", "resource_id"}).AddRow("create-user:key-7", "fp-7", "usr_00000007"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, actor_user_id, device_id, action, resource_type")).WillReturnRows(
		sqlmock.NewRows([]string{"id", "product", "actor_user_id", "device_id", "action", "resource_type", "resource_id", "request_id", "outcome", "status_code", "error_code", "created_at"}).AddRow("audit_00000007", string(controlplane.ProductAutoLive), "usr_00000007", "dev_00000007", "create", "user", "usr_00000007", "req-7", "success", 201, nil, createdAt),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("completed"))
	mock.ExpectRollback()

	stop := errors.New("inspect normalized state")
	var loaded *State
	if err := repository.Run(context.Background(), func(state *State) error {
		loaded = state
		return stop
	}); !errors.Is(err, stop) {
		t.Fatalf("Run() error = %v, want callback error", err)
	}
	if loaded == nil {
		t.Fatal("normalized callback did not receive state")
	}
	if loaded.Devices["dev_00000007"].CurrentMediaName != "demo.mp4" || loaded.Devices["dev_00000007"].PlaybackState != "playing" {
		t.Fatalf("device playback fields were not restored: %+v", loaded.Devices["dev_00000007"])
	}
	if loaded.ModelPoolAccounts["mpa_00000007"].SecretRef != "model-account/7" || loaded.ModelPoolAccounts["mpa_00000007"].ActiveLeases != 1 {
		t.Fatalf("model account fields were not restored: %+v", loaded.ModelPoolAccounts["mpa_00000007"])
	}
	if loaded.ModelLeases["lease_00000007"].CreatedAt != createdAt.UTC().Format(time.RFC3339) || loaded.ModelLeases["lease_00000007"].ReleasedAt != "" {
		t.Fatalf("model lease lifecycle fields were not restored: %+v", loaded.ModelLeases["lease_00000007"])
	}
	if loaded.SequenceCounters["usage"] != 7 || loaded.IdempotencyRecords["create-user:key-7"].ResourceID != "usr_00000007" {
		t.Fatalf("normalized derived state was not restored: %+v / %+v", loaded.SequenceCounters, loaded.IdempotencyRecords)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryBackfillNormalizedUsesAdvisoryLockAndCommits(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR UPDATE")).WillReturnRows(
		sqlmock.NewRows([]string{"status"}).AddRow("pending"),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE")).WillReturnRows(
		sqlmock.NewRows([]string{"state"}).AddRow([]byte(`{"version":1,"state":{}}`)),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, secret_ref FROM model_accounts")).WillReturnRows(sqlmock.NewRows([]string{"id", "secret_ref"}))
	for _, query := range []string{
		"SELECT COUNT(*) FROM users",
		"SELECT COUNT(*) FROM user_authorization_policies",
		"SELECT COUNT(*) FROM devices",
		"SELECT COUNT(*) FROM activation_codes",
		"SELECT COUNT(*) FROM model_accounts",
		"SELECT COUNT(*) FROM model_leases",
		"SELECT COUNT(*) FROM model_usage_records",
		"SELECT COUNT(*) FROM model_pool_test_results",
		"SELECT COUNT(*) FROM idempotency_records WHERE scope = 'control-plane-state'",
		"SELECT COUNT(*) FROM audit_logs",
	} {
		mock.ExpectQuery(regexp.QuoteMeta(query)).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	}
	mock.ExpectExec(regexp.QuoteMeta("UPDATE normalized_backfill_state")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	if err := repository.BackfillNormalized(context.Background()); err != nil {
		t.Fatalf("BackfillNormalized() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryBackfillNormalizedRefusesCompletedMarker(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR UPDATE")).WillReturnRows(
		sqlmock.NewRows([]string{"status"}).AddRow("completed"),
	)
	mock.ExpectRollback()
	if err := repository.BackfillNormalized(context.Background()); err == nil || !strings.Contains(err.Error(), "refusing to rerun") {
		t.Fatalf("BackfillNormalized() error = %v, want completed-marker refusal", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedCoverageUsesBackfillMarkerOnly(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).WillReturnRows(sqlmock.NewRows([]string{"status"}).AddRow("completed"))
	mock.ExpectRollback()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	if err = repository.ensureNormalizedCoverage(context.Background(), tx, NewState()); err != nil {
		t.Fatalf("ensureNormalizedCoverage() error = %v, want marker-only gate", err)
	}
	if err := tx.Rollback(); err != nil {
		t.Fatalf("Rollback() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedCoverageRejectsIncompleteBackfillMarker(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatalf("NewPostgresRepository() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE")).WillReturnRows(
		sqlmock.NewRows([]string{"status"}).AddRow("pending"),
	)
	mock.ExpectRollback()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	err = repository.ensureNormalizedCoverage(context.Background(), tx, NewState())
	if err == nil || !strings.Contains(err.Error(), "not complete") {
		t.Fatalf("ensureNormalizedCoverage() error = %v, want incomplete backfill marker", err)
	}
	if err := tx.Rollback(); err != nil {
		t.Fatalf("Rollback() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
