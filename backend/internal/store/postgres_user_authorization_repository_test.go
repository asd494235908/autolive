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

func TestPostgresRepositoryUpdateUserAuthorizationWritesNormalizedDomain(t *testing.T) {
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
	input := controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"anthropic/claude", "openai/rewrite"}, DailyTokenLimit: 100}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "update-user-authorization:usr_1:key-1", "fp-1", "usr_1", now).WillReturnRows(
		sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-1", "usr_1"),
	)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO user_authorization_policies (user_id, allowed_models, daily_token_limit, updated_at)")).WithArgs("usr_1", []byte(`["anthropic/claude","openai/rewrite"]`), 100, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	policy, err := repository.UpdateUserAuthorization(context.Background(), "control-plane-state", "update-user-authorization:usr_1:key-1", "fp-1", "usr_1", input)
	if err != nil {
		t.Fatalf("UpdateUserAuthorization() error = %v", err)
	}
	if policy.UserID != "usr_1" || len(policy.AllowedModels) != 2 || policy.DailyTokenLimit != 100 || policy.UpdatedAt != now.Format(time.RFC3339) {
		t.Fatalf("policy = %+v", policy)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryUpdateUserAuthorizationReplaysAndRejectsConflict(t *testing.T) {
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
	input := controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"openai/rewrite"}}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "update-user-authorization:usr_1:key-2", "fp-2", "usr_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "update-user-authorization:usr_1:key-2").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-2", "usr_1"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT user_id, allowed_models, daily_token_limit, updated_at FROM user_authorization_policies")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"user_id", "allowed_models", "daily_token_limit", "updated_at"}).AddRow("usr_1", []byte(`["openai/rewrite"]`), 0, now),
	)
	mock.ExpectRollback()
	policy, err := repository.UpdateUserAuthorization(context.Background(), "control-plane-state", "update-user-authorization:usr_1:key-2", "fp-2", "usr_1", input)
	if err != nil || policy.UserID != "usr_1" {
		t.Fatalf("idempotent replay = %+v, error %v", policy, err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).AddRow("usr_1", "alice", controlplane.RoleUser, controlplane.UserStatusActive, now),
	)
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).WithArgs("control-plane-state", "update-user-authorization:usr_1:key-3", "fp-3", "usr_1", now).WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records")).WithArgs("control-plane-state", "update-user-authorization:usr_1:key-3").WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("different", "usr_1"))
	mock.ExpectRollback()
	if _, err := repository.UpdateUserAuthorization(context.Background(), "control-plane-state", "update-user-authorization:usr_1:key-3", "fp-3", "usr_1", input); !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("idempotency conflict error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetUserAuthorizationSummaryUsesNormalizedAggregates(t *testing.T) {
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
	dayStart := time.Date(2026, 8, 21, 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT allowed_models, daily_token_limit")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"allowed_models", "daily_token_limit"}).AddRow([]byte(`["openai/rewrite"]`), 100),
	)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*), COUNT(*) FILTER (WHERE status = 'active')")).WithArgs("usr_1").WillReturnRows(sqlmock.NewRows([]string{"device_count", "active_device_count"}).AddRow(2, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FILTER (WHERE status = 'active' AND expires_at > $2)")).WithArgs("usr_1", now).WillReturnRows(sqlmock.NewRows([]string{"active_lease_count", "active_account_count"}).AddRow(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COALESCE(SUM(u.total_tokens), 0)")).WithArgs("usr_1", dayStart, dayEnd).WillReturnRows(sqlmock.NewRows([]string{"daily_used_tokens"}).AddRow(17))
	mock.ExpectCommit()

	summary, err := repository.GetUserAuthorizationSummary(context.Background(), "usr_1")
	if err != nil {
		t.Fatalf("GetUserAuthorizationSummary() error = %v", err)
	}
	if summary.UserID != "usr_1" || summary.DeviceCount != 2 || summary.ActiveDeviceCount != 1 || summary.ActiveLeaseCount != 1 || summary.ActiveAccountCount != 1 || summary.DailyUsedTokens != 17 || summary.DailyTokenLimit != 100 || len(summary.AllowedModels) != 1 {
		t.Fatalf("authorization summary = %+v", summary)
	}
	if summary.HardQuotaConfigured || summary.UsageSource != "client_reported_soft" || summary.QuotaEnforcement != "server_recorded_usage_guard" {
		t.Fatalf("authorization quota semantics = %+v", summary)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
