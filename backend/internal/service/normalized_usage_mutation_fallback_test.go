package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestNormalizedUsageMutationFailsClosedWithoutRepository(t *testing.T) {
	repository := &normalizedUsageMutationFallbackRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	service := NewControlPlaneWithRepository(repository)
	input := controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_0001", LeaseID: "lease_0001", Provider: "openai", Model: "gpt-4o",
		Status: "succeeded", UsageSource: "client_reported", InputTokens: 1, OutputTokens: 2,
	}
	_, err := service.RecordDirectLLMCallWithAudit(context.Background(), "usage-0001", "user_0001", "device_0001", "request-0001", input, controlplane.AuditLogInput{})
	if !errors.Is(err, store.ErrNormalizedModelUsageRepositoryRequired) {
		t.Fatalf("RecordDirectLLMCall() error = %v, want normalized model usage repository requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized usage mutation used StateOperation %d times", repository.runCalls)
	}
}

type normalizedUsageMutationFallbackRepository struct {
	*store.MemoryStore
	runCalls int
}

func (r *normalizedUsageMutationFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized usage mutation must not use StateOperation")
}

func (*normalizedUsageMutationFallbackRepository) UsesNormalizedReadSource() bool { return true }
