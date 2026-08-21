package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestNormalizedActivationMutationsFailClosedWithoutRepository(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &normalizedActivationMutationFallbackRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	if _, err := service.CreateActivationCode(ctx, "create-code", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour)}); !errors.Is(err, store.ErrNormalizedActivationRepositoryRequired) {
		t.Fatalf("CreateActivationCode() error = %v, want normalized activation repository requirement", err)
	}
	if _, err := service.RevokeActivationCode(ctx, "revoke-code", "code_1"); !errors.Is(err, store.ErrNormalizedActivationRepositoryRequired) {
		t.Fatalf("RevokeActivationCode() error = %v, want normalized activation repository requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized activation mutations used StateOperation %d times", repository.runCalls)
	}
}

type normalizedActivationMutationFallbackRepository struct {
	*store.MemoryStore
	runCalls int
}

func (r *normalizedActivationMutationFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized activation mutation must not use StateOperation")
}

func (*normalizedActivationMutationFallbackRepository) UsesNormalizedReadSource() bool { return true }
