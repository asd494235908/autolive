package service

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type modelPoolPageRepository struct {
	now  time.Time
	page store.ModelPoolPage
}

type normalizedModelPoolWithoutPageReader struct {
	*store.MemoryStore
}

type failOnceRepository struct {
	store.Repository
	failNext atomic.Bool
}

type unknownCommitRepository struct {
	*store.MemoryStore
	runs atomic.Int32
}

func (r *unknownCommitRepository) Run(ctx context.Context, operation store.StateOperation) error {
	if r.runs.Add(1) == 2 {
		if err := r.MemoryStore.Run(ctx, operation); err != nil {
			return err
		}
		return store.ErrCommitOutcomeUnknown
	}
	return r.MemoryStore.Run(ctx, operation)
}

type trackingSecretStore struct {
	*store.MemorySecretStore
	mu      sync.Mutex
	puts    []string
	deleted []string
}

type failRetiredSecretStore struct {
	*store.MemorySecretStore
	failReference string
	failNext      bool
}

func (s *failRetiredSecretStore) Delete(ctx context.Context, reference string) error {
	if s.failNext && reference == s.failReference {
		s.failNext = false
		return errors.New("forced retired secret deletion failure")
	}
	return s.MemorySecretStore.Delete(ctx, reference)
}

func (s *trackingSecretStore) Put(ctx context.Context, reference, value string) error {
	s.mu.Lock()
	s.puts = append(s.puts, reference)
	s.mu.Unlock()
	return s.MemorySecretStore.Put(ctx, reference, value)
}

func (s *trackingSecretStore) Delete(ctx context.Context, reference string) error {
	s.mu.Lock()
	s.deleted = append(s.deleted, reference)
	s.mu.Unlock()
	return s.MemorySecretStore.Delete(ctx, reference)
}

type normalizedSecretRotationRepository struct {
	*store.MemoryStore
	record store.ModelPoolSecretRotationRecord
}

func (r *normalizedSecretRotationRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedSecretRotationRepository) RotateModelPoolAccountSecret(_ context.Context, record store.ModelPoolSecretRotationRecord) (controlplane.ModelPoolAccountSummary, error) {
	r.record = record
	return controlplane.ModelPoolAccountSummary{ID: record.AccountID, SecretConfigured: true, SecretRef: record.ExpectedSecretRef + "/rotation_test"}, nil
}

func (r *normalizedSecretRotationRepository) PrepareModelPoolAccountSecretRotation(ctx context.Context, scope, idempotencyKey, fingerprint, accountID string) (store.ModelPoolSecretRotationPreparation, error) {
	var preparation store.ModelPoolSecretRotationPreparation
	err := r.MemoryStore.Run(ctx, func(state *store.State) error {
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return controlplane.ErrModelPoolAccountNotFound
		}
		preparation.Account = account
		_ = scope
		if existing, exists := state.IdempotencyRecords[idempotencyKey]; exists {
			if existing.Fingerprint != fingerprint || existing.ResourceID != accountID {
				return controlplane.ErrIdempotencyConflict
			}
			summary := decorateModelPoolAccount(state, account, r.Now())
			preparation.Existing = &summary
		}
		return nil
	})
	return preparation, err
}

func (r *failOnceRepository) Run(ctx context.Context, operation store.StateOperation) error {
	if r.failNext.CompareAndSwap(true, false) {
		return errors.New("forced repository commit failure")
	}
	return r.Repository.Run(ctx, operation)
}

func (r *modelPoolPageRepository) Now() time.Time { return r.now }

func (r *modelPoolPageRepository) Run(context.Context, store.StateOperation) error { return nil }

func (r *modelPoolPageRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelPoolWithoutPageReader) UsesNormalizedReadSource() bool { return true }

func (r *modelPoolPageRepository) ListModelPoolAccountsPage(context.Context, int, int) (store.ModelPoolPage, error) {
	return r.page, nil
}

func TestListModelPoolAccountsPageNormalizesNormalizedProjectionStatus(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &modelPoolPageRepository{
		now: now,
		page: store.ModelPoolPage{Total: 1, Items: []controlplane.ModelPoolAccountSummary{{
			ID: "mpa_00000001", Status: controlplane.ModelAccountStatusActive, DailyLimit: 100, DailyUsedTokens: 100,
		}}},
	}
	svc := NewControlPlaneWithRepository(repository)
	items, total, err := svc.ListModelPoolAccountsPage(context.Background(), 1, 20)
	if err != nil {
		t.Fatalf("ListModelPoolAccountsPage() error = %v", err)
	}
	if total != 1 || len(items) != 1 || items[0].Status != controlplane.ModelAccountStatusExhausted {
		t.Fatalf("normalized model pool page = total %d items %+v", total, items)
	}
}

func TestListModelPoolAccountsPageFailsClosedWithoutNormalizedReader(t *testing.T) {
	repository := &normalizedModelPoolWithoutPageReader{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	_, _, err := svc.ListModelPoolAccountsPage(context.Background(), 1, 20)
	if !errors.Is(err, store.ErrNormalizedModelPoolPageReaderRequired) {
		t.Fatalf("ListModelPoolAccountsPage() error = %v, want normalized reader requirement", err)
	}
}

func TestListModelPoolHealthAccountsFailsClosedWithoutNormalizedReader(t *testing.T) {
	repository := &normalizedModelPoolWithoutPageReader{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	_, err := svc.ListModelPoolHealthAccounts(context.Background(), 20)
	if !errors.Is(err, store.ErrNormalizedModelPoolHealthPageReaderRequired) {
		t.Fatalf("ListModelPoolHealthAccounts() error = %v, want normalized health reader requirement", err)
	}
}

func TestModelPoolConnectivityTestUsesOpenAICompatibleModelsEndpoint(t *testing.T) {
	var authorization string
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/models" {
			t.Fatalf("path = %q, want /v1/models", r.URL.Path)
		}
		authorization = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	client, resolver := modelPoolTestHTTPClient(t, provider)
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, store.NewMemorySecretStore(), resolver)
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "succeeded" || result.HTTPStatus != http.StatusOK || result.Model != "rewrite-model" {
		t.Fatalf("connectivity result = %+v", result)
	}
	if authorization != "Bearer sk-test-secret" {
		t.Fatalf("authorization = %q, want provider bearer header", authorization)
	}
	if result.TestedAt == "" {
		t.Fatal("connectivity result TestedAt is empty")
	}
	items, err := svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after connectivity test error = %v", err)
	}
	if len(items) != 1 || items[0].LastTestStatus != "succeeded" || items[0].LastTestedAt != result.TestedAt {
		t.Fatalf("account health summary = %+v", items)
	}
}

