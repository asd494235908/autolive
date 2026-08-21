package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestListModelUsagePageWithOptionsFiltersMemoryByOwnershipAndTime(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelLeases["lease-a"] = controlplane.ModelLease{ID: "lease-a", UserID: "user-a", DeviceID: "device-a"}
		state.ModelLeases["lease-b"] = controlplane.ModelLease{ID: "lease-b", UserID: "user-b", DeviceID: "device-b"}
		state.ModelUsageRecords["usage-a"] = controlplane.ModelUsageRecord{ID: "usage-a", LeaseID: "lease-a", Provider: "openai", Model: "rewrite", RequestID: "request-a", CreatedAt: "2026-08-21T10:00:00Z"}
		state.ModelUsageRecords["usage-b"] = controlplane.ModelUsageRecord{ID: "usage-b", LeaseID: "lease-b", Provider: "openai", Model: "rewrite", RequestID: "request-b", CreatedAt: "2026-08-21T11:00:00Z"}
		return nil
	}); err != nil {
		t.Fatalf("seed usage state: %v", err)
	}
	items, total, err := NewControlPlane(repository).ListModelUsagePageWithOptions(context.Background(), 1, 20, ModelUsageListOptions{
		Provider: " openai ", UserID: "user-a", DeviceID: "device-a", CreatedAfter: "2026-08-21T00:00:00Z",
	})
	if err != nil {
		t.Fatalf("ListModelUsagePageWithOptions() error = %v", err)
	}
	if total != 1 || len(items) != 1 || items[0].ID != "usage-a" {
		t.Fatalf("filtered usage = total %d items %+v", total, items)
	}
	if _, _, err := NewControlPlane(repository).ListModelUsagePageWithOptions(context.Background(), 1, 20, ModelUsageListOptions{CreatedAfter: "not-a-timestamp"}); !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("invalid usage time error = %v, want INVALID_ARGUMENT", err)
	}
	if _, _, err := NewControlPlane(repository).ListModelUsagePageWithOptions(context.Background(), 1, 20, ModelUsageListOptions{CreatedAfter: "2026-08-22T00:00:00Z", CreatedBefore: "2026-08-21T00:00:00Z"}); !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("invalid usage range error = %v, want INVALID_ARGUMENT", err)
	}
}
