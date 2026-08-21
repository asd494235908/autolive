package httpapi

import (
	"context"
	"net/http"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestAdminUserAuthorizationSummaryEndpoint(t *testing.T) {
	now := time.Date(2026, 8, 20, 11, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_summary"] = controlplane.UserSummary{ID: "usr_summary", Username: "summary-user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.Devices["dev_summary"] = controlplane.DeviceSummary{ID: "dev_summary", UserID: "usr_summary", Status: controlplane.DeviceStatusActive}
		state.ModelLeases["lease_summary"] = controlplane.ModelLease{ID: "lease_summary", AccountID: "account_summary", UserID: "usr_summary", DeviceID: "dev_summary", Status: controlplane.ModelLeaseStatusActive}
		state.ModelUsageRecords["usage_summary"] = controlplane.ModelUsageRecord{ID: "usage_summary", LeaseID: "lease_summary", TotalTokens: 12, CreatedAt: now.Add(-time.Hour).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed authorization summary state: %v", err)
	}
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), nil, true)
	token := loginForTest(t, handler)
	rec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/users/usr_summary/authorization-summary", nil, token, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("summary status = %d, want %d, body=%s", rec.Code, http.StatusOK, rec.Body.String())
	}
	var payload userAuthorizationSummaryResponse
	decodeJSON(t, rec.Body.Bytes(), &payload)
	if payload.Summary.UserID != "usr_summary" || payload.Summary.ActiveDeviceCount != 1 || payload.Summary.ActiveLeaseCount != 1 || payload.Summary.DailyUsedTokens != 12 {
		t.Fatalf("summary payload = %+v", payload)
	}
	if payload.Summary.HardQuotaConfigured || payload.Summary.UsageSource != "client_reported_soft" {
		t.Fatalf("summary quota semantics = %+v", payload.Summary)
	}
}

func TestAdminUserAuthorizationPolicyEndpoint(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_policy"] = controlplane.UserSummary{ID: "usr_policy", Username: "policy-user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		return nil
	}); err != nil {
		t.Fatalf("seed policy state: %v", err)
	}
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), nil, true)
	token := loginForTest(t, handler)
	rec := doJSON(t, handler, http.MethodPatch, "/api/v1/admin/users/usr_policy/authorization", map[string]any{
		"allowed_models": []string{"openai/rewrite"}, "daily_token_limit": 500,
	}, token, "policy-http-key")
	if rec.Code != http.StatusOK {
		t.Fatalf("policy update status = %d, want %d, body=%s", rec.Code, http.StatusOK, rec.Body.String())
	}
	var payload userAuthorizationPolicyResponse
	decodeJSON(t, rec.Body.Bytes(), &payload)
	if payload.Policy.UserID != "usr_policy" || len(payload.Policy.AllowedModels) != 1 || payload.Policy.DailyTokenLimit != 500 {
		t.Fatalf("policy payload = %+v", payload)
	}
	repeat := doJSON(t, handler, http.MethodPatch, "/api/v1/admin/users/usr_policy/authorization", map[string]any{
		"allowed_models": []string{"openai/rewrite"}, "daily_token_limit": 500,
	}, token, "policy-http-key")
	if repeat.Code != http.StatusOK {
		t.Fatalf("idempotent policy update status = %d, want %d", repeat.Code, http.StatusOK)
	}
}
