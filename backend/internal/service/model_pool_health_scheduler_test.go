package service

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestModelPoolHealthProbeSchedulerRunsBoundedProbes(t *testing.T) {
	var requests atomic.Int32
	var active atomic.Int32
	var maximum atomic.Int32
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		current := active.Add(1)
		defer active.Add(-1)
		requests.Add(1)
		for {
			previous := maximum.Load()
			if current <= previous || maximum.CompareAndSwap(previous, current) {
				break
			}
		}
		time.Sleep(20 * time.Millisecond)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := store.NewMemoryStore(time.Now)
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore(), resolver)
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	for index := 1; index <= 3; index++ {
		if _, err := svc.CreateModelPoolAccount(context.Background(), "scheduler-account-"+string(rune('a'+index)), controlplane.CreateModelPoolAccountInput{
			Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
		}); err != nil {
			t.Fatalf("CreateModelPoolAccount(%d) error = %v", index, err)
		}
	}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:          time.Minute,
		ProbeTimeout:      time.Second,
		MaxConcurrent:     2,
		MaxAccountsPerRun: 2,
		BackoffBase:       time.Second,
		BackoffMax:        2 * time.Second,
		Now:               repository.Now,
		Telemetry:         telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if requests.Load() != 2 {
		t.Fatalf("probe requests = %d, want 2", requests.Load())
	}
	if maximum.Load() > 2 {
		t.Fatalf("maximum concurrent probes = %d, want <= 2", maximum.Load())
	}
	accounts, err := svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	var succeeded int
	for _, account := range accounts {
		if account.LastTestStatus == "succeeded" {
			succeeded++
		}
	}
	if succeeded != 2 {
		t.Fatalf("succeeded health accounts = %d, want 2: %+v", succeeded, accounts)
	}
	telemetry.assertRun(t, ModelPoolHealthProbeRunCompleted)
	telemetry.assertProbeCount(t, ModelPoolHealthProbeSucceeded, 2)
	telemetry.assertSkipCount(t, ModelPoolHealthProbeSkipLimit, 1)
}

func TestModelPoolHealthProbeSchedulerBacksOffFailures(t *testing.T) {
	now := time.Date(2026, 8, 21, 20, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	secretStore := store.NewMemorySecretStore()
	svc := NewControlPlaneWithRepositoryAndSecretStore(repository, &http.Client{}, secretStore)
	account, err := svc.CreateModelPoolAccount(context.Background(), "scheduler-backoff-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "https://api.example.com/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	svc.secretStore = failingSecretStore{}
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:     time.Minute,
		ProbeTimeout: time.Second,
		BackoffBase:  time.Minute,
		BackoffMax:   5 * time.Minute,
		Now:          repository.Now,
		Telemetry:    telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("first RunOnce() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("second RunOnce() error = %v", err)
	}
	scheduler.mu.Lock()
	backoff := scheduler.backoffs[account.ID]
	scheduler.mu.Unlock()
	if backoff.failures != 1 || !backoff.nextAt.After(now) {
		t.Fatalf("backoff state = %+v, want one failure in the future", backoff)
	}
	telemetry.assertRunCount(t, 2, ModelPoolHealthProbeRunCompleted)
	telemetry.assertProbeCount(t, ModelPoolHealthProbeFailed, 1)
	telemetry.assertSkipCount(t, ModelPoolHealthProbeSkipBackoff, 1)
}

func TestModelPoolHealthProbeSchedulerTelemetryRecordsUnhealthyResult(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"another-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := store.NewMemoryStore(time.Now)
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore(), resolver)
	if _, err := svc.CreateModelPoolAccount(context.Background(), "scheduler-unhealthy-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:     time.Minute,
		ProbeTimeout: time.Second,
		BackoffBase:  time.Second,
		BackoffMax:   2 * time.Second,
		Now:          repository.Now,
		Telemetry:    telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	telemetry.assertRun(t, ModelPoolHealthProbeRunCompleted)
	telemetry.assertProbeCount(t, ModelPoolHealthProbeUnhealthy, 1)
}

func TestModelPoolHealthProbeSchedulerTelemetryRecordsTimeout(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := store.NewMemoryStore(time.Now)
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore(), resolver)
	if _, err := svc.CreateModelPoolAccount(context.Background(), "scheduler-cancelled-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:     time.Minute,
		ProbeTimeout: time.Second,
		BackoffBase:  time.Second,
		BackoffMax:   2 * time.Second,
		Now:          repository.Now,
		Telemetry:    telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	if err := scheduler.RunOnce(ctx); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("RunOnce() error = %v, want deadline exceeded", err)
	}
	telemetry.assertRun(t, ModelPoolHealthProbeRunTimedOut)
	telemetry.assertProbeCount(t, ModelPoolHealthProbeTimedOut, 1)
}

func TestModelPoolHealthProbeSchedulerTelemetryRecordsCancellation(t *testing.T) {
	started := make(chan struct{})
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		<-r.Context().Done()
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := store.NewMemoryStore(time.Now)
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore(), resolver)
	if _, err := svc.CreateModelPoolAccount(context.Background(), "scheduler-cancelled-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval: time.Minute, ProbeTimeout: time.Second, BackoffBase: time.Second, BackoffMax: 2 * time.Second, Telemetry: telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		<-started
		cancel()
	}()
	if err := scheduler.RunOnce(ctx); !errors.Is(err, context.Canceled) {
		t.Fatalf("RunOnce() error = %v, want context canceled", err)
	}
	telemetry.assertRun(t, ModelPoolHealthProbeRunCancelled)
	telemetry.assertProbeCount(t, ModelPoolHealthProbeCancelled, 1)
}

