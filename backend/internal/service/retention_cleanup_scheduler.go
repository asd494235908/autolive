package service

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"sync"
	"time"

	"autoLive/backend/internal/store"
)

// RetentionCleanupPolicy defines how long each control-plane record remains
// available before the bounded cleanup worker removes it.
type RetentionCleanupPolicy struct {
	AuthSessionTTL       time.Duration
	IdempotencyRecordTTL time.Duration
	ModelTestResultTTL   time.Duration
	AuditLogTTL          time.Duration
}

func (p RetentionCleanupPolicy) validate() error {
	if p.AuthSessionTTL <= 0 || p.IdempotencyRecordTTL <= 0 || p.ModelTestResultTTL <= 0 || p.AuditLogTTL <= 0 {
		return fmt.Errorf("retention TTLs must be positive")
	}
	return nil
}

// RetentionCleanupSchedulerOptions bounds one cleanup pass and its lifecycle.
// An interval of zero disables the worker while preserving the same wiring.
type RetentionCleanupSchedulerOptions struct {
	Interval              time.Duration
	OperationTimeout      time.Duration
	BatchSize             int
	Policy                RetentionCleanupPolicy
	StagedSecretCleaner   func(context.Context, store.RetentionCleanupRequest) (int64, error)
	AuditOutboxDispatcher func(context.Context, int) (int64, error)
	Logger                *slog.Logger
	Now                   func() time.Time
	Telemetry             RetentionCleanupTelemetry
}

const defaultRetentionCleanupOperationTimeout = 30 * time.Second

// A binding must remain old for this window before reconciliation. The
// request/transaction timeouts are far shorter, so this protects in-flight
// activation and heartbeat requests without making recovery depend on the
// long auth-session retention TTL.
const orphanedDeviceBindingGrace = 5 * time.Minute

// Staged rotation candidates are independent from long audit retention. A
// one-hour grace window covers normal retries while bounding crash leftovers.
const stagedSecretCleanupGrace = time.Hour

// RetentionCleanupSummary reports only aggregate deletion counts. It never
// contains record IDs, request IDs, URLs, error text or secrets.
type RetentionCleanupSummary struct {
	AuthSessions           int64
	OrphanedDeviceBindings int64
	StagedSecrets          int64
	IdempotencyRecords     int64
	ModelTestResults       int64
	AuditLogs              int64
	AuditOutboxDispatched  int64
}

// RetentionCleanupScheduler owns one loop and serializes cleanup passes. A
// second RunOnce waits for the in-flight pass instead of issuing overlapping
// deletes from the same process.
type RetentionCleanupScheduler struct {
	controlPlaneCleaner   store.ControlPlaneRetentionCleaner
	authSessionCleaner    store.AuthSessionRetentionCleaner
	stagedSecretCleaner   func(context.Context, store.RetentionCleanupRequest) (int64, error)
	auditOutboxDispatcher func(context.Context, int) (int64, error)
	interval              time.Duration
	operationTimeout      time.Duration
	batchSize             int
	policy                RetentionCleanupPolicy
	logger                *slog.Logger
	now                   func() time.Time
	telemetry             RetentionCleanupTelemetry

	mu      sync.Mutex
	cancel  context.CancelFunc
	done    chan struct{}
	started bool
	runMu   sync.Mutex
}

