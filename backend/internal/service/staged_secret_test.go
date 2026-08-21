package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestCleanupStagedSecretsProtectsCurrentAccountReferences(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelPoolAccounts["account_a"] = controlplane.ModelPoolAccountSummary{ID: "account_a", SecretRef: "model-account/account-a/rotation_live"}
		state.ModelPoolAccounts["account_b"] = controlplane.ModelPoolAccountSummary{ID: "account_b", SecretRef: "model-account/account-b"}
		return nil
	}); err != nil {
		t.Fatalf("seed repository: %v", err)
	}
	secretStore := &recordingStagedSecretStore{MemorySecretStore: store.NewMemorySecretStore()}
	service := NewControlPlaneWithRepositoryAndSecretStore(repository, nil, secretStore)
	request := store.RetentionCleanupRequest{Cutoff: time.Date(2026, 8, 21, 10, 0, 0, 0, time.UTC), BatchSize: 17}
	deleted, err := service.CleanupStagedSecrets(context.Background(), request)
	if err != nil {
		t.Fatalf("CleanupStagedSecrets() error = %v", err)
	}
	if deleted != 3 {
		t.Fatalf("deleted = %d, want 3", deleted)
	}
	want := []string{"model-account/account-a/rotation_live", "model-account/account-b"}
	if len(secretStore.protected) != len(want) {
		t.Fatalf("protected refs = %v, want %v", secretStore.protected, want)
	}
	for index := range want {
		if secretStore.protected[index] != want[index] {
			t.Fatalf("protected refs = %v, want %v", secretStore.protected, want)
		}
	}
}

type recordingStagedSecretStore struct {
	*store.MemorySecretStore
	protected []string
}

func (s *recordingStagedSecretStore) CleanupStagedSecrets(_ context.Context, _ store.RetentionCleanupRequest, protected []string) (int64, error) {
	s.protected = append([]string(nil), protected...)
	return 3, nil
}
