package httpapi

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"slices"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
	"golang.org/x/crypto/bcrypt"
)

func TestAdminProductScopeFiltersActualControlPlanePages(t *testing.T) {
	endpoints := []struct {
		name     string
		path     string
		itemsKey string
	}{
		{name: "devices", path: "/api/v1/admin/devices", itemsKey: "items"},
		{name: "activation codes", path: "/api/v1/admin/activation-codes", itemsKey: "items"},
		{name: "model pool", path: "/api/v1/admin/model-pool", itemsKey: "accounts"},
		{name: "model usage", path: "/api/v1/admin/model-usage", itemsKey: "items"},
		{name: "model leases", path: "/api/v1/admin/model-leases", itemsKey: "items"},
		{name: "audit logs", path: "/api/v1/admin/audit-logs?action=seed", itemsKey: "items"},
	}

	for _, endpoint := range endpoints {
		t.Run(endpoint.name, func(t *testing.T) {
			handler, localToken, _ := newProductScopeIntegrationRouter(t)
			assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, endpoint.path, nil, localToken, ""), endpoint.itemsKey, 2, "")
			assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, appendProductQuery(endpoint.path, "douyin_desktop"), nil, localToken, ""), endpoint.itemsKey, 1, controlplane.ProductDouyinDesktop)
		})
	}
}

func TestAdminUserListProductScopeReturnsActualItemsAndTotals(t *testing.T) {
	handler, localToken, ordinaryToken := newProductScopeIntegrationRouter(t)
	wantAll := []string{"usr_local_admin", "usr_product_admin", "usr_shared"}
	wantDouyin := []string{"usr_shared"}
	assertUserPage(t, doJSON(t, handler, http.MethodGet, "/api/v1/admin/users", nil, localToken, ""), wantAll)
	assertUserPage(t, doJSON(t, handler, http.MethodGet, "/api/v1/admin/users?product=douyin_desktop", nil, localToken, ""), wantDouyin)
	assertUserPage(t, doJSON(t, handler, http.MethodGet, "/api/v1/admin/users", nil, ordinaryToken, ""), wantAll)
	assertUserPage(t, doJSON(t, handler, http.MethodGet, "/api/v1/admin/users?product=autolive", nil, ordinaryToken, ""), wantAll)
}

func TestOrdinaryAdminProductScopeDefaultsToSessionProduct(t *testing.T) {
	handler, _, ordinaryToken := newProductScopeIntegrationRouter(t)
	endpoints := []struct {
		name     string
		path     string
		itemsKey string
		want     int
	}{
		{name: "user devices", path: "/api/v1/admin/users/usr_shared/devices", itemsKey: "items", want: 1},
		{name: "devices", path: "/api/v1/admin/devices", itemsKey: "items", want: 1},
		{name: "activation codes", path: "/api/v1/admin/activation-codes", itemsKey: "items", want: 1},
		{name: "model pool", path: "/api/v1/admin/model-pool", itemsKey: "accounts", want: 1},
		{name: "model usage", path: "/api/v1/admin/model-usage", itemsKey: "items", want: 1},
		{name: "model leases", path: "/api/v1/admin/model-leases", itemsKey: "items", want: 1},
		{name: "audit logs", path: "/api/v1/admin/audit-logs?action=seed", itemsKey: "items", want: 1},
	}
	for _, endpoint := range endpoints {
		t.Run(endpoint.name, func(t *testing.T) {
			assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, endpoint.path, nil, ordinaryToken, ""), endpoint.itemsKey, endpoint.want, controlplane.ProductAutoLive)
			assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, appendProductQuery(endpoint.path, "autolive"), nil, ordinaryToken, ""), endpoint.itemsKey, endpoint.want, controlplane.ProductAutoLive)
		})
	}
}

func appendProductQuery(path, product string) string {
	if len(path) > 0 && path[len(path)-1] == '?' {
		return path + "product=" + product
	}
	for _, char := range path {
		if char == '?' {
			return path + "&product=" + product
		}
	}
	return path + "?product=" + product
}

