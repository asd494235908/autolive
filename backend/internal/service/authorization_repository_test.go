package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedAuthorizationWriterRepository struct {
	*store.MemoryStore
	called bool
	policy controlplane.UserAuthorizationPolicy
}

func (r *normalizedAuthorizationWriterRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedAuthorizationWriterRepository) UpdateUserAuthorization(_ context.Context, _, _, _, userID string, input controlplane.UpdateUserAuthorizationInput) (controlplane.UserAuthorizationPolicy, error) {
	r.called = true
	r.policy = controlplane.UserAuthorizationPolicy{UserID: userID, AllowedModels: input.AllowedModels, DailyTokenLimit: input.DailyTokenLimit, UpdatedAt: time.Now().UTC().Format(time.RFC3339)}
	return r.policy, nil
}

func TestUpdateUserAuthorizationUsesNormalizedRepositoryWriter(t *testing.T) {
	repository := &normalizedAuthorizationWriterRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	policy, err := svc.UpdateUserAuthorization(context.Background(), "policy-key", "usr_1", controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"openai/rewrite"}, DailyTokenLimit: 100})
	if err != nil {
		t.Fatalf("UpdateUserAuthorization() error = %v", err)
	}
	if !repository.called || policy.UserID != "usr_1" || len(policy.AllowedModels) != 1 || policy.DailyTokenLimit != 100 {
		t.Fatalf("normalized authorization writer = called %t policy %+v", repository.called, policy)
	}
}
