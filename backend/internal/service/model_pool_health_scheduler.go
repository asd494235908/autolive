package service

import (
	"context"
	"errors"
	"fmt"
	"hash/fnv"
	"io"
	"log/slog"
	"math"
	"sync"
	"sync/atomic"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// ModelPoolHealthProbeSchedulerOptions bounds the background health worker.
// The scheduler uses a repository advisory lock when the deployment provides
// one, while retaining a single-process fallback for MemoryStore/tests.
type ModelPoolHealthProbeSchedulerOptions struct {
	Interval          time.Duration
	ProbeTimeout      time.Duration
	MaxConcurrent     int
	MaxAccountsPerRun int
	BackoffBase       time.Duration
	BackoffMax        time.Duration
	Logger            *slog.Logger
	Now               func() time.Time
	Telemetry         ModelPoolHealthProbeTelemetry
}

const (
	defaultModelPoolHealthProbeTimeout           = 10 * time.Second
	defaultModelPoolHealthProbeConcurrency       = 2
	defaultModelPoolHealthProbeMaxAccounts       = 50
	defaultModelPoolHealthProbeBackoffBase       = 30 * time.Second
	defaultModelPoolHealthProbeBackoffMax        = 15 * time.Minute
	modelPoolHealthProbeAdvisoryLockKey    int64 = 0x6175746f6c697665
)

type modelPoolProbeBackoff struct {
	failures int
	nextAt   time.Time
}

// ModelPoolHealthProbeScheduler owns exactly one loop and all probe
// goroutines. Stop cancels the loop and waits for every in-flight probe.
type ModelPoolHealthProbeScheduler struct {
	controlPlane  *ControlPlane
	interval      time.Duration
	probeTimeout  time.Duration
	maxConcurrent int
	maxAccounts   int
	backoffBase   time.Duration
	backoffMax    time.Duration
	logger        *slog.Logger
	now           func() time.Time
	telemetry     ModelPoolHealthProbeTelemetry

	mu       sync.Mutex
	backoffs map[string]modelPoolProbeBackoff
	cancel   context.CancelFunc
	done     chan struct{}
	started  bool
	sequence atomic.Uint64
}

func NewModelPoolHealthProbeScheduler(controlPlane *ControlPlane, options ModelPoolHealthProbeSchedulerOptions) (*ModelPoolHealthProbeScheduler, error) {
	if controlPlane == nil {
		return nil, fmt.Errorf("control plane must not be nil")
	}
	if options.ProbeTimeout == 0 {
		options.ProbeTimeout = defaultModelPoolHealthProbeTimeout
	}
	if options.MaxConcurrent == 0 {
		options.MaxConcurrent = defaultModelPoolHealthProbeConcurrency
	}
	if options.MaxAccountsPerRun == 0 {
		options.MaxAccountsPerRun = defaultModelPoolHealthProbeMaxAccounts
	}
	if options.BackoffBase == 0 {
		options.BackoffBase = defaultModelPoolHealthProbeBackoffBase
	}
	if options.BackoffMax == 0 {
		options.BackoffMax = defaultModelPoolHealthProbeBackoffMax
	}
	if options.Interval < 0 || options.ProbeTimeout <= 0 || options.MaxConcurrent < 1 || options.MaxAccountsPerRun < 1 || options.BackoffBase <= 0 || options.BackoffMax < options.BackoffBase {
		return nil, fmt.Errorf("invalid model pool health probe scheduler options")
	}
	if options.Logger == nil {
		options.Logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	if options.Now == nil {
		options.Now = time.Now
	}
	if options.Telemetry == nil {
		options.Telemetry = discardModelPoolHealthProbeTelemetry{}
	}
	return &ModelPoolHealthProbeScheduler{
		controlPlane:  controlPlane,
		interval:      options.Interval,
		probeTimeout:  options.ProbeTimeout,
		maxConcurrent: options.MaxConcurrent,
		maxAccounts:   options.MaxAccountsPerRun,
		backoffBase:   options.BackoffBase,
		backoffMax:    options.BackoffMax,
		logger:        options.Logger,
		now:           options.Now,
		telemetry:     options.Telemetry,
		backoffs:      make(map[string]modelPoolProbeBackoff),
	}, nil
}

// Start begins the worker. An interval of zero creates a disabled scheduler
// so callers can keep lifecycle wiring identical across environments.
func (s *ModelPoolHealthProbeScheduler) Start(parent context.Context) error {
	if parent == nil {
		parent = context.Background()
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.started {
		return fmt.Errorf("model pool health probe scheduler already started")
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

// Stop is cancellation-aware and waits for the loop plus all probes to exit.
func (s *ModelPoolHealthProbeScheduler) Stop(ctx context.Context) error {
	if ctx == nil {
		ctx = context.Background()
	}
	s.mu.Lock()
	if !s.started {
		s.mu.Unlock()
		return nil
	}
	cancel := s.cancel
	done := s.done
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

func (s *ModelPoolHealthProbeScheduler) loop(ctx context.Context) {
	defer close(s.done)
	ticker := time.NewTicker(s.interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			if err := s.RunOnce(ctx); err != nil && ctx.Err() == nil {
				s.logger.Warn("model pool health probe run failed", "error", err)
			}
		}
	}
}

// RunOnce performs one bounded pass. It is exported for startup smoke tests
// and controlled maintenance tooling; individual provider failures are stored
// as account health results and do not fail the whole pass.
func (s *ModelPoolHealthProbeScheduler) RunOnce(ctx context.Context) (runErr error) {
	if ctx == nil {
		ctx = context.Background()
	}
	startedAt := time.Now()
	defer func() {
		s.telemetry.ObserveRun(modelPoolHealthProbeRunStatus(runErr, ctx), time.Since(startedAt))
	}()
	release, acquired, err := s.controlPlane.TryAdvisoryLock(ctx, modelPoolHealthProbeAdvisoryLockKey)
	if err != nil {
		return err
	}
	if !acquired {
		s.telemetry.ObserveSkip(ModelPoolHealthProbeSkipDistributedLock)
		return nil
	}
	defer func() {
		releaseErr := release(context.WithoutCancel(ctx))
		if releaseErr != nil && runErr == nil && ctx.Err() == nil {
			runErr = fmt.Errorf("release model pool health advisory lock: %w", releaseErr)
		}
	}()
	var accounts []controlplane.ModelPoolAccountSummary
	if source, ok := s.controlPlane.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		// Health checks must not materialize the full compatibility snapshot. The
		// normalized reader also filters ineligible rows before applying the limit,
		// so an inactive prefix cannot hide later probe candidates.
		var pageErr error
		accounts, pageErr = s.controlPlane.ListModelPoolHealthAccounts(ctx, s.maxAccounts)
		if pageErr != nil {
			return pageErr
		}
	} else {
		// Memory/snapshot repositories retain their existing compatibility
		// behavior; their state is already the authoritative local projection.
		var listErr error
		accounts, listErr = s.controlPlane.ListModelPoolAccounts(ctx)
		if listErr != nil {
			return listErr
		}
	}
	now := s.now().UTC()
	selected := make([]controlplane.ModelPoolAccountSummary, 0, minInt(len(accounts), s.maxAccounts))
	for _, account := range accounts {
		if len(selected) >= s.maxAccounts {
			s.telemetry.ObserveSkip(ModelPoolHealthProbeSkipLimit)
			continue
		}
		if account.Status != controlplane.ModelAccountStatusActive {
			s.telemetry.ObserveSkip(ModelPoolHealthProbeSkipInactive)
			continue
		}
		if account.SecretRef == "" {
			s.telemetry.ObserveSkip(ModelPoolHealthProbeSkipMissingSecret)
			continue
		}
		if !s.readyForProbe(account.ID, now) {
			s.telemetry.ObserveSkip(ModelPoolHealthProbeSkipBackoff)
			continue
		}
		selected = append(selected, account)
	}
	if len(selected) == 0 {
		return nil
	}
	semaphore := make(chan struct{}, s.maxConcurrent)
	var wait sync.WaitGroup
	for _, account := range selected {
		if err := ctx.Err(); err != nil {
			break
		}
		account := account
		acquired := false
		select {
		case semaphore <- struct{}{}:
			acquired = true
		case <-ctx.Done():
		}
		if !acquired {
			break
		}
		wait.Add(1)
		go func() {
			defer wait.Done()
			defer func() { <-semaphore }()
			s.probeOne(ctx, account)
		}()
	}
	wait.Wait()
	return ctx.Err()
}

func (s *ModelPoolHealthProbeScheduler) probeOne(parent context.Context, account controlplane.ModelPoolAccountSummary) {
	probeCtx, cancel := context.WithTimeout(parent, s.probeTimeout)
	defer cancel()
	sequence := s.sequence.Add(1)
	idempotencyKey := fmt.Sprintf("health-probe-%s-%d", account.ID, sequence)
	result, err := s.controlPlane.TestModelPoolAccount(probeCtx, idempotencyKey, account.ID, controlplane.TestModelPoolAccountInput{
		TimeoutSeconds: timeoutSeconds(s.probeTimeout),
	})
	if err != nil {
		if parent.Err() != nil {
			if errors.Is(parent.Err(), context.DeadlineExceeded) {
				s.telemetry.ObserveProbe(ModelPoolHealthProbeTimedOut)
			} else {
				s.telemetry.ObserveProbe(ModelPoolHealthProbeCancelled)
			}
			return
		}
		if errors.Is(err, context.DeadlineExceeded) {
			s.recordFailure(account.ID)
			s.telemetry.ObserveProbe(ModelPoolHealthProbeTimedOut)
			return
		}
		s.recordFailure(account.ID)
		s.telemetry.ObserveProbe(ModelPoolHealthProbeFailed)
		s.logger.Warn("model pool health probe failed", "account_id", account.ID, "error", err)
		return
	}
	if parent.Err() != nil {
		if errors.Is(parent.Err(), context.DeadlineExceeded) {
			s.telemetry.ObserveProbe(ModelPoolHealthProbeTimedOut)
		} else {
			s.telemetry.ObserveProbe(ModelPoolHealthProbeCancelled)
		}
		return
	}
	if result.Status == "succeeded" {
		s.recordSuccess(account.ID)
		s.telemetry.ObserveProbe(ModelPoolHealthProbeSucceeded)
		return
	}
	s.recordFailure(account.ID)
	if result.Status == "timeout" {
		s.telemetry.ObserveProbe(ModelPoolHealthProbeTimedOut)
		return
	}
	s.telemetry.ObserveProbe(ModelPoolHealthProbeUnhealthy)
	s.logger.Warn("model pool health probe unhealthy", "account_id", account.ID, "status", result.Status, "error_code", result.ErrorCode)
}

func modelPoolHealthProbeRunStatus(err error, ctx context.Context) ModelPoolHealthProbeRunStatus {
	if err == nil {
		return ModelPoolHealthProbeRunCompleted
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return ModelPoolHealthProbeRunTimedOut
	}
	if ctx != nil && ctx.Err() != nil {
		return ModelPoolHealthProbeRunCancelled
	}
	return ModelPoolHealthProbeRunFailed
}

func (s *ModelPoolHealthProbeScheduler) readyForProbe(accountID string, now time.Time) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	state := s.backoffs[accountID]
	return state.nextAt.IsZero() || !now.Before(state.nextAt)
}

func (s *ModelPoolHealthProbeScheduler) recordSuccess(accountID string) {
	s.mu.Lock()
	delete(s.backoffs, accountID)
	s.mu.Unlock()
}

func (s *ModelPoolHealthProbeScheduler) recordFailure(accountID string) {
	now := s.now().UTC()
	s.mu.Lock()
	state := s.backoffs[accountID]
	state.failures++
	state.nextAt = now.Add(jitteredProbeBackoff(accountID, state.failures, s.backoffBase, s.backoffMax))
	s.backoffs[accountID] = state
	s.mu.Unlock()
}

func jitteredProbeBackoff(accountID string, failures int, base, maximum time.Duration) time.Duration {
	if failures < 1 {
		failures = 1
	}
	shift := failures - 1
	if shift > 30 {
		shift = 30
	}
	backoff := base * time.Duration(uint64(1)<<shift)
	if backoff <= 0 || backoff > maximum {
		backoff = maximum
	}
	hash := fnv.New32a()
	_, _ = hash.Write([]byte(fmt.Sprintf("%s/%d", accountID, failures)))
	// Deterministic per account/attempt jitter keeps tests reproducible while
	// preventing a fleet of accounts from retrying on exactly one instant.
	percent := 80 + int(hash.Sum32()%41)
	return time.Duration(math.Round(float64(backoff) * float64(percent) / 100))
}

func timeoutSeconds(timeout time.Duration) int {
	seconds := int(math.Ceil(timeout.Seconds()))
	if seconds < 1 {
		return 1
	}
	if seconds > 60 {
		return 60
	}
	return seconds
}

func minInt(left, right int) int {
	if left < right {
		return left
	}
	return right
}