func TestAdminUserDevicePageAppliesProductScope(t *testing.T) {
	handler, localToken, ordinaryToken := newProductScopeIntegrationRouter(t)
	path := "/api/v1/admin/users/usr_shared/devices"
	assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, path, nil, localToken, ""), "items", 2, "")
	assertScopedProductPage(t, doJSON(t, handler, http.MethodGet, path+"?product=douyin_desktop", nil, localToken, ""), "items", 1, controlplane.ProductDouyinDesktop)

	for _, query := range []string{"?product=unknown", "?product=autolive&product=douyin_desktop"} {
		response := doJSON(t, handler, http.MethodGet, path+query, nil, localToken, "")
		if response.Code != http.StatusBadRequest {
			t.Fatalf("local admin %s status = %d, want %d; body=%s", query, response.Code, http.StatusBadRequest, response.Body.String())
		}
	}

	forbidden := doJSON(t, handler, http.MethodGet, path+"?product=douyin_desktop", nil, ordinaryToken, "")
	if forbidden.Code != http.StatusForbidden {
		t.Fatalf("ordinary admin cross-product status = %d, want %d; body=%s", forbidden.Code, http.StatusForbidden, forbidden.Body.String())
	}
}

func TestAdminDeviceDetailAppliesProductScope(t *testing.T) {
	handler, localToken, ordinaryToken := newProductScopeIntegrationRouter(t)

	local := doJSON(t, handler, http.MethodGet, "/api/v1/admin/devices/dev_douyin?product=douyin_desktop", nil, localToken, "")
	if local.Code != http.StatusOK {
		t.Fatalf("local admin device detail status = %d, want %d; body=%s", local.Code, http.StatusOK, local.Body.String())
	}

	ordinary := doJSON(t, handler, http.MethodGet, "/api/v1/admin/devices/dev_douyin", nil, ordinaryToken, "")
	if ordinary.Code != http.StatusNotFound {
		t.Fatalf("ordinary admin cross-product device detail status = %d, want %d; body=%s", ordinary.Code, http.StatusNotFound, ordinary.Body.String())
	}

	widened := doJSON(t, handler, http.MethodGet, "/api/v1/admin/devices/dev_douyin?product=douyin_desktop", nil, ordinaryToken, "")
	if widened.Code != http.StatusForbidden {
		t.Fatalf("ordinary admin widened device detail status = %d, want %d; body=%s", widened.Code, http.StatusForbidden, widened.Body.String())
	}
}

func TestOrdinaryAdminCannotAccessGlobalUserAuthorizationOperations(t *testing.T) {
	handler, _, ordinaryToken := newProductScopeIntegrationRouter(t)
	for _, endpoint := range []struct {
		name   string
		method string
		path   string
		body   any
	}{
		{name: "authorization summary", method: http.MethodGet, path: "/api/v1/admin/users/usr_shared/authorization-summary"},
		{name: "authorization policy", method: http.MethodPatch, path: "/api/v1/admin/users/usr_shared/authorization", body: map[string]any{}},
		{name: "disable user", method: http.MethodPost, path: "/api/v1/admin/users/usr_shared/disable"},
		{name: "update user", method: http.MethodPatch, path: "/api/v1/admin/users/usr_shared", body: map[string]any{}},
		{name: "reset password", method: http.MethodPost, path: "/api/v1/admin/users/usr_shared/reset-password", body: map[string]any{}},
		{name: "create user", method: http.MethodPost, path: "/api/v1/admin/users", body: map[string]any{"username": "ordinary-created", "password": "ordinary-created-password", "role": "user"}},
	} {
		t.Run(endpoint.name, func(t *testing.T) {
			response := doJSON(t, handler, endpoint.method, endpoint.path, endpoint.body, ordinaryToken, "ordinary-admin-guard")
			if response.Code != http.StatusForbidden {
				t.Fatalf("ordinary admin %s status = %d, want %d; body=%s", endpoint.name, response.Code, http.StatusForbidden, response.Body.String())
			}
		})
	}
}

