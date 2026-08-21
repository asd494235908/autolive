package service

import "time"

// RetentionCleanupRunStatus is a fixed outcome label for one cleanup pass.
type RetentionCleanupRunStatus string

const (
	RetentionCleanupRunCompleted RetentionCleanupRunStatus = "completed"
	RetentionCleanupRunFailed    RetentionCleanupRunStatus = "failed"
	RetentionCleanupRunCancelled RetentionCleanupRunStatus = "cancelled"
	RetentionCleanupRunTimedOut  RetentionCleanupRunStatus = "timeout"
	RetentionCleanupRunUnknown   RetentionCleanupRunStatus = "unknown"
)

// RetentionCleanupDataset is deliberately limited to the supported
// record families; it must never contain a table name or caller input.
type RetentionCleanupDataset string

const (
	RetentionCleanupDatasetAuthSessions           RetentionCleanupDataset = "auth_sessions"
	RetentionCleanupDatasetOrphanedDeviceBindings RetentionCleanupDataset = "orphaned_device_bindings"
	RetentionCleanupDatasetStagedSecrets          RetentionCleanupDataset = "staged_secrets"
	RetentionCleanupDatasetIdempotencyRecords     RetentionCleanupDataset = "idempotency_records"
	RetentionCleanupDatasetModelTestResults       RetentionCleanupDataset = "model_test_results"
	RetentionCleanupDatasetAuditLogs              RetentionCleanupDataset = "audit_logs"
	RetentionCleanupDatasetAuditOutbox            RetentionCleanupDataset = "audit_outbox"
	RetentionCleanupDatasetUnknown                RetentionCleanupDataset = "unknown"
)

// RetentionCleanupDatasetStatus is a fixed outcome label for one dataset.
type RetentionCleanupDatasetStatus string

const (
	RetentionCleanupDatasetCompleted     RetentionCleanupDatasetStatus = "completed"
	RetentionCleanupDatasetFailed        RetentionCleanupDatasetStatus = "failed"
	RetentionCleanupDatasetSkipped       RetentionCleanupDatasetStatus = "skipped"
	RetentionCleanupDatasetCancelled     RetentionCleanupDatasetStatus = "cancelled"
	RetentionCleanupDatasetTimedOut      RetentionCleanupDatasetStatus = "timeout"
	RetentionCleanupDatasetStatusUnknown RetentionCleanupDatasetStatus = "unknown"
)

// RetentionCleanupTelemetry is the observability boundary for the scheduler.
// Implementations must keep dataset and status labels bounded and must not
// include record IDs, request IDs, URLs, error text or secrets.
type RetentionCleanupTelemetry interface {
	ObserveRun(status RetentionCleanupRunStatus, duration time.Duration)
	ObserveDataset(dataset RetentionCleanupDataset, status RetentionCleanupDatasetStatus, deleted int64)
}

type discardRetentionCleanupTelemetry struct{}

func (discardRetentionCleanupTelemetry) ObserveRun(RetentionCleanupRunStatus, time.Duration) {}
func (discardRetentionCleanupTelemetry) ObserveDataset(RetentionCleanupDataset, RetentionCleanupDatasetStatus, int64) {
}