func TestModelPoolHealthProbeSchedulerStopsOwnedLoop(t *testing.T) {
	svc := NewControlPlane(store.NewMemoryStore(time.Now))
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:     10 * time.Millisecond,
		ProbeTimeout: 50 * time.Millisecond,
		BackoffBase:  10 * time.Millisecond,
		BackoffMax:   100 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	parent, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := scheduler.Start(parent); err != nil {
		t.Fatalf("Start() error = %v", err)
	}
	stopCtx, stopCancel := context.WithTimeout(context.Background(), time.Second)
	defer stopCancel()
	if err := scheduler.Stop(stopCtx); err != nil {
		t.Fatalf("Stop() error = %v", err)
	}
	if err := scheduler.Stop(stopCtx); err != nil {
		t.Fatalf("second Stop() error = %v", err)
	}
}

func TestModelPoolHealthProbeSchedulerRejectsInvalidOptions(t *testing.T) {
	svc := NewControlPlane(store.NewMemoryStore(time.Now))
	if _, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{Interval: -time.Second}); err == nil {
		t.Fatal("negative interval accepted")
	}
	if _, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{BackoffBase: time.Minute, BackoffMax: time.Second}); err == nil {
		t.Fatal("backoff max shorter than base accepted")
	}
}

func TestModelPoolHealthProbeSchedulerSkipsWhenDistributedLockIsBusy(t *testing.T) {
	repository := &busyAdvisoryLockRepository{Repository: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepositoryAndSecretStore(repository, &http.Client{}, store.NewMemorySecretStore())
	telemetry := &recordingModelPoolHealthProbeTelemetry{}
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:  time.Minute,
		Telemetry: telemetry,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	telemetry.assertRun(t, ModelPoolHealthProbeRunCompleted)
	telemetry.assertSkipCount(t, ModelPoolHealthProbeSkipDistributedLock, 1)
	if repository.tryCount != 1 {
		t.Fatalf("TryAdvisoryLock calls = %d, want 1", repository.tryCount)
	}
}

type normalizedHealthPageRepository struct {
	*store.MemoryStore
	pageCalls   atomic.Int32
	healthCalls atomic.Int32
}

func (r *normalizedHealthPageRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedHealthPageRepository) ListModelPoolAccountsPage(context.Context, int, int) (store.ModelPoolPage, error) {
	r.pageCalls.Add(1)
	return store.ModelPoolPage{}, nil
}

func (r *normalizedHealthPageRepository) ListModelPoolHealthAccounts(context.Context, int) ([]controlplane.ModelPoolAccountSummary, error) {
	r.healthCalls.Add(1)
	return nil, nil
}

func TestModelPoolHealthProbeSchedulerUsesBoundedNormalizedReader(t *testing.T) {
	repository := &normalizedHealthPageRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	scheduler, err := NewModelPoolHealthProbeScheduler(svc, ModelPoolHealthProbeSchedulerOptions{
		Interval:          time.Minute,
		ProbeTimeout:      time.Second,
		MaxAccountsPerRun: 3,
		BackoffBase:       time.Second,
		BackoffMax:        2 * time.Second,
	})
	if err != nil {
		t.Fatalf("NewModelPoolHealthProbeScheduler() error = %v", err)
	}
	if err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if calls := repository.healthCalls.Load(); calls != 1 {
		t.Fatalf("normalized health page reader calls = %d, want 1", calls)
	}
	if calls := repository.pageCalls.Load(); calls != 0 {
		t.Fatalf("generic normalized page reader calls = %d, want 0", calls)
	}
}

type normalizedModelPoolListRepository struct {
	now  time.Time
	page store.ModelPoolPage
}

func (r *normalizedModelPoolListRepository) Now() time.Time { return r.now }

func (r *normalizedModelPoolListRepository) Run(context.Context, store.StateOperation) error {
	return errors.New("normalized model pool list must not use snapshot state")
}

func (r *normalizedModelPoolListRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelPoolListRepository) ListModelPoolAccountsPage(context.Context, int, int) (store.ModelPoolPage, error) {
	return r.page, nil
}

func TestListModelPoolAccountsUsesNormalizedPageReader(t *testing.T) {
	repository := &normalizedModelPoolListRepository{
		now: time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC),
		page: store.ModelPoolPage{Total: 2, Items: []controlplane.ModelPoolAccountSummary{
			{ID: "mpa_1", Status: controlplane.ModelAccountStatusActive, DailyLimit: 10, DailyUsedTokens: 10},
			{ID: "mpa_2", Status: controlplane.ModelAccountStatusDisabled},
		}},
	}
	svc := NewControlPlaneWithRepository(repository)
	items, err := svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if len(items) != 2 || items[0].Status != controlplane.ModelAccountStatusExhausted || items[1].Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("normalized model pool list = %+v", items)
	}
}