func TestBuiltinLocalAdminCanCreateUpdateAndDisableUser(t *testing.T) {
	handler, localToken, _ := newProductScopeIntegrationRouter(t)
	created := doJSON(t, handler, http.MethodPost, "/api/v1/admin/users", map[string]any{
		"username": "local-managed-user",
		"password": "local-managed-password",
		"role":     "user",
	}, localToken, "local-create-managed-user")
	if created.Code != http.StatusCreated {
		t.Fatalf("local admin create status = %d, want %d; body=%s", created.Code, http.StatusCreated, created.Body.String())
	}
	var createPayload userEnvelope
	decodeJSON(t, created.Body.Bytes(), &createPayload)
	if createPayload.User.ID == "" || createPayload.User.Username != "local-managed-user" || createPayload.User.Status != controlplane.UserStatusActive {
		t.Fatalf("created user = %+v", createPayload.User)
	}

	updated := doJSON(t, handler, http.MethodPatch, "/api/v1/admin/users/"+createPayload.User.ID, map[string]any{
		"username": "local-managed-user-renamed",
	}, localToken, "local-update-managed-user")
	if updated.Code != http.StatusOK {
		t.Fatalf("local admin update status = %d, want %d; body=%s", updated.Code, http.StatusOK, updated.Body.String())
	}
	var updatePayload userEnvelope
	decodeJSON(t, updated.Body.Bytes(), &updatePayload)
	if updatePayload.User.Username != "local-managed-user-renamed" || updatePayload.User.Status != controlplane.UserStatusActive {
		t.Fatalf("updated user = %+v", updatePayload.User)
	}

	disabled := doJSON(t, handler, http.MethodPost, "/api/v1/admin/users/"+createPayload.User.ID+"/disable", nil, localToken, "local-disable-managed-user")
	if disabled.Code != http.StatusOK {
		t.Fatalf("local admin disable status = %d, want %d; body=%s", disabled.Code, http.StatusOK, disabled.Body.String())
	}
	var disablePayload userEnvelope
	decodeJSON(t, disabled.Body.Bytes(), &disablePayload)
	if disablePayload.User.Status != controlplane.UserStatusDisabled {
		t.Fatalf("disabled user = %+v", disablePayload.User)
	}
}