func TestModelPoolConnectivityTestUsesNormalizedRepositoryBoundary(t *testing.T) {
	var authorization string
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		authorization = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := &normalizedModelPoolTestStub{
		MemoryStore: store.NewMemoryStore(time.Now),
		preparation: store.ModelPoolTestPreparation{Account: controlplane.ModelPoolAccountSummary{
			ID: "mpa_normalized", Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1",
			Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/mpa_normalized",
		}},
	}
	secretStore := store.NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/mpa_normalized", "sk-normalized-secret"); err != nil {
		t.Fatalf("seed secret: %v", err)
	}
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, secretStore, resolver)
	audit := controlplane.AuditLogInput{Action: "POST /api/v1/admin/model-pool/{account_id}/test", TargetType: "model_account", Outcome: "success", StatusCode: 200, RequestID: "req-model-test"}
	result, err := svc.TestModelPoolAccountWithAudit(context.Background(), "normalized-test", "mpa_normalized", controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5}, audit)
	if err != nil {
		t.Fatalf("TestModelPoolAccountWithAudit() error = %v", err)
	}
	if result.Status != "succeeded" || authorization != "Bearer sk-normalized-secret" {
		t.Fatalf("normalized connectivity result = %+v authorization=%q", result, authorization)
	}
	if repository.prepareRecord.IdempotencyKey != "test-model-account:mpa_normalized:normalized-test" || repository.record.AccountID != "mpa_normalized" || repository.record.Audit != audit {
		t.Fatalf("normalized test records = prepare:%+v complete:%+v", repository.prepareRecord, repository.record)
	}
}

func TestModelPoolConnectivityTestRequiresExplicitInsecureHTTPOptIn(t *testing.T) {
	svc := NewControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), &http.Client{}, store.NewMemorySecretStore())
	account, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-insecure-http", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-insecure-http-test", account.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" || result.ErrorCode != modelPoolConnectivityInsecureHTTP || result.HTTPStatus != 0 {
		t.Fatalf("insecure HTTP result = %+v", result)
	}
}

func TestRotateModelPoolAccountSecretValidatesBeforeSwitchingAndIsIdempotent(t *testing.T) {
	allowedSecret := "new-secret"
	var requests int
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests++
		if r.Header.Get("Authorization") != "Bearer "+allowedSecret {
			http.Error(w, "invalid api key", http.StatusUnauthorized)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	client, resolver := modelPoolTestHTTPClient(t, provider)
	secretStore := store.NewMemorySecretStore()
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, secretStore, resolver)
	account, err := svc.CreateModelPoolAccount(context.Background(), "rotate-account-create", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "old-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	if _, err := svc.RotateModelPoolAccountSecret(context.Background(), "rotate-account-failed", account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: "bad-secret", TimeoutSeconds: 5}); !controlplane.IsErrorCode(err, controlplane.ErrModelPoolSecretValidation.Code) {
		t.Fatalf("failed rotation error = %v, want validation error", err)
	}
	if got, err := secretStore.Get(context.Background(), account.SecretRef); err != nil || got != "old-secret" {
		t.Fatalf("secret after failed rotation = %q, %v; want old-secret", got, err)
	}

	rotated, err := svc.RotateModelPoolAccountSecret(context.Background(), "rotate-account-success", account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: allowedSecret, TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("successful rotation error = %v", err)
	}
	if !rotated.SecretConfigured {
		t.Fatal("rotated account should report configured secret")
	}
	if rotated.SecretRef == account.SecretRef {
		t.Fatal("successful rotation should switch to a staged secret reference")
	}
	if got, err := secretStore.Get(context.Background(), rotated.SecretRef); err != nil || got != allowedSecret {
		t.Fatalf("secret after successful rotation = %q, %v; want new-secret", got, err)
	}
	if _, err := secretStore.Get(context.Background(), account.SecretRef); !errors.Is(err, store.ErrSecretNotFound) {
		t.Fatalf("old secret after successful rotation error = %v, want ErrSecretNotFound", err)
	}
	requestCount := requests
	repeated, err := svc.RotateModelPoolAccountSecret(context.Background(), "rotate-account-success", account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: allowedSecret, TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("idempotent rotation error = %v", err)
	}
	if repeated.ID != rotated.ID || requests != requestCount {
		t.Fatalf("idempotent rotation repeated probe or changed account: repeated=%+v requests=%d want=%d", repeated, requests, requestCount)
	}
}