func NewRetentionCleanupScheduler(
	controlPlaneCleaner store.ControlPlaneRetentionCleaner,
	authSessionCleaner store.AuthSessionRetentionCleaner,
	options RetentionCleanupSchedulerOptions,
) (*RetentionCleanupScheduler, error) {
	if controlPlaneCleaner == nil && authSessionCleaner == nil && options.StagedSecretCleaner == nil && options.AuditOutboxDispatcher == nil {
		return nil, errors.New("retention cleanup requires at least one cleaner")
	}
	if options.Interval < 0 || options.OperationTimeout < 0 || options.BatchSize < 0 {
		return nil, errors.New("invalid retention cleanup scheduler options")
	}
	if options.Interval == 0 {
		// A disabled worker still validates its policy when explicitly provided;
		// this prevents typos from being hidden by an environment switch.
		if err := options.Policy.validate(); err != nil {
			return nil, err
		}
	}
	if options.OperationTimeout == 0 {
		options.OperationTimeout = defaultRetentionCleanupOperationTimeout
	}
	if options.BatchSize == 0 {
		options.BatchSize = store.MaxRetentionCleanupBatchSize
	}
	request := store.RetentionCleanupRequest{Cutoff: time.Now(), BatchSize: options.BatchSize}
	if err := request.Validate(); err != nil {
		return nil, err
	}
	if err := options.Policy.validate(); err != nil {
		return nil, err
	}
	if options.Logger == nil {
		options.Logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	if options.Now == nil {
		options.Now = time.Now
	}
	if options.Telemetry == nil {
		options.Telemetry = discardRetentionCleanupTelemetry{}
	}
	return &RetentionCleanupScheduler{
		controlPlaneCleaner:   controlPlaneCleaner,
		authSessionCleaner:    authSessionCleaner,
		stagedSecretCleaner:   options.StagedSecretCleaner,
		auditOutboxDispatcher: options.AuditOutboxDispatcher,
		interval:              options.Interval,
		operationTimeout:      options.OperationTimeout,
		batchSize:             options.BatchSize,
		policy:                options.Policy,
		logger:                options.Logger,
		now:                   options.Now,
		telemetry:             options.Telemetry,
	}, nil
}

// Start begins the owned cleanup loop. Cleanup is intentionally not run at
// startup; the first pass waits for the configured interval to avoid adding a
// database burst to readiness and migration startup.
func (s *RetentionCleanupScheduler) Start(parent context.Context) error {
	if parent == nil {
		parent = context.Background()
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.started {
		return errors.New("retention cleanup scheduler already started")
	}
	s.started = true
	s.done = make(chan struct{})
	if s.interval == 0 {
		close(s.done)
		return nil
	}
	ctx, cancel := context.WithCancel(parent)
	s.cancel = cancel
	go s.loop(ctx)
	return nil
}

// Stop cancels the loop and waits for a running pass to finish.
func (s *RetentionCleanupScheduler) Stop(ctx context.Context) error {
	if ctx == nil {
		ctx = context.Background()
	}
	s.mu.Lock()
	if !s.started {
		s.mu.Unlock()
		return nil
	}
	cancel, done := s.cancel, s.done
	s.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	select {
	case <-done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (s *RetentionCleanupScheduler) loop(ctx context.Context) {
	defer close(s.done)
	ticker := time.NewTicker(s.interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			summary, err := s.RunOnce(ctx)
			if err != nil && ctx.Err() == nil {
				s.logger.Warn("retention cleanup pass failed", "error", err, "auth_sessions", summary.AuthSessions, "orphaned_device_bindings", summary.OrphanedDeviceBindings, "staged_secrets", summary.StagedSecrets, "idempotency_records", summary.IdempotencyRecords, "model_test_results", summary.ModelTestResults, "audit_logs", summary.AuditLogs, "audit_outbox_dispatched", summary.AuditOutboxDispatched)
			}
		}
	}
}

// RunOnce executes all available cleaners under one bounded context. A
// failure in one dataset does not prevent the other datasets from making
// progress; the returned error preserves every failure for the caller.
func (s *RetentionCleanupScheduler) RunOnce(parent context.Context) (summary RetentionCleanupSummary, runErr error) {
	if parent == nil {
		parent = context.Background()
	}
	s.runMu.Lock()
	defer s.runMu.Unlock()
	startedAt := time.Now()
	defer func() {
		s.telemetry.ObserveRun(retentionCleanupRunStatus(runErr, parent), time.Since(startedAt))
	}()
	ctx, cancel := context.WithTimeout(parent, s.operationTimeout)
	defer cancel()
	now := s.now().UTC()
	if s.auditOutboxDispatcher != nil {
		if err := ctx.Err(); err != nil {
			s.telemetry.ObserveDataset(RetentionCleanupDatasetAuditOutbox, RetentionCleanupDatasetCancelled, 0)
			runErr = errors.Join(runErr, err)
		} else {
			dispatched, err := s.auditOutboxDispatcher(ctx, s.batchSize)
			if dispatched > 0 {
				summary.AuditOutboxDispatched += dispatched
			}
			if err != nil {
				if errors.Is(err, store.ErrNormalizedRetentionCleanupRequired) {
					s.telemetry.ObserveDataset(RetentionCleanupDatasetAuditOutbox, RetentionCleanupDatasetSkipped, dispatched)
				} else {
					status := RetentionCleanupDatasetFailed
					if errors.Is(err, context.DeadlineExceeded) {
						status = RetentionCleanupDatasetTimedOut
					} else if errors.Is(err, context.Canceled) {
						status = RetentionCleanupDatasetCancelled
					}
					s.telemetry.ObserveDataset(RetentionCleanupDatasetAuditOutbox, status, dispatched)
					runErr = errors.Join(runErr, fmt.Errorf("%s cleanup: %w", RetentionCleanupDatasetAuditOutbox, err))
				}
			} else {
				s.telemetry.ObserveDataset(RetentionCleanupDatasetAuditOutbox, RetentionCleanupDatasetCompleted, dispatched)
			}
		}
	}
	cleanup := func(dataset RetentionCleanupDataset, ttl time.Duration, fn func(context.Context, store.RetentionCleanupRequest) (int64, error), target *int64) {
		if fn == nil {
			return
		}
		if err := ctx.Err(); err != nil {
			s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetCancelled, 0)
			runErr = errors.Join(runErr, err)
			return
		}
		deleted, err := fn(ctx, store.RetentionCleanupRequest{Cutoff: now.Add(-ttl), BatchSize: s.batchSize})
		if deleted > 0 {
			*target += deleted
		}
		if err != nil {
			// Snapshot-backed repositories intentionally return a typed boundary
			// error. Keep it observable without treating it as a process failure.
			if errors.Is(err, store.ErrNormalizedRetentionCleanupRequired) {
				s.logger.Debug("retention cleanup skipped for snapshot source", "dataset", dataset)
				s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetSkipped, deleted)
				return
			}
			if errors.Is(err, context.DeadlineExceeded) {
				s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetTimedOut, deleted)
			} else if errors.Is(err, context.Canceled) {
				s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetCancelled, deleted)
			} else {
				s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetFailed, deleted)
			}
			runErr = errors.Join(runErr, fmt.Errorf("%s cleanup: %w", dataset, err))
			return
		}
		s.telemetry.ObserveDataset(dataset, RetentionCleanupDatasetCompleted, deleted)
	}
	if s.authSessionCleaner != nil {
		cleanup(RetentionCleanupDatasetAuthSessions, s.policy.AuthSessionTTL, s.authSessionCleaner.CleanupAuthSessions, &summary.AuthSessions)
		if bindingCleaner, ok := s.authSessionCleaner.(store.AuthSessionBindingCleaner); ok {
			cleanup(RetentionCleanupDatasetOrphanedDeviceBindings, orphanedDeviceBindingGrace, bindingCleaner.CleanupOrphanedDeviceBindings, &summary.OrphanedDeviceBindings)
		}
	}
	if s.stagedSecretCleaner != nil {
		cleanup(RetentionCleanupDatasetStagedSecrets, stagedSecretCleanupGrace, s.stagedSecretCleaner, &summary.StagedSecrets)
	}
	if s.controlPlaneCleaner != nil {
		cleanup(RetentionCleanupDatasetIdempotencyRecords, s.policy.IdempotencyRecordTTL, s.controlPlaneCleaner.CleanupIdempotencyRecords, &summary.IdempotencyRecords)
		cleanup(RetentionCleanupDatasetModelTestResults, s.policy.ModelTestResultTTL, s.controlPlaneCleaner.CleanupModelPoolTestResults, &summary.ModelTestResults)
		cleanup(RetentionCleanupDatasetAuditLogs, s.policy.AuditLogTTL, s.controlPlaneCleaner.CleanupAuditLogs, &summary.AuditLogs)
	}
	return summary, runErr
}

func retentionCleanupRunStatus(err error, ctx context.Context) RetentionCleanupRunStatus {
	if err == nil {
		return RetentionCleanupRunCompleted
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return RetentionCleanupRunTimedOut
	}
	if ctx != nil && ctx.Err() != nil {
		return RetentionCleanupRunCancelled
	}
	return RetentionCleanupRunFailed
}