func newProductScopeIntegrationRouter(t *testing.T) (http.Handler, string, string) {
	t.Helper()
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	passwordHash, err := bcrypt.GenerateFromPassword([]byte("ordinary-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatalf("hash ordinary admin password: %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_shared"] = controlplane.UserSummary{ID: "usr_shared", Username: "shared", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.Users["usr_product_admin"] = controlplane.UserSummary{ID: "usr_product_admin", Username: "ordinary-admin", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.UserCredentialHashes["usr_product_admin"] = passwordHash
		state.UserProducts["usr_shared:autolive"] = controlplane.UserProductMembership{UserID: "usr_shared", Product: controlplane.ProductAutoLive, Status: "active"}
		state.UserProducts["usr_shared:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_shared", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		state.UserProducts["usr_product_admin:autolive"] = controlplane.UserProductMembership{UserID: "usr_product_admin", Product: controlplane.ProductAutoLive, Status: "active"}
		state.Devices["dev_autolive"] = controlplane.DeviceSummary{ID: "dev_autolive", UserID: "usr_shared", Product: controlplane.ProductAutoLive, Status: controlplane.DeviceStatusActive}
		state.Devices["dev_douyin"] = controlplane.DeviceSummary{ID: "dev_douyin", UserID: "usr_shared", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ActivationCodes["ac_autolive"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "ac_autolive", Product: controlplane.ProductAutoLive, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339), MaxDevices: 1}}
		state.ActivationCodes["ac_douyin"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "ac_douyin", Product: controlplane.ProductDouyinDesktop, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339), MaxDevices: 1}}
		state.ModelPoolAccounts["mpa_autolive"] = controlplane.ModelPoolAccountSummary{ID: "mpa_autolive", Product: controlplane.ProductAutoLive, Provider: "openai-compatible", Model: "autolive", Status: controlplane.ModelAccountStatusActive}
		state.ModelPoolAccounts["mpa_douyin"] = controlplane.ModelPoolAccountSummary{ID: "mpa_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai-compatible", Model: "douyin", Status: controlplane.ModelAccountStatusActive}
		state.ModelLeases["lease_autolive"] = controlplane.ModelLease{ID: "lease_autolive", Product: controlplane.ProductAutoLive, UserID: "usr_shared", DeviceID: "dev_autolive", AccountID: "mpa_autolive", Provider: "openai-compatible", Model: "autolive", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelLeases["lease_douyin"] = controlplane.ModelLease{ID: "lease_douyin", Product: controlplane.ProductDouyinDesktop, UserID: "usr_shared", DeviceID: "dev_douyin", AccountID: "mpa_douyin", Provider: "openai-compatible", Model: "douyin", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_autolive"] = controlplane.ModelUsageRecord{ID: "usage_autolive", Product: controlplane.ProductAutoLive, LeaseID: "lease_autolive", Provider: "openai-compatible", Model: "autolive", Status: "succeeded", CreatedAt: now.Format(time.RFC3339)}
		state.ModelUsageRecords["usage_douyin"] = controlplane.ModelUsageRecord{ID: "usage_douyin", Product: controlplane.ProductDouyinDesktop, LeaseID: "lease_douyin", Provider: "openai-compatible", Model: "douyin", Status: "succeeded", CreatedAt: now.Add(time.Second).Format(time.RFC3339)}
		state.AuditLogs["audit_autolive"] = controlplane.AuditLog{ID: "audit_autolive", Product: controlplane.ProductAutoLive, Action: "seed", Outcome: "success", CreatedAt: now.Format(time.RFC3339)}
		state.AuditLogs["audit_douyin"] = controlplane.AuditLog{ID: "audit_douyin", Product: controlplane.ProductDouyinDesktop, Action: "seed", Outcome: "success", CreatedAt: now.Add(time.Second).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed product scope state: %v", err)
	}
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), nil, true)
	return handler,
		loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"password","product":"autolive"}`),
		loginWithCredentialsForTest(t, handler, `{"username":"ordinary-admin","password":"ordinary-password","product":"autolive"}`)
}

func assertScopedProductPage(t *testing.T, response *httptest.ResponseRecorder, itemsKey string, wantTotal int, wantProduct controlplane.ProductCode) {
	t.Helper()
	if response.Code != http.StatusOK {
		t.Fatalf("list status = %d, want %d; body=%s", response.Code, http.StatusOK, response.Body.String())
	}
	var payload map[string]json.RawMessage
	decodeJSON(t, response.Body.Bytes(), &payload)
	var items []struct {
		Product controlplane.ProductCode `json:"product"`
	}
	if err := json.Unmarshal(payload[itemsKey], &items); err != nil {
		t.Fatalf("decode %s: %v; body=%s", itemsKey, err, response.Body.String())
	}
	var page struct {
		Total int `json:"total"`
	}
	if err := json.Unmarshal(payload["pagination"], &page); err != nil {
		t.Fatalf("decode pagination: %v; body=%s", err, response.Body.String())
	}
	if page.Total != wantTotal || len(items) != wantTotal {
		t.Fatalf("page total/items = %d/%d, want %d/%d; body=%s", page.Total, len(items), wantTotal, wantTotal, response.Body.String())
	}
	if wantProduct == "" {
		seen := map[controlplane.ProductCode]bool{}
		for _, item := range items {
			seen[item.Product] = true
		}
		if !seen[controlplane.ProductAutoLive] || !seen[controlplane.ProductDouyinDesktop] {
			t.Fatalf("unfiltered products = %v, want both products", seen)
		}
		return
	}
	for _, item := range items {
		if item.Product != wantProduct {
			t.Fatalf("item product = %q, want %q", item.Product, wantProduct)
		}
	}
}

func assertUserPage(t *testing.T, response *httptest.ResponseRecorder, wantIDs []string) {
	t.Helper()
	if response.Code != http.StatusOK {
		t.Fatalf("user list status = %d, want %d; body=%s", response.Code, http.StatusOK, response.Body.String())
	}
	var payload struct {
		Items      []controlplane.UserSummary `json:"items"`
		Pagination struct {
			Total int `json:"total"`
		} `json:"pagination"`
	}
	decodeJSON(t, response.Body.Bytes(), &payload)
	if payload.Pagination.Total != len(wantIDs) || len(payload.Items) != len(wantIDs) {
		t.Fatalf("user page total/items = %d/%d, want %d/%d; body=%s", payload.Pagination.Total, len(payload.Items), len(wantIDs), len(wantIDs), response.Body.String())
	}
	gotIDs := make([]string, 0, len(payload.Items))
	for _, item := range payload.Items {
		gotIDs = append(gotIDs, item.ID)
	}
	if !slices.Equal(gotIDs, wantIDs) {
		t.Fatalf("user page ids = %v, want %v", gotIDs, wantIDs)
	}
}