func TestRotateModelPoolAccountSecretQueuesRetiredSecretAfterDeleteFailure(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := store.NewMemoryStore(time.Now)
	secretStore := &failRetiredSecretStore{MemorySecretStore: store.NewMemorySecretStore()}
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, secretStore, resolver)
	account, err := svc.CreateModelPoolAccount(context.Background(), "rotate-delete-failure-create", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "old-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	secretStore.failReference = account.SecretRef
	secretStore.failNext = true

	rotated, err := svc.RotateModelPoolAccountSecret(context.Background(), "rotate-delete-failure", account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: "new-secret", TimeoutSeconds: 5})
	if !controlplane.IsErrorCode(err, controlplane.ErrSecretStoreUnavailable.Code) {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v, want SECRET_STORE_UNAVAILABLE", err)
	}
	if rotated.ID != "" {
		t.Fatalf("rotation response after retired-secret deletion failure = %+v, want empty response", rotated)
	}
	if got, getErr := secretStore.Get(context.Background(), account.SecretRef); getErr != nil || got != "old-secret" {
		t.Fatalf("old secret after deletion failure = %q, %v", got, getErr)
	}

	var pendingAt time.Time
	var activeRef string
	if err := repository.Run(context.Background(), func(state *store.State) error {
		activeRef = state.ModelPoolAccounts[account.ID].SecretRef
		pendingAt = state.PendingSecretCleanup[account.SecretRef]
		return nil
	}); err != nil {
		t.Fatalf("read pending cleanup state: %v", err)
	}
	if activeRef == account.SecretRef || pendingAt.IsZero() {
		t.Fatalf("rotation state = active_ref %q pending_at %v, want switched active ref and queued retired ref", activeRef, pendingAt)
	}

	deleted, err := svc.CleanupStagedSecrets(context.Background(), store.RetentionCleanupRequest{Cutoff: time.Now().UTC().Add(time.Minute), BatchSize: 10})
	if err != nil {
		t.Fatalf("CleanupStagedSecrets() error = %v", err)
	}
	if deleted != 1 {
		t.Fatalf("CleanupStagedSecrets() deleted = %d, want 1 retired reference", deleted)
	}
	if _, err := secretStore.Get(context.Background(), account.SecretRef); !errors.Is(err, store.ErrSecretNotFound) {
		t.Fatalf("old secret after queued cleanup error = %v, want ErrSecretNotFound", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if _, exists := state.PendingSecretCleanup[account.SecretRef]; exists {
			t.Fatalf("pending retired reference remains after successful cleanup")
		}
		return nil
	}); err != nil {
		t.Fatalf("verify pending cleanup removal: %v", err)
	}
}

func TestRotateModelPoolAccountSecretPreservesStagedSecretAfterUnknownCommit(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	repository := &unknownCommitRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	if err := repository.MemoryStore.Run(context.Background(), func(state *store.State) error {
		state.ModelPoolAccounts["mpa_commit_unknown"] = controlplane.ModelPoolAccountSummary{
			ID: "mpa_commit_unknown", Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1",
			Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/mpa_commit_unknown", ConcurrencyLimit: 1,
		}
		return nil
	}); err != nil {
		t.Fatalf("seed account = %v", err)
	}
	secretStore := &trackingSecretStore{MemorySecretStore: store.NewMemorySecretStore()}
	if err := secretStore.Put(context.Background(), "model-account/mpa_commit_unknown", "old-secret"); err != nil {
		t.Fatalf("seed secret = %v", err)
	}
	client, resolver := modelPoolTestHTTPClient(t, provider)
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, secretStore, resolver)

	_, err := svc.RotateModelPoolAccountSecret(context.Background(), "commit-unknown", "mpa_commit_unknown", controlplane.RotateModelPoolAccountSecretInput{APIKey: "new-secret", TimeoutSeconds: 5})
	if !errors.Is(err, store.ErrCommitOutcomeUnknown) {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v, want ErrCommitOutcomeUnknown", err)
	}
	if len(secretStore.deleted) != 0 {
		t.Fatalf("secret cleanup after unknown commit = %v, want no deletion", secretStore.deleted)
	}
	if len(secretStore.puts) != 2 {
		t.Fatalf("secret writes = %v, want original and staged candidate", secretStore.puts)
	}
	if staged, err := secretStore.Get(context.Background(), secretStore.puts[1]); err != nil || staged != "new-secret" {
		t.Fatalf("staged secret after unknown commit = %q, %v", staged, err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if _, ok := state.PendingSecretCleanup["model-account/mpa_commit_unknown"]; !ok {
			t.Fatalf("pending cleanup queue missing after unknown commit: %+v", state.PendingSecretCleanup)
		}
		return nil
	}); err != nil {
		t.Fatalf("inspect pending cleanup queue: %v", err)
	}
}

func TestRotateModelPoolAccountSecretUsesNormalizedTransactionalRepository(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := &normalizedSecretRotationRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	secretStore := store.NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/mpa_1", "old-secret"); err != nil {
		t.Fatalf("secretStore.Put() error = %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelPoolAccounts["mpa_1"] = controlplane.ModelPoolAccountSummary{ID: "mpa_1", Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", Status: controlplane.ModelAccountStatusActive, SecretRef: "model-account/mpa_1", ConcurrencyLimit: 1}
		return nil
	}); err != nil {
		t.Fatalf("seed repository = %v", err)
	}
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, secretStore, resolver)
	audit := controlplane.AuditLogInput{Action: "POST /api/v1/admin/model-pool/{account_id}/rotate-secret", TargetType: "model_account", TargetID: "mpa_1", Outcome: "success", StatusCode: 200, RequestID: "req-normalized-rotation"}
	rotated, err := svc.RotateModelPoolAccountSecretWithAudit(context.Background(), "rotate-normalized", "mpa_1", controlplane.RotateModelPoolAccountSecretInput{APIKey: "new-secret", TimeoutSeconds: 5}, audit)
	if err != nil {
		t.Fatalf("RotateModelPoolAccountSecretWithAudit() error = %v", err)
	}
	if rotated.ID != "mpa_1" || repository.record.ExpectedSecretRef != "model-account/mpa_1" || repository.record.APIKey != "new-secret" || repository.record.Audit != audit {
		t.Fatalf("normalized rotation route = summary=%+v record=%+v", rotated, repository.record)
	}
}