type failingSecretStore struct{}

type busyAdvisoryLockRepository struct {
	store.Repository
	tryCount int
}

func (r *busyAdvisoryLockRepository) TryAdvisoryLock(context.Context, int64) (store.AdvisoryLockRelease, bool, error) {
	r.tryCount++
	return nil, false, nil
}

func (failingSecretStore) Put(context.Context, string, string) error {
	return errors.New("secret store unavailable")
}
func (failingSecretStore) Get(context.Context, string) (string, error) {
	return "", errors.New("secret store unavailable")
}
func (failingSecretStore) Delete(context.Context, string) error {
	return errors.New("secret store unavailable")
}
func (failingSecretStore) Ping(context.Context) error { return errors.New("secret store unavailable") }

type recordingModelPoolHealthProbeTelemetry struct {
	mu     sync.Mutex
	runs   []recordedModelPoolHealthProbeRun
	probes map[ModelPoolHealthProbeStatus]int
	skips  map[ModelPoolHealthProbeSkipReason]int
}

type recordedModelPoolHealthProbeRun struct {
	status   ModelPoolHealthProbeRunStatus
	duration time.Duration
}

func (r *recordingModelPoolHealthProbeTelemetry) ObserveRun(status ModelPoolHealthProbeRunStatus, duration time.Duration) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.runs = append(r.runs, recordedModelPoolHealthProbeRun{status: status, duration: duration})
}

func (r *recordingModelPoolHealthProbeTelemetry) ObserveProbe(status ModelPoolHealthProbeStatus) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.probes == nil {
		r.probes = make(map[ModelPoolHealthProbeStatus]int)
	}
	r.probes[status]++
}

func (r *recordingModelPoolHealthProbeTelemetry) ObserveSkip(reason ModelPoolHealthProbeSkipReason) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.skips == nil {
		r.skips = make(map[ModelPoolHealthProbeSkipReason]int)
	}
	r.skips[reason]++
}

func (r *recordingModelPoolHealthProbeTelemetry) assertRun(t *testing.T, want ModelPoolHealthProbeRunStatus) {
	t.Helper()
	r.assertRunCount(t, 1, want)
}

func (r *recordingModelPoolHealthProbeTelemetry) assertRunCount(t *testing.T, wantCount int, want ModelPoolHealthProbeRunStatus) {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	if len(r.runs) != wantCount {
		t.Fatalf("telemetry runs = %+v, want %d runs", r.runs, wantCount)
	}
	for _, run := range r.runs {
		if run.status != want {
			t.Fatalf("telemetry run status = %q, want %q", run.status, want)
		}
		if run.duration < 0 {
			t.Fatalf("telemetry run duration = %s, want non-negative", run.duration)
		}
	}
}

func (r *recordingModelPoolHealthProbeTelemetry) assertProbeCount(t *testing.T, status ModelPoolHealthProbeStatus, want int) {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	if got := r.probes[status]; got != want {
		t.Fatalf("telemetry probe %q count = %d, want %d", status, got, want)
	}
}

func (r *recordingModelPoolHealthProbeTelemetry) assertSkipCount(t *testing.T, reason ModelPoolHealthProbeSkipReason, want int) {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	if got := r.skips[reason]; got != want {
		t.Fatalf("telemetry skip %q count = %d, want %d", reason, got, want)
	}
}
