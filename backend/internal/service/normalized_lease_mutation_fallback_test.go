package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// Normalized lease mutations must not silently materialize and rewrite the
// compatibility snapshot when a domain Repository is unavailable.
func TestNormalizedLeaseMutationsFailClosedWithoutDomainRepositories(t *testing.T) {
	repository := &normalizedLeaseMutationFallbackRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time {
			return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
		}),
	}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	tests := []struct {
		name string
		want error
		call func() error
	}{
		{name: "create", want: store.ErrNormalizedModelLeaseCreatorRequired, call: func() error {
			_, err := service.CreateModelLease(ctx, "create-lease", "user_1", "device_1", controlplane.CreateModelLeaseInput{
				Provider: "openai", Model: "gpt-4o", Purpose: "chat", MaxDurationSeconds: 300,
			})
			return err
		}},
		{name: "renew", want: store.ErrNormalizedModelLeaseRepositoryRequired, call: func() error {
			_, err := service.RenewModelLease(ctx, "renew-lease", "user_1", "device_1", "lease_1", controlplane.RenewModelLeaseInput{ExtendSeconds: 300})
			return err
		}},
		{name: "release", want: store.ErrNormalizedModelLeaseRepositoryRequired, call: func() error {
			_, err := service.ReleaseModelLease(ctx, "release-lease", "user_1", "device_1", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "done"})
			return err
		}},
		{name: "reclaim", want: store.ErrNormalizedModelLeaseRepositoryRequired, call: func() error {
			_, err := service.ReclaimModelLease(ctx, "reclaim-lease", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "cleanup"})
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
		t.Fatalf("normalized lease mutations used StateOperation %d times", repository.runCalls)
	}
}

type normalizedLeaseMutationFallbackRepository struct {
	*store.MemoryStore
	runCalls int
}

func (r *normalizedLeaseMutationFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized lease mutation must not use StateOperation")
}

func (*normalizedLeaseMutationFallbackRepository) UsesNormalizedReadSource() bool { return true }