func TestRotateModelPoolAccountSecretConcurrentFailureCannotDeleteWinnerSecret(t *testing.T) {
	const allowedSecret = "new-secret"
	var probes atomic.Int32
	probedTwice := make(chan struct{})
	release := make(chan struct{})
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if probes.Add(1) == 2 {
			close(probedTwice)
		}
		<-release
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	client, resolver := modelPoolTestHTTPClient(t, provider)
	memoryRepository := store.NewMemoryStore(time.Now)
	repository := &failOnceRepository{Repository: memoryRepository}
	secretStore := store.NewMemorySecretStore()
	svc := newControlPlaneWithRepositoryAndSecretStore(repository, client, secretStore, resolver)
	account, err := svc.CreateModelPoolAccount(context.Background(), "rotate-concurrent-create", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "old-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	type result struct {
		account controlplane.ModelPoolAccountSummary
		err     error
	}
	results := make(chan result, 2)
	var waitGroup sync.WaitGroup
	for range 2 {
		waitGroup.Add(1)
		go func() {
			defer waitGroup.Done()
			rotated, rotateErr := svc.RotateModelPoolAccountSecret(context.Background(), "rotate-concurrent", account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: allowedSecret, TimeoutSeconds: 5})
			results <- result{account: rotated, err: rotateErr}
		}()
	}
	<-probedTwice
	repository.failNext.Store(true)
	close(release)
	waitGroup.Wait()
	close(results)

	var winner controlplane.ModelPoolAccountSummary
	successes := 0
	for item := range results {
		if item.err == nil {
			successes++
			winner = item.account
		}
	}
	if successes != 1 {
		t.Fatalf("rotation successes = %d, want one winner", successes)
	}
	if winner.SecretRef == account.SecretRef {
		t.Fatal("winner did not switch to a unique staged secret")
	}
	if got, err := secretStore.Get(context.Background(), winner.SecretRef); err != nil || got != allowedSecret {
		t.Fatalf("winner secret = %q, %v; want retained new secret", got, err)
	}
	if _, err := secretStore.Get(context.Background(), account.SecretRef); !errors.Is(err, store.ErrSecretNotFound) {
		t.Fatalf("old secret lookup error = %v, want ErrSecretNotFound", err)
	}
}

func TestRotateModelPoolAccountSecretDifferentIdempotencyKeysCannotOverwriteWinner(t *testing.T) {
	const allowedSecret = "new-secret"
	var probes atomic.Int32
	probedTwice := make(chan struct{})
	release := make(chan struct{})
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if probes.Add(1) == 2 {
			close(probedTwice)
		}
		<-release
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()

	client, resolver := modelPoolTestHTTPClient(t, provider)
	secretStore := store.NewMemorySecretStore()
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, secretStore, resolver)
	account, err := svc.CreateModelPoolAccount(context.Background(), "rotate-different-keys-create", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "old-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	type result struct {
		account controlplane.ModelPoolAccountSummary
		err     error
	}
	results := make(chan result, 2)
	var waitGroup sync.WaitGroup
	for _, idempotencyKey := range []string{"rotate-different-keys-a", "rotate-different-keys-b"} {
		waitGroup.Add(1)
		go func(idempotencyKey string) {
			defer waitGroup.Done()
			rotated, rotateErr := svc.RotateModelPoolAccountSecret(context.Background(), idempotencyKey, account.ID, controlplane.RotateModelPoolAccountSecretInput{APIKey: allowedSecret, TimeoutSeconds: 5})
			results <- result{account: rotated, err: rotateErr}
		}(idempotencyKey)
	}
	<-probedTwice
	close(release)
	waitGroup.Wait()
	close(results)

	var winner controlplane.ModelPoolAccountSummary
	successes := 0
	conflicts := 0
	for item := range results {
		if item.err == nil {
			successes++
			winner = item.account
			continue
		}
		if controlplane.IsErrorCode(item.err, controlplane.ErrModelPoolSecretRotationConflict.Code) {
			conflicts++
			continue
		}
		t.Fatalf("unexpected rotation error = %v", item.err)
	}
	if successes != 1 || conflicts != 1 {
		t.Fatalf("rotation outcomes = successes %d conflicts %d, want one each", successes, conflicts)
	}
	if got, err := secretStore.Get(context.Background(), winner.SecretRef); err != nil || got != allowedSecret {
		t.Fatalf("winner secret = %q, %v; want retained new secret", got, err)
	}
	if _, err := secretStore.Get(context.Background(), account.SecretRef); !errors.Is(err, store.ErrSecretNotFound) {
		t.Fatalf("old secret lookup error = %v, want ErrSecretNotFound", err)
	}
}

func TestModelPoolConnectivityTestRejectsPrivateEndpointBeforeConnecting(t *testing.T) {
	called := false
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
	}))
	defer provider.Close()

	svc := NewControlPlaneWithRepositoryAndSecretStoreAndOptions(store.NewMemoryStore(time.Now), provider.Client(), store.NewMemorySecretStore(), ControlPlaneOptions{AllowInsecureHTTP: true})
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-private", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://127.0.0.1/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-private-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" || result.ErrorCode != "MODEL_POOL_CONNECTIVITY_SSRF_BLOCKED" || result.HTTPStatus != 0 {
		t.Fatalf("private endpoint result = %+v", result)
	}
	if called {
		t.Fatal("private endpoint was contacted")
	}
}

func TestModelPoolConnectivityTestRejectsIPv6LoopbackBeforeConnecting(t *testing.T) {
	svc := NewControlPlaneWithRepositoryAndSecretStoreAndOptions(store.NewMemoryStore(time.Now), &http.Client{}, store.NewMemorySecretStore(), ControlPlaneOptions{AllowInsecureHTTP: true})
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-ipv6-loopback", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://[::1]/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-ipv6-loopback-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" || result.ErrorCode != modelPoolConnectivitySSRFBlocked || result.HTTPStatus != 0 {
		t.Fatalf("IPv6 loopback result = %+v", result)
	}
}

