package store

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestStateSnapshotRedactsPlainActivationCodeAndSecretRef(t *testing.T) {
	state := NewState()
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
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_accounts (")).
		WithArgs("mpa_new", "openai", "rewrite", "https://example.com", "model-account/new", "active", 0, 1, 0, 0, 0).
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
