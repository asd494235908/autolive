package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// Normalized model-account mutations must not fall back to the compatibility
// snapshot when the Secret/Repository transaction boundary is unavailable.
func TestNormalizedModelAccountMutationsFailClosedWithoutDomainRepositories(t *testing.T) {
	repository := &normalizedModelAccountMutationFallbackRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time {
			return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
		}),
	}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()
	priority := 1

	tests := []struct {
		name string
		want error
		call func() error
	}{
		{name: "create", want: store.ErrNormalizedModelPoolAccountCreatorRequired, call: func() error {
			_, err := service.CreateModelPoolAccount(ctx, "create-account", controlplane.CreateModelPoolAccountInput{
				Provider: "openai", Model: "gpt-4o", APIKey: "test-api-key", Priority: priority, ConcurrencyLimit: 1,
			})
			return err
		}},
		{name: "disable", want: store.ErrNormalizedModelPoolRepositoryRequired, call: func() error {
			_, err := service.DisableModelPoolAccountWithAudit(ctx, "disable-account", "account_1", controlplane.AuditLogInput{})
			return err
		}},
		{name: "update", want: store.ErrNormalizedModelPoolRepositoryRequired, call: func() error {
			_, err := service.UpdateModelPoolAccountWithAudit(ctx, "update-account", "account_1", controlplane.UpdateModelPoolAccountInput{Priority: &priority}, controlplane.AuditLogInput{})
			return err
		}},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := test.call(); !errors.Is(err, test.want) {
				t.Fatalf("error = %v, want %v", err, test.want)
			}
		})
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized model-account mutations used StateOperation %d times", repository.runCalls)
	}
}

type normalizedModelAccountMutationFallbackRepository struct {
	*store.MemoryStore
	runCalls int
}

func (r *normalizedModelAccountMutationFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized model-account mutation must not use StateOperation")
}

func (*normalizedModelAccountMutationFallbackRepository) UsesNormalizedReadSource() bool { return true }