func TestModelPoolConnectivityTestRequiresTargetModelInSuccessfulJSON(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"another-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, store.NewMemorySecretStore(), resolver)
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-missing", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-missing-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" || result.ErrorCode != "MODEL_POOL_MODEL_NOT_FOUND" || result.HTTPStatus != http.StatusOK {
		t.Fatalf("missing model result = %+v", result)
	}
}

func TestModelPoolConnectivityTestRejectsOversizedAndInvalidResponses(t *testing.T) {
	tests := []struct {
		name      string
		body      string
		errorCode string
	}{
		{name: "invalid-json", body: "not-json", errorCode: "MODEL_POOL_INVALID_RESPONSE"},
		{name: "oversized", body: strings.Repeat("x", (1<<20)+1), errorCode: "MODEL_POOL_RESPONSE_TOO_LARGE"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(tt.body))
			}))
			defer provider.Close()
			client, resolver := modelPoolTestHTTPClient(t, provider)
			svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, store.NewMemorySecretStore(), resolver)
			created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-response-"+tt.name, controlplane.CreateModelPoolAccountInput{
				Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
			})
			if err != nil {
				t.Fatalf("CreateModelPoolAccount() error = %v", err)
			}
			result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-response-test-"+tt.name, created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
			if err != nil {
				t.Fatalf("TestModelPoolAccount() error = %v", err)
			}
			if result.Status != "failed" || result.ErrorCode != tt.errorCode {
				t.Fatalf("response result = %+v", result)
			}
		})
	}
}

func TestModelPoolAccountCooldownExpiresAndRecoversWithInjectedClock(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte("not-json"))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(func() time.Time { return now }), client, store.NewMemorySecretStore(), resolver)
	account, err := svc.CreateModelPoolAccount(context.Background(), "cooldown-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "cooldown-test", account.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" {
		t.Fatalf("connectivity result status = %q, want failed", result.Status)
	}
	items, err := svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() during cooldown error = %v", err)
	}
	if items[0].Status != controlplane.ModelAccountStatusCooldown || items[0].CooldownUntil == "" {
		t.Fatalf("cooldown summary = %+v", items[0])
	}
	now = now.Add(modelAccountCooldownDuration)
	items, err = svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after cooldown error = %v", err)
	}
	if items[0].Status != controlplane.ModelAccountStatusActive || items[0].CooldownUntil != "" {
		t.Fatalf("recovered summary = %+v", items[0])
	}
}

func TestModelPoolConnectivityTestRejectsPrivateRedirect(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "http://127.0.0.1/v1/models", http.StatusFound)
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	svc := newControlPlaneWithRepositoryAndSecretStore(store.NewMemoryStore(time.Now), client, store.NewMemorySecretStore(), resolver)
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-redirect", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-redirect-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "failed" || result.ErrorCode != "MODEL_POOL_CONNECTIVITY_SSRF_BLOCKED" {
		t.Fatalf("redirect result = %+v", result)
	}
}

func modelPoolTestHTTPClient(t *testing.T, provider *httptest.Server) (*http.Client, func(context.Context, string) ([]netip.Addr, error)) {
	t.Helper()
	transport, ok := provider.Client().Transport.(*http.Transport)
	if !ok {
		t.Fatalf("provider transport type = %T, want *http.Transport", provider.Client().Transport)
	}
	transport = transport.Clone()
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, provider.Listener.Addr().String())
	}
	resolver := func(ctx context.Context, host string) ([]netip.Addr, error) {
		if host == "model.test" {
			return []netip.Addr{netip.MustParseAddr("192.0.2.1")}, nil
		}
		return nil, errors.New("unexpected test host: " + host)
	}
	return &http.Client{Transport: transport}, resolver
}

func derefModelPoolTestString(t *testing.T, value *string) string {
	t.Helper()
	if value == nil {
		t.Fatal("expected non-nil string pointer")
	}
	return *value
}

func TestModelPoolSummaryTracksLeasesAndDailyUsageAndStopsAtQuota(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	ctx := context.Background()
	code, err := svc.CreateActivationCode(ctx, "quota-code-1", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(ctx, "quota-device-1", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefModelPoolTestString(t, code.PlainCode),
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_quota01", DeviceName: "MacBook", Platform: "macOS", AppVersion: "0.1.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	account, err := svc.CreateModelPoolAccount(ctx, "quota-account-1", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", APIKey: "sk-test-secret", DailyLimit: 10, ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	lease, err := svc.CreateModelLease(ctx, "quota-lease-1", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: account.Provider, Model: account.Model, Purpose: "realtime_script"})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	_, err = svc.RecordDirectLLMCall(ctx, "quota-call-1", "usr_local_admin", device.ID, "req_quota01", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_quota01", LeaseID: lease.ID, Provider: account.Provider, Model: account.Model, InputTokens: 4, OutputTokens: 3, Status: "succeeded", UsageSource: "client_reported",
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() first error = %v", err)
	}
	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if items[0].ActiveLeases != 1 || items[0].DailyUsedTokens != 7 || items[0].Status != controlplane.ModelAccountStatusActive {
		t.Fatalf("account usage summary after first call = %+v", items[0])
	}
	_, err = svc.RecordDirectLLMCall(ctx, "quota-call-2", "usr_local_admin", device.ID, "req_quota02", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_quota02", LeaseID: lease.ID, Provider: account.Provider, Model: account.Model, InputTokens: 2, OutputTokens: 1, Status: "succeeded", UsageSource: "client_reported",
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() second error = %v", err)
	}
	items, err = svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after quota error = %v", err)
	}
	if items[0].DailyUsedTokens != 10 || items[0].Status != controlplane.ModelAccountStatusExhausted {
		t.Fatalf("account usage summary at quota = %+v", items[0])
	}
	if _, err := svc.CreateModelLease(ctx, "quota-lease-2", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: account.Provider, Model: account.Model, Purpose: "realtime_script"}); !controlplane.IsErrorCode(err, "MODEL_POOL_UNAVAILABLE") {
		t.Fatalf("CreateModelLease() after quota error = %v, want MODEL_POOL_UNAVAILABLE", err)
	}
	now = now.Add(24 * time.Hour)
	items, err = svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after UTC day rollover error = %v", err)
	}
	if items[0].Status != controlplane.ModelAccountStatusActive || items[0].DailyUsedTokens != 0 {
		t.Fatalf("account usage summary after UTC day rollover = %+v", items[0])
	}
}

