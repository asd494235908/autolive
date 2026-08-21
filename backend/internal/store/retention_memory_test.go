package store

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestMemoryStoreRetentionCleanupUsesCutoffAndBatch(t *testing.T) {
	clock := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	repository := NewMemoryStore(func() time.Time { return clock })
	if err := repository.Run(context.Background(), func(state *State) error {
		state.IdempotencyRecords["old"] = IdempotencyRecord{Fingerprint: "old", ResourceID: "resource-old"}
		return nil
	}); err != nil {
		t.Fatalf("seed old idempotency record: %v", err)
	}

	clock = clock.Add(48 * time.Hour)
	cutoff := clock.Add(-24 * time.Hour)
	if err := repository.Run(context.Background(), func(state *State) error {
		state.IdempotencyRecords["new"] = IdempotencyRecord{Fingerprint: "new", ResourceID: "resource-new"}
		state.ModelPoolTestResults["old"] = controlplane.ModelPoolConnectivityTestResult{TestedAt: cutoff.Add(-time.Hour).Format(time.RFC3339)}
		state.ModelPoolTestResults["new"] = controlplane.ModelPoolConnectivityTestResult{TestedAt: cutoff.Add(time.Hour).Format(time.RFC3339)}
		state.AuditLogs["old"] = controlplane.AuditLog{ID: "old", CreatedAt: cutoff.Add(-time.Hour).Format(time.RFC3339)}
		state.AuditLogs["new"] = controlplane.AuditLog{ID: "new", CreatedAt: cutoff.Add(time.Hour).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed retention records: %v", err)
	}

	request := RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 1}
	cleanups := []struct {
		name string
		run  func(context.Context, RetentionCleanupRequest) (int64, error)
	}{
		{name: "idempotency", run: repository.CleanupIdempotencyRecords},
		{name: "model tests", run: repository.CleanupModelPoolTestResults},
		{name: "audit logs", run: repository.CleanupAuditLogs},
	}
	for _, cleanup := range cleanups {
		t.Run(cleanup.name, func(t *testing.T) {
			deleted, err := cleanup.run(context.Background(), request)
			if err != nil {
				t.Fatalf("cleanup error = %v", err)
			}
			if deleted != 1 {
				t.Fatalf("deleted = %d, want 1", deleted)
			}
		})
	}

	if err := repository.Run(context.Background(), func(state *State) error {
		if _, ok := state.IdempotencyRecords["old"]; ok {
			t.Error("old idempotency record was retained")
		}
		if _, ok := state.IdempotencyRecords["new"]; !ok {
			t.Error("new idempotency record was removed")
		}
		if _, ok := state.ModelPoolTestResults["old"]; ok {
			t.Error("old model test was retained")
		}
		if _, ok := state.ModelPoolTestResults["new"]; !ok {
			t.Error("new model test was removed")
		}
		if _, ok := state.AuditLogs["old"]; ok {
			t.Error("old audit log was retained")
		}
		if _, ok := state.AuditLogs["new"]; !ok {
			t.Error("new audit log was removed")
		}
		return nil
	}); err != nil {
		t.Fatalf("inspect retention records: %v", err)
	}
}

func TestMemoryStoreRetentionCleanupIsCancellable(t *testing.T) {
	repository := NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *State) error {
		state.AuditLogs["old"] = controlplane.AuditLog{ID: "old", CreatedAt: "2026-01-01T00:00:00Z"}
		return nil
	}); err != nil {
		t.Fatalf("seed audit log: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	deleted, err := repository.CleanupAuditLogs(ctx, RetentionCleanupRequest{Cutoff: time.Now().UTC(), BatchSize: 10})
	if !errors.Is(err, context.Canceled) || deleted != 0 {
		t.Fatalf("CleanupAuditLogs() = (%d, %v), want (0, context canceled)", deleted, err)
	}
}

func TestRetentionCleanupRejectsInvalidBounds(t *testing.T) {
	repository := NewMemoryStore(time.Now)
	for _, request := range []RetentionCleanupRequest{
		{BatchSize: 1},
		{Cutoff: time.Now(), BatchSize: 0},
		{Cutoff: time.Now(), BatchSize: MaxRetentionCleanupBatchSize + 1},
	} {
		if _, err := repository.CleanupAuditLogs(context.Background(), request); err == nil {
			t.Fatalf("CleanupAuditLogs(%+v) error = nil", request)
		}
	}
}
