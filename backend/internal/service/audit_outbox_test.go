package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedAuditOutboxStub struct {
	*store.MemoryStore
	called     bool
	input      controlplane.AuditLogInput
	dispatches int
}

func (s *normalizedAuditOutboxStub) UsesNormalizedReadSource() bool { return true }

func (s *normalizedAuditOutboxStub) RecordAuditWithOutbox(_ context.Context, input controlplane.AuditLogInput) error {
	s.called = true
	s.input = input
	return nil
}

func (s *normalizedAuditOutboxStub) DispatchAuditOutbox(_ context.Context, batchSize int) (int64, error) {
	s.dispatches = batchSize
	return 2, nil
}

func (s *normalizedAuditOutboxStub) RecordAudit(_ context.Context, _ controlplane.AuditLogInput) error {
	return nil
}

func TestRecordAuditPrefersNormalizedOutbox(t *testing.T) {
	repository := &normalizedAuditOutboxStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	if err := svc.RecordAudit(context.Background(), controlplane.AuditLogInput{Action: "user.update", TargetType: "user", Outcome: "success", StatusCode: 200}); err != nil {
		t.Fatalf("RecordAudit() error = %v", err)
	}
	if !repository.called {
		t.Fatal("RecordAudit() did not use normalized audit outbox")
	}
}

func TestRetentionCleanupSchedulerDispatchesAuditOutbox(t *testing.T) {
	dispatches := 0
	scheduler, err := NewRetentionCleanupScheduler(nil, nil, RetentionCleanupSchedulerOptions{
		BatchSize: 7,
		AuditOutboxDispatcher: func(_ context.Context, batchSize int) (int64, error) {
			dispatches = batchSize
			return 3, nil
		},
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour,
		},
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	summary, err := scheduler.RunOnce(context.Background())
	if err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if dispatches != 7 || summary.AuditOutboxDispatched != 3 {
		t.Fatalf("dispatches = %d summary = %+v, want batch 7 and count 3", dispatches, summary)
	}
}