func TestRecordDirectLLMCallValidatesLeaseAndIsIdempotent(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(context.Background(), "direct-call-code", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(context.Background(), "direct-call-device", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefModelPoolTestString(t, code.PlainCode),
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_direct01", DeviceName: "MacBook", Platform: "macOS", AppVersion: "0.1.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	if _, err := svc.CreateModelPoolAccount(context.Background(), "direct-call-account", controlplane.CreateModelPoolAccountInput{Provider: "openai-compatible", Model: "rewrite-model", APIKey: "sk-test-secret", ConcurrencyLimit: 1}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	lease, err := svc.CreateModelLease(context.Background(), "direct-call-lease", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: "openai-compatible", Model: "rewrite-model", Purpose: "realtime_script"})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	input := controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_direct01", LeaseID: lease.ID, Provider: lease.Provider, Model: lease.Model, InputTokens: 3, OutputTokens: 5, LatencyMS: 40, Status: "succeeded", UsageSource: "client_reported"}
	record, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-record", "usr_local_admin", device.ID, "req_test", input)
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() error = %v", err)
	}
	if record.TotalTokens != 8 || record.RequestID != "req_test" || record.UsageSource != "client_reported" {
		t.Fatalf("record = %+v", record)
	}
	repeated, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-record", "usr_local_admin", device.ID, "req_test-2", input)
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() repeated error = %v", err)
	}
	if repeated.ID != record.ID {
		t.Fatalf("repeated id = %q, want %q", repeated.ID, record.ID)
	}
	if _, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-invalid", "usr_local_admin", device.ID, "req_invalid", controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_direct02", LeaseID: lease.ID, Provider: "other", Model: lease.Model, Status: "succeeded", UsageSource: "client_reported"}); err == nil {
		t.Fatal("RecordDirectLLMCall() with provider mismatch unexpectedly succeeded")
	}
}

func TestCreateAndListModelPoolRedactsSecretAndSupportsIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	if !created.SecretConfigured {
		t.Fatalf("SecretConfigured = %v, want true", created.SecretConfigured)
	}

	repeated, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() duplicate error = %v", err)
	}
	if repeated.ID != created.ID {
		t.Fatalf("duplicate id = %q, want %q", repeated.ID, created.ID)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model-v2",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	}); err == nil {
		t.Fatal("CreateModelPoolAccount() with conflicting idempotency unexpectedly succeeded")
	}

	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("len(items) = %d, want 1", len(items))
	}
	if items[0].ID != created.ID || !items[0].SecretConfigured {
		t.Fatalf("unexpected item = %+v", items[0])
	}
}

func TestDisableModelPoolAccountIsIdempotentAndBlocksMissingAccounts(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "disable-account-create", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	disabled, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", created.ID)
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() error = %v", err)
	}
	if disabled.Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("disabled status = %q, want %q", disabled.Status, controlplane.ModelAccountStatusDisabled)
	}

	repeated, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", created.ID)
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() repeated error = %v", err)
	}
	if repeated.ID != created.ID || repeated.Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("repeated result = %+v, want disabled account %q", repeated, created.ID)
	}

	if _, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", "another-account"); !controlplane.IsErrorCode(err, "IDEMPOTENCY_CONFLICT") {
		t.Fatalf("conflicting idempotency error = %v, want IDEMPOTENCY_CONFLICT", err)
	}
	if _, err := svc.DisableModelPoolAccount(ctx, "disable-account-missing", "missing-account"); !controlplane.IsErrorCode(err, "MODEL_POOL_ACCOUNT_NOT_FOUND") {
		t.Fatalf("missing account error = %v, want MODEL_POOL_ACCOUNT_NOT_FOUND", err)
	}
}

func TestUpdateModelPoolAccountSupportsCooldownAndIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "update-account-create", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		BaseURL:          "https://api.example.com/v1",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	priority := 20
	dailyLimit := 2000
	status := controlplane.ModelAccountStatusCooldown
	updated, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{
		Priority:   &priority,
		DailyLimit: &dailyLimit,
		Status:     &status,
	})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() error = %v", err)
	}
	if updated.Priority != priority || updated.DailyLimit != dailyLimit || updated.Status != status {
		t.Fatalf("updated account = %+v", updated)
	}

	repeated, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{
		Priority:   &priority,
		DailyLimit: &dailyLimit,
		Status:     &status,
	})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() repeated error = %v", err)
	}
	if repeated.ID != created.ID || repeated.Status != status {
		t.Fatalf("repeated account = %+v", repeated)
	}

	otherPriority := 30
	if _, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{Priority: &otherPriority}); !controlplane.IsErrorCode(err, "IDEMPOTENCY_CONFLICT") {
		t.Fatalf("conflicting update error = %v, want IDEMPOTENCY_CONFLICT", err)
	}
	if _, err := svc.UpdateModelPoolAccount(ctx, "update-account-missing", "missing-account", controlplane.UpdateModelPoolAccountInput{Priority: &priority}); !controlplane.IsErrorCode(err, "MODEL_POOL_ACCOUNT_NOT_FOUND") {
		t.Fatalf("missing account error = %v, want MODEL_POOL_ACCOUNT_NOT_FOUND", err)
	}
}

