package service

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

func TestRetentionCleanupSchedulerRunsAllDatasetsWithBoundedCutoffs(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	cleaner := &recordingRetentionCleaner{}
	telemetry := &recordingRetentionTelemetry{}
	scheduler, err := NewRetentionCleanupScheduler(cleaner, cleaner, RetentionCleanupSchedulerOptions{
		OperationTimeout: time.Second,
		BatchSize:        23,
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL:       24 * time.Hour,
			AuthThrottleTTL:      36 * time.Hour,
			IdempotencyRecordTTL: 48 * time.Hour,
			ModelTestResultTTL:   72 * time.Hour,
			AuditLogTTL:          96 * time.Hour,
		},
		Now:       func() time.Time { return now },
		Telemetry: telemetry,
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	summary, err := scheduler.RunOnce(context.Background())
	if err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if summary != (RetentionCleanupSummary{AuthSessions: 1, AuthThrottleBuckets: 5, IdempotencyRecords: 2, ModelTestResults: 3, AuditLogs: 4}) {
		t.Fatalf("summary = %+v", summary)
	}
	cleaner.mu.Lock()
	defer cleaner.mu.Unlock()
	if len(cleaner.requests) != 5 {
		t.Fatalf("cleanup requests = %+v, want 5", cleaner.requests)
	}
	want := map[string]time.Time{
		"auth_sessions":         now.Add(-24 * time.Hour),
		"auth_throttle_buckets": now.Add(-36 * time.Hour),
		"idempotency_records":   now.Add(-48 * time.Hour),
		"model_test_results":    now.Add(-72 * time.Hour),
		"audit_logs":            now.Add(-96 * time.Hour),
	}
	for _, request := range cleaner.requests {
		if request.request.BatchSize != 23 {
			t.Fatalf("%s batch size = %d", request.name, request.request.BatchSize)
		}
		if !request.request.Cutoff.Equal(want[request.name]) {
			t.Fatalf("%s cutoff = %s, want %s", request.name, request.request.Cutoff, want[request.name])
		}
	}
	telemetry.mu.Lock()
	defer telemetry.mu.Unlock()
	if telemetry.runStatus != RetentionCleanupRunCompleted || telemetry.runDuration < 0 {
		t.Fatalf("run telemetry = %+v", telemetry)
	}
	for _, dataset := range []RetentionCleanupDataset{RetentionCleanupDatasetAuthSessions, RetentionCleanupDatasetAuthThrottleBuckets, RetentionCleanupDatasetIdempotencyRecords, RetentionCleanupDatasetModelTestResults, RetentionCleanupDatasetAuditLogs} {
		if telemetry.datasetStatus[dataset] != RetentionCleanupDatasetCompleted {
			t.Fatalf("dataset %q telemetry = %+v", dataset, telemetry.datasetStatus)
		}
	}
}

func TestRetentionCleanupSchedulerSkipsSnapshotBoundaryButContinues(t *testing.T) {
	cleaner := &recordingRetentionCleaner{controlPlaneError: store.ErrNormalizedRetentionCleanupRequired}
	scheduler, err := NewRetentionCleanupScheduler(cleaner, nil, RetentionCleanupSchedulerOptions{
		BatchSize: 1,
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL:       time.Hour,
			AuthThrottleTTL:      time.Hour,
			IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL:   time.Hour,
			AuditLogTTL:          time.Hour,
		},
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	if summary, err := scheduler.RunOnce(context.Background()); err != nil || summary != (RetentionCleanupSummary{}) {
		t.Fatalf("RunOnce() = (%+v, %v), want empty summary and no boundary error", summary, err)
	}
}

func TestRetentionCleanupSchedulerPreservesCancellation(t *testing.T) {
	cleaner := &recordingRetentionCleaner{controlPlaneError: context.Canceled}
	scheduler, err := NewRetentionCleanupScheduler(cleaner, nil, RetentionCleanupSchedulerOptions{
		BatchSize: 1,
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL:       time.Hour,
			AuthThrottleTTL:      time.Hour,
			IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL:   time.Hour,
			AuditLogTTL:          time.Hour,
		},
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	if _, err := scheduler.RunOnce(context.Background()); !errors.Is(err, context.Canceled) {
		t.Fatalf("RunOnce() error = %v, want context canceled", err)
	}
}

func TestRetentionCleanupSchedulerRecordsTimeout(t *testing.T) {
	cleaner := &recordingRetentionCleaner{controlPlaneError: context.DeadlineExceeded}
	telemetry := &recordingRetentionTelemetry{}
	scheduler, err := NewRetentionCleanupScheduler(cleaner, nil, RetentionCleanupSchedulerOptions{
		BatchSize: 1,
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour,
		},
		Telemetry: telemetry,
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	if _, err := scheduler.RunOnce(context.Background()); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("RunOnce() error = %v, want deadline exceeded", err)
	}
	telemetry.mu.Lock()
	defer telemetry.mu.Unlock()
	if telemetry.runStatus != RetentionCleanupRunTimedOut {
		t.Fatalf("run status = %q, want timeout", telemetry.runStatus)
	}
	if telemetry.datasetStatus[RetentionCleanupDatasetIdempotencyRecords] != RetentionCleanupDatasetTimedOut {
		t.Fatalf("dataset status = %q, want timeout", telemetry.datasetStatus[RetentionCleanupDatasetIdempotencyRecords])
	}
}

func TestRetentionCleanupSchedulerReconcilesOrphanedDeviceBindings(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	cleaner := &bindingRecordingCleaner{}
	scheduler, err := NewRetentionCleanupScheduler(nil, cleaner, RetentionCleanupSchedulerOptions{
		BatchSize: 7,
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour,
		},
		Now: func() time.Time { return now },
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	summary, err := scheduler.RunOnce(context.Background())
	if err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if summary.OrphanedDeviceBindings != 3 {
		t.Fatalf("orphaned binding summary = %d, want 3", summary.OrphanedDeviceBindings)
	}
	if !cleaner.request.Cutoff.Equal(now.Add(-orphanedDeviceBindingGrace)) || cleaner.request.BatchSize != 7 {
		t.Fatalf("orphan binding request = %+v", cleaner.request)
	}
}

func TestRetentionCleanupSchedulerCleansStagedSecrets(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	var request store.RetentionCleanupRequest
	scheduler, err := NewRetentionCleanupScheduler(nil, nil, RetentionCleanupSchedulerOptions{
		BatchSize: 5,
		StagedSecretCleaner: func(_ context.Context, got store.RetentionCleanupRequest) (int64, error) {
			request = got
			return 2, nil
		},
		Policy: RetentionCleanupPolicy{
			AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour,
		},
		Now: func() time.Time { return now },
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	summary, err := scheduler.RunOnce(context.Background())
	if err != nil || summary.StagedSecrets != 2 {
		t.Fatalf("RunOnce() = (%+v, %v), want staged secret count 2", summary, err)
	}
	if !request.Cutoff.Equal(now.Add(-stagedSecretCleanupGrace)) || request.BatchSize != 5 {
		t.Fatalf("staged secret request = %+v", request)
	}
}

func TestRetentionCleanupSchedulerLifecycle(t *testing.T) {
	cleaner := &recordingRetentionCleaner{}
	scheduler, err := NewRetentionCleanupScheduler(cleaner, nil, RetentionCleanupSchedulerOptions{
		Interval:         10 * time.Millisecond,
		OperationTimeout: time.Second,
		BatchSize:        1,
		Policy:           RetentionCleanupPolicy{AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour, ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour},
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	if err := scheduler.Start(context.Background()); err != nil {
		t.Fatalf("Start() error = %v", err)
	}
	stopCtx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err := scheduler.Stop(stopCtx); err != nil {
		t.Fatalf("Stop() error = %v", err)
	}
	if err := scheduler.Stop(stopCtx); err != nil {
		t.Fatalf("second Stop() error = %v", err)
	}
}

func TestRetentionCleanupSchedulerRejectsInvalidOptions(t *testing.T) {
	policy := RetentionCleanupPolicy{AuthSessionTTL: time.Hour, AuthThrottleTTL: time.Hour, IdempotencyRecordTTL: time.Hour, ModelTestResultTTL: time.Hour, AuditLogTTL: time.Hour}
	if _, err := NewRetentionCleanupScheduler(nil, nil, RetentionCleanupSchedulerOptions{Policy: policy}); err == nil {
		t.Fatal("nil cleaners accepted")
	}
	if _, err := NewRetentionCleanupScheduler(&recordingRetentionCleaner{}, nil, RetentionCleanupSchedulerOptions{BatchSize: store.MaxRetentionCleanupBatchSize + 1, Policy: policy}); err == nil {
		t.Fatal("oversized batch accepted")
	}
	if _, err := NewRetentionCleanupScheduler(&recordingRetentionCleaner{}, nil, RetentionCleanupSchedulerOptions{BatchSize: 1, Policy: RetentionCleanupPolicy{AuthSessionTTL: time.Hour}}); err == nil {
		t.Fatal("incomplete policy accepted")
	}
}

type retentionRequestRecord struct {
	name    string
	request store.RetentionCleanupRequest
}

type recordingRetentionCleaner struct {
	mu                sync.Mutex
	requests          []retentionRequestRecord
	controlPlaneError error
}

type recordingRetentionTelemetry struct {
	mu            sync.Mutex
	runStatus     RetentionCleanupRunStatus
	runDuration   time.Duration
	datasetStatus map[RetentionCleanupDataset]RetentionCleanupDatasetStatus
}

func (r *recordingRetentionTelemetry) ObserveRun(status RetentionCleanupRunStatus, duration time.Duration) {
	r.mu.Lock()
	defer r.mu.Unlock()
	runStatus := status
	runDuration := duration
	r.runStatus = runStatus
	r.runDuration = runDuration
}

func (r *recordingRetentionTelemetry) ObserveDataset(dataset RetentionCleanupDataset, status RetentionCleanupDatasetStatus, _ int64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.datasetStatus == nil {
		r.datasetStatus = make(map[RetentionCleanupDataset]RetentionCleanupDatasetStatus)
	}
	r.datasetStatus[dataset] = status
}

func (r *recordingRetentionCleaner) CleanupAuthSessions(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.record("auth_sessions", request)
	return 1, nil
}

func (r *recordingRetentionCleaner) CleanupLoginThrottleBuckets(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.record("auth_throttle_buckets", request)
	if r.controlPlaneError != nil {
		return 0, r.controlPlaneError
	}
	return 5, r.controlPlaneError
}

func (r *recordingRetentionCleaner) CleanupIdempotencyRecords(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.record("idempotency_records", request)
	if r.controlPlaneError != nil {
		return 0, r.controlPlaneError
	}
	return 2, r.controlPlaneError
}

func (r *recordingRetentionCleaner) CleanupModelPoolTestResults(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.record("model_test_results", request)
	if r.controlPlaneError != nil {
		return 0, r.controlPlaneError
	}
	return 3, r.controlPlaneError
}

func (r *recordingRetentionCleaner) CleanupAuditLogs(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.record("audit_logs", request)
	if r.controlPlaneError != nil {
		return 0, r.controlPlaneError
	}
	return 4, r.controlPlaneError
}

func (r *recordingRetentionCleaner) record(name string, request store.RetentionCleanupRequest) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.requests = append(r.requests, retentionRequestRecord{name: name, request: request})
}

type bindingRecordingCleaner struct {
	request store.RetentionCleanupRequest
}

func (r *bindingRecordingCleaner) CleanupAuthSessions(context.Context, store.RetentionCleanupRequest) (int64, error) {
	return 0, nil
}

func (r *bindingRecordingCleaner) CleanupOrphanedDeviceBindings(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	r.request = request
	return 3, nil
}
