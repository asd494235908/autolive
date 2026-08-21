package service

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestUpdateUserAuthorizationNormalizesAndIsIdempotent(t *testing.T) {
	now := time.Date(2026, 8, 21, 3, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	user, err := svc.CreateUser(ctx, "create-policy-user", controlplane.CreateUserInput{Username: "policy-user", Password: "password-123", Role: controlplane.RoleUser})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	input := controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"openai/rewrite", "anthropic/claude"}, DailyTokenLimit: 100}
	policy, err := svc.UpdateUserAuthorization(ctx, "policy-key", user.ID, input)
	if err != nil {
		t.Fatalf("UpdateUserAuthorization() error = %v", err)
	}
	if policy.AllowedModels[0] != "anthropic/claude" || policy.AllowedModels[1] != "openai/rewrite" || policy.DailyTokenLimit != 100 {
		t.Fatalf("policy = %+v", policy)
	}
	repeated, err := svc.UpdateUserAuthorization(ctx, "policy-key", user.ID, input)
	if err != nil || repeated.UpdatedAt != policy.UpdatedAt {
		t.Fatalf("idempotent update = %+v, error = %v", repeated, err)
	}
	summary, err := svc.GetUserAuthorizationSummary(ctx, user.ID)
	if err != nil {
		t.Fatalf("GetUserAuthorizationSummary() error = %v", err)
	}
	if summary.DailyTokenLimit != 100 || summary.HardQuotaConfigured || summary.QuotaEnforcement != "server_recorded_usage_guard" {
		t.Fatalf("summary quota semantics = %+v", summary)
	}
}

type normalizedAuthorizationSummaryRepository struct {
	*store.MemoryStore
	summary controlplane.UserAuthorizationSummary
	called  bool
}

func (r *normalizedAuthorizationSummaryRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedAuthorizationSummaryRepository) GetUserAuthorizationSummary(context.Context, string) (controlplane.UserAuthorizationSummary, error) {
	r.called = true
	return r.summary, nil
}

func TestGetUserAuthorizationSummaryUsesNormalizedReader(t *testing.T) {
	repository := &normalizedAuthorizationSummaryRepository{
		MemoryStore: store.NewMemoryStore(time.Now),
		summary:     controlplane.UserAuthorizationSummary{UserID: "usr_1", UsageSource: "client_reported_soft"},
	}
	summary, err := NewControlPlaneWithRepository(repository).GetUserAuthorizationSummary(context.Background(), "usr_1")
	if err != nil {
		t.Fatalf("GetUserAuthorizationSummary() error = %v", err)
	}
	if !repository.called || summary.UserID != "usr_1" {
		t.Fatalf("reader called:%t summary:%+v", repository.called, summary)
	}
}

type normalizedAuthorizationSummaryWithoutReader struct{ *store.MemoryStore }

func (r *normalizedAuthorizationSummaryWithoutReader) UsesNormalizedReadSource() bool { return true }

func TestGetUserAuthorizationSummaryFailsClosedWithoutNormalizedReader(t *testing.T) {
	repository := &normalizedAuthorizationSummaryWithoutReader{MemoryStore: store.NewMemoryStore(time.Now)}
	_, err := NewControlPlaneWithRepository(repository).GetUserAuthorizationSummary(context.Background(), "usr_1")
	if !errors.Is(err, store.ErrNormalizedUserAuthorizationSummaryReaderRequired) {
		t.Fatalf("error = %v, want normalized reader requirement", err)
	}
}

func TestModelLeaseHonorsUserModelAllowlistAndRecordedQuota(t *testing.T) {
	now := time.Date(2026, 8, 21, 3, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	secretStore := store.NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/mpa_policy", "sk-policy-secret"); err != nil {
		t.Fatalf("secretStore.Put() error = %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_policy"] = controlplane.UserSummary{ID: "usr_policy", Username: "policy", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		state.Devices["dev_policy"] = controlplane.DeviceSummary{ID: "dev_policy", UserID: "usr_policy", Status: controlplane.DeviceStatusActive}
		state.ModelPoolAccounts["mpa_policy"] = controlplane.ModelPoolAccountSummary{ID: "mpa_policy", Provider: "openai", Model: "rewrite", Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/mpa_policy", ConcurrencyLimit: 1}
		return nil
	}); err != nil {
		t.Fatalf("seed state error = %v", err)
	}
	svc := NewControlPlaneWithRepositoryAndSecretStore(repository, http.DefaultClient, secretStore)
	ctx := context.Background()
	if _, err := svc.UpdateUserAuthorization(ctx, "policy-config", "usr_policy", controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"openai/rewrite"}, DailyTokenLimit: 10}); err != nil {
		t.Fatalf("UpdateUserAuthorization() error = %v", err)
	}
	lease, err := svc.CreateModelLease(ctx, "lease-policy-1", "usr_policy", "dev_policy", controlplane.CreateModelLeaseInput{Provider: "openai", Model: "rewrite", Purpose: "chat"})
	if err != nil {
		t.Fatalf("CreateModelLease() allowed error = %v", err)
	}
	if _, err := svc.ReleaseModelLease(ctx, "release-policy-1", "usr_policy", "dev_policy", lease.ID, controlplane.ReleaseModelLeaseInput{}); err != nil {
		t.Fatalf("ReleaseModelLease() error = %v", err)
	}
	if err := repository.Run(ctx, func(state *store.State) error {
		state.ModelUsageRecords["usage_policy"] = controlplane.ModelUsageRecord{ID: "usage_policy", LeaseID: lease.ID, TotalTokens: 10, CreatedAt: now.Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed usage error = %v", err)
	}
	if _, err := svc.CreateModelLease(ctx, "lease-policy-2", "usr_policy", "dev_policy", controlplane.CreateModelLeaseInput{Provider: "openai", Model: "rewrite", Purpose: "chat"}); !controlplane.IsErrorCode(err, controlplane.ErrUserRecordedQuotaExceeded.Code) {
		t.Fatalf("quota lease error = %v, want %s", err, controlplane.ErrUserRecordedQuotaExceeded.Code)
	}
	if _, err := svc.UpdateUserAuthorization(ctx, "policy-config-2", "usr_policy", controlplane.UpdateUserAuthorizationInput{AllowedModels: []string{"anthropic/claude"}, DailyTokenLimit: 0}); err != nil {
		t.Fatalf("UpdateUserAuthorization() second error = %v", err)
	}
	if _, err := svc.CreateModelLease(ctx, "lease-policy-3", "usr_policy", "dev_policy", controlplane.CreateModelLeaseInput{Provider: "openai", Model: "rewrite", Purpose: "chat"}); !controlplane.IsErrorCode(err, controlplane.ErrUserModelNotAuthorized.Code) {
		t.Fatalf("allowlist lease error = %v, want %s", err, controlplane.ErrUserModelNotAuthorized.Code)
	}
}