func TestModelLeaseLifecycleChecksOwnershipAndReleaseIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	ctx := context.Background()

	if _, err := svc.CreateUser(ctx, "lease-user-1", controlplane.CreateUserInput{
		Username: "lease-user",
		Password: "correct-password",
		Role:     controlplane.RoleUser,
	}); err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if _, err := svc.CreateUser(ctx, "lease-user-2", controlplane.CreateUserInput{
		Username: "other-user",
		Password: "correct-password",
		Role:     controlplane.RoleUser,
	}); err != nil {
		t.Fatalf("CreateUser() second user error = %v", err)
	}

	code, err := svc.CreateActivationCode(ctx, "lease-code", controlplane.CreateActivationCodeInput{
		ExpiresAt:  now.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(ctx, "lease-device", "usr_00000001", controlplane.ActivateDeviceInput{
		ActivationCode: derefModelPoolTestString(t, code.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_lease001",
			DeviceName: "MacBook",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "lease-account", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	lease, err := svc.CreateModelLease(ctx, "lease-create-1", "usr_00000001", device.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	if lease.ProxyMode != controlplane.ModelLeaseProxyModeDirectLease {
		t.Fatalf("ProxyMode = %q, want %q", lease.ProxyMode, controlplane.ModelLeaseProxyModeDirectLease)
	}

	leaseAgain, err := svc.CreateModelLease(ctx, "lease-create-1", "usr_00000001", device.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() duplicate error = %v", err)
	}
	if leaseAgain.ID != lease.ID {
		t.Fatalf("duplicate lease id = %q, want %q", leaseAgain.ID, lease.ID)
	}

	renewed, err := svc.RenewModelLease(ctx, "lease-renew-1", "usr_00000001", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 120,
	})
	if err != nil {
		t.Fatalf("RenewModelLease() error = %v", err)
	}
	if renewed.ID != lease.ID || renewed.ExpiresAt <= lease.ExpiresAt {
		t.Fatalf("unexpected renewed lease = %+v", renewed)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-foreign", "usr_00000002", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 60,
	}); err == nil {
		t.Fatal("RenewModelLease() with foreign user unexpectedly succeeded")
	}

	release, err := svc.ReleaseModelLease(ctx, "lease-release-1", "usr_00000001", device.ID, lease.ID, controlplane.ReleaseModelLeaseInput{
		Reason: "done",
	})
	if err != nil {
		t.Fatalf("ReleaseModelLease() error = %v", err)
	}
	if !release.Released {
		t.Fatalf("Released = %v, want true", release.Released)
	}

	releaseAgain, err := svc.ReleaseModelLease(ctx, "lease-release-2", "usr_00000001", device.ID, lease.ID, controlplane.ReleaseModelLeaseInput{
		Reason: "retry",
	})
	if err != nil {
		t.Fatalf("ReleaseModelLease() repeat error = %v", err)
	}
	if !releaseAgain.Released {
		t.Fatalf("repeat Released = %v, want true", releaseAgain.Released)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-after-release", "usr_00000001", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 60,
	}); err == nil {
		t.Fatal("RenewModelLease() after release unexpectedly succeeded")
	}
}

func TestListModelLeasesPageWithOptionsFiltersAndSorts(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelLeases["lease-active"] = controlplane.ModelLease{
			ID: "lease-active", AccountID: "account-a", UserID: "user-a", DeviceID: "device-a", Provider: "openai", Model: "rewrite", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339),
		}
		state.ModelLeases["lease-released"] = controlplane.ModelLease{
			ID: "lease-released", AccountID: "account-b", UserID: "user-b", DeviceID: "device-b", Provider: "other", Model: "chat", Status: controlplane.ModelLeaseStatusReleased, ExpiresAt: now.Add(2 * time.Hour).Format(time.RFC3339),
		}
		state.ModelLeases["lease-expired"] = controlplane.ModelLease{
			ID: "lease-expired", AccountID: "account-c", UserID: "user-c", DeviceID: "device-c", Provider: "openai", Model: "rewrite", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(-time.Hour).Format(time.RFC3339),
		}
		return nil
	}); err != nil {
		t.Fatalf("seed model leases error = %v", err)
	}
	svc := NewControlPlane(repository)
	items, total, err := svc.ListModelLeasesPageWithOptions(context.Background(), 1, 20, ModelLeaseListOptions{
		Status: controlplane.ModelLeaseStatusActive, Provider: " openai ", Sort: store.ModelLeaseSortProviderModel,
	})
	if err != nil {
		t.Fatalf("ListModelLeasesPageWithOptions() error = %v", err)
	}
	if total != 1 || len(items) != 1 || items[0].ID != "lease-active" || items[0].Status != controlplane.ModelLeaseStatusActive {
		t.Fatalf("filtered lease page = total %d items %+v", total, items)
	}
	if _, _, err := svc.ListModelLeasesPageWithOptions(context.Background(), 1, 20, ModelLeaseListOptions{Status: "unknown"}); !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("invalid lease status error = %v, want INVALID_ARGUMENT", err)
	}
	if _, _, err := svc.ListModelLeasesPageWithOptions(context.Background(), 1, 20, ModelLeaseListOptions{Sort: "expires_at desc"}); !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("invalid lease sort error = %v, want INVALID_ARGUMENT", err)
	}
}

func TestAdminModelLeaseDetailAndReclaimAreRedactedAndIdempotent(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelLeases["lease-admin-detail"] = controlplane.ModelLease{
			ID: "lease-admin-detail", AccountID: "account-a", UserID: "user-a", DeviceID: "device-a", Purpose: "validation",
			Provider: "openai", Model: "rewrite", Status: controlplane.ModelLeaseStatusActive,
			CreatedAt: now.Add(-time.Minute).Format(time.RFC3339), ExpiresAt: now.Add(time.Hour).Format(time.RFC3339),
			DirectBaseURL: "https://provider.example/v1", ConcurrencyLimit: 2,
		}
		return nil
	}); err != nil {
		t.Fatalf("seed model lease error = %v", err)
	}
	svc := NewControlPlane(repository)
	detail, err := svc.GetModelLeaseAdminDetail(context.Background(), "lease-admin-detail")
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetail() error = %v", err)
	}
	if detail.ID != "lease-admin-detail" || detail.CreatedAt == "" || detail.Status != controlplane.ModelLeaseStatusActive {
		t.Fatalf("unexpected admin detail = %+v", detail)
	}
	detailPayload, err := json.Marshal(detail)
	if err != nil {
		t.Fatalf("marshal admin detail: %v", err)
	}
	if strings.Contains(string(detailPayload), "provider.example") {
		t.Fatal("admin detail unexpectedly contains direct base URL")
	}

	result, err := svc.ReclaimModelLease(context.Background(), "admin-reclaim-1", "lease-admin-detail", controlplane.ReleaseModelLeaseInput{Reason: "stale"})
	if err != nil || !result.Released {
		t.Fatalf("ReclaimModelLease() = %+v, %v", result, err)
	}
	replayed, err := svc.ReclaimModelLease(context.Background(), "admin-reclaim-1", "lease-admin-detail", controlplane.ReleaseModelLeaseInput{Reason: "stale"})
	if err != nil || !replayed.Released {
		t.Fatalf("ReclaimModelLease() replay = %+v, %v", replayed, err)
	}
	if _, err := svc.ReclaimModelLease(context.Background(), "admin-reclaim-1", "lease-admin-detail", controlplane.ReleaseModelLeaseInput{Reason: "different"}); !controlplane.IsErrorCode(err, "IDEMPOTENCY_CONFLICT") {
		t.Fatalf("conflicting reclaim error = %v, want IDEMPOTENCY_CONFLICT", err)
	}
	detail, err = svc.GetModelLeaseAdminDetail(context.Background(), "lease-admin-detail")
	if err != nil || detail.Status != controlplane.ModelLeaseStatusReleased || detail.ReleasedAt == "" {
		t.Fatalf("reclaimed detail = %+v, %v", detail, err)
	}
}

func TestModelLeaseConcurrencyDisabledAccountAndLazyExpiryRecycle(t *testing.T) {
	startNow := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	currentNow := startNow
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return currentNow }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	ctx := context.Background()

	code1, err := svc.CreateActivationCode(ctx, "lease-code-1", controlplane.CreateActivationCodeInput{
		ExpiresAt:  startNow.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device1, err := svc.ActivateDevice(ctx, "lease-device-1", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefModelPoolTestString(t, code1.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_expiry001",
			DeviceName: "MacBook",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() first error = %v", err)
	}

	code2, err := svc.CreateActivationCode(ctx, "lease-code-2", controlplane.CreateActivationCodeInput{
		ExpiresAt:  startNow.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() second error = %v", err)
	}
	device2, err := svc.ActivateDevice(ctx, "lease-device-2", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefModelPoolTestString(t, code2.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_expiry002",
			DeviceName: "MacBook 2",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() second error = %v", err)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "lease-account-disabled", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-disabled",
		Priority:         100,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
		Status:           controlplane.ModelAccountStatusDisabled,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() disabled error = %v", err)
	}
	activeAccount, err := svc.CreateModelPoolAccount(ctx, "lease-account-active", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-active",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() active error = %v", err)
	}
	if _, err := svc.CreateModelLease(ctx, "lease-create-unbound", "usr_local_admin", "", controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	}); !controlplane.IsErrorCode(err, "DEVICE_BINDING_REQUIRED") {
		t.Fatalf("CreateModelLease() without bound device error = %v, want DEVICE_BINDING_REQUIRED", err)
	}

	first, err := svc.CreateModelLease(ctx, "lease-create-first", "usr_local_admin", device1.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() first error = %v", err)
	}

	disabledStatus := controlplane.ModelAccountStatusDisabled
	if _, err := svc.UpdateModelPoolAccount(ctx, "lease-account-disable-via-update", activeAccount.ID, controlplane.UpdateModelPoolAccountInput{
		Status: &disabledStatus,
	}); !controlplane.IsErrorCode(err, "MODEL_POOL_ACCOUNT_IN_USE") {
		t.Fatalf("UpdateModelPoolAccount() disabling leased account error = %v, want MODEL_POOL_ACCOUNT_IN_USE", err)
	}

	if _, err := svc.CreateModelLease(ctx, "lease-create-second", "usr_local_admin", device2.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	}); err == nil {
		t.Fatal("CreateModelLease() under concurrency limit unexpectedly succeeded")
	}

	currentNow = startNow.Add(31 * time.Second)
	second, err := svc.CreateModelLease(ctx, "lease-create-third", "usr_local_admin", device2.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() after expiry error = %v", err)
	}
	if second.ID == first.ID {
		t.Fatalf("CreateModelLease() recycled same lease id = %q", second.ID)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-expired", "usr_local_admin", device1.ID, first.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 30,
	}); err == nil {
		t.Fatal("RenewModelLease() expired lease unexpectedly succeeded")
	}

	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("len(items) = %d, want 2", len(items))
	}
	if items[0].Status != controlplane.ModelAccountStatusDisabled && items[1].Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("disabled account summary missing: %+v", items)
	}
}
