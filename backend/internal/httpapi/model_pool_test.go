package httpapi

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"autoLive/backend/internal/controlplane"
)

func TestAdminModelPoolCreateAndListEndpointsRedactSecret(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)

	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool", map[string]any{
		"provider":          "openai-compatible",
		"model":             "rewrite-model",
		"api_key":           "sk-live-secret",
		"priority":          10,
		"daily_limit":       1000,
		"concurrency_limit": 2,
	}, token, "idem-model-account-1")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create model account status = %d, want %d, body=%s", createRec.Code, http.StatusCreated, createRec.Body.String())
	}

	unboundLeaseRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider": "openai-compatible",
		"model":    "rewrite-model",
		"purpose":  "realtime_script",
	}, token, "idem-model-lease-unbound")
	if unboundLeaseRec.Code != http.StatusForbidden {
		t.Fatalf("unbound lease status = %d, want %d, body=%s", unboundLeaseRec.Code, http.StatusForbidden, unboundLeaseRec.Body.String())
	}

	var createPayload map[string]any
	decodeJSON(t, createRec.Body.Bytes(), &createPayload)
	account := createPayload["account"].(map[string]any)
	if _, exists := account["api_key"]; exists {
		t.Fatalf("api_key leaked in create response: %v", account)
	}
	if account["secret_configured"] != true {
		t.Fatalf("secret_configured = %v, want true", account["secret_configured"])
	}

	listRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-pool", nil, token, "")
	if listRec.Code != http.StatusOK {
		t.Fatalf("list model pool status = %d, want %d, body=%s", listRec.Code, http.StatusOK, listRec.Body.String())
	}

	var listPayload map[string]any
	decodeJSON(t, listRec.Body.Bytes(), &listPayload)
	accounts := listPayload["accounts"].([]any)
	if len(accounts) != 1 {
		t.Fatalf("len(accounts) = %d, want 1", len(accounts))
	}
	listAccount := accounts[0].(map[string]any)
	if _, exists := listAccount["api_key"]; exists {
		t.Fatalf("api_key leaked in list response: %v", listAccount)
	}
	if listAccount["secret_configured"] != true {
		t.Fatalf("secret_configured = %v, want true", listAccount["secret_configured"])
	}
	pagination := listPayload["pagination"].(map[string]any)
	if pagination["page"].(float64) != 1 || pagination["page_size"].(float64) != 20 || pagination["total"].(float64) != 1 {
		t.Fatalf("model pool pagination = %v", pagination)
	}
	invalidPageRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-pool?page_size=201", nil, token, "")
	if invalidPageRec.Code != http.StatusBadRequest {
		t.Fatalf("invalid model pool page status = %d, want %d, body=%s", invalidPageRec.Code, http.StatusBadRequest, invalidPageRec.Body.String())
	}
}

func TestAdminModelPoolConnectivityTestEndpoint(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool", map[string]any{
		"provider": "openai-compatible", "model": "rewrite-model", "api_key": "sk-live-secret", "base_url": "http://127.0.0.1:1/v1", "concurrency_limit": 1,
	}, token, "idem-model-account-connectivity")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create status = %d, body=%s", createRec.Code, createRec.Body.String())
	}
	var payload map[string]any
	decodeJSON(t, createRec.Body.Bytes(), &payload)
	accountID := payload["account"].(map[string]any)["id"].(string)
	testRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool/"+accountID+"/test", map[string]any{"timeout_seconds": 1}, token, "idem-model-account-connectivity-test")
	if testRec.Code != http.StatusOK {
		t.Fatalf("test status = %d, body=%s", testRec.Code, testRec.Body.String())
	}
	var testPayload map[string]any
	decodeJSON(t, testRec.Body.Bytes(), &testPayload)
	if testPayload["status"] != "failed" && testPayload["status"] != "timeout" {
		t.Fatalf("test result = %v", testPayload)
	}
	if strings.Contains(testRec.Body.String(), "sk-live-secret") {
		t.Fatalf("connectivity response leaked api key: %s", testRec.Body.String())
	}
	auditRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs?page_size=50", nil, token, "")
	if auditRec.Code != http.StatusOK {
		t.Fatalf("audit list status = %d, body=%s", auditRec.Code, auditRec.Body.String())
	}
	if strings.Contains(auditRec.Body.String(), "sk-live-secret") || strings.Contains(auditRec.Body.String(), "data:[]") {
		t.Fatalf("audit response leaked secret or provider response body: %s", auditRec.Body.String())
	}
}

func TestAdminModelPoolDisableEndpoint(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)

	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool", map[string]any{
		"provider":          "openai-compatible",
		"model":             "rewrite-model",
		"api_key":           "sk-live-secret",
		"priority":          10,
		"daily_limit":       1000,
		"concurrency_limit": 2,
	}, token, "idem-model-account-disable")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create model account status = %d, want %d, body=%s", createRec.Code, http.StatusCreated, createRec.Body.String())
	}

	var createPayload map[string]any
	decodeJSON(t, createRec.Body.Bytes(), &createPayload)
	accountID := createPayload["account"].(map[string]any)["id"].(string)

	disableRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool/"+accountID+"/disable", nil, token, "idem-model-account-disable-action")
	if disableRec.Code != http.StatusOK {
		t.Fatalf("disable model account status = %d, want %d, body=%s", disableRec.Code, http.StatusOK, disableRec.Body.String())
	}
	var disablePayload map[string]any
	decodeJSON(t, disableRec.Body.Bytes(), &disablePayload)
	if disablePayload["account"].(map[string]any)["status"] != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("disabled account payload = %v, want status %q", disablePayload, controlplane.ModelAccountStatusDisabled)
	}

	repeatRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool/"+accountID+"/disable", nil, token, "idem-model-account-disable-action")
	if repeatRec.Code != http.StatusOK {
		t.Fatalf("repeat disable status = %d, want %d, body=%s", repeatRec.Code, http.StatusOK, repeatRec.Body.String())
	}
}

func TestAdminModelPoolUpdateEndpoint(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)

	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool", map[string]any{
		"provider":          "openai-compatible",
		"model":             "rewrite-model",
		"api_key":           "sk-live-secret",
		"base_url":          "https://api.example.com/v1",
		"priority":          10,
		"daily_limit":       1000,
		"concurrency_limit": 2,
	}, token, "idem-model-account-update")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create model account status = %d, want %d, body=%s", createRec.Code, http.StatusCreated, createRec.Body.String())
	}
	var createPayload map[string]any
	decodeJSON(t, createRec.Body.Bytes(), &createPayload)
	accountID := createPayload["account"].(map[string]any)["id"].(string)

	updateRec := doJSON(t, handler, http.MethodPatch, "/api/v1/admin/model-pool/"+accountID, map[string]any{
		"priority":          20,
		"daily_limit":       2000,
		"concurrency_limit": 3,
		"status":            "cooldown",
	}, token, "idem-model-account-update-action")
	if updateRec.Code != http.StatusOK {
		t.Fatalf("update model account status = %d, want %d, body=%s", updateRec.Code, http.StatusOK, updateRec.Body.String())
	}
	var updatePayload map[string]any
	decodeJSON(t, updateRec.Body.Bytes(), &updatePayload)
	account := updatePayload["account"].(map[string]any)
	if account["status"] != "cooldown" || account["priority"].(float64) != 20 || account["daily_limit"].(float64) != 2000 {
		t.Fatalf("update payload = %v", updatePayload)
	}
}

func TestClientModelLeaseEndpointsLifecycleAndConflict(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	clientToken, userID := createDesktopUserForTest(t, handler, token, "model-lease-user")

	doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-pool", map[string]any{
		"provider":          "openai-compatible",
		"model":             "rewrite-model",
		"api_key":           "sk-live-secret",
		"priority":          10,
		"daily_limit":       1000,
		"concurrency_limit": 1,
	}, token, "idem-model-account-2")

	codeRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"user_id":     userID,
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "idem-model-lease-code")
	if codeRec.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d, body=%s", codeRec.Code, http.StatusCreated, codeRec.Body.String())
	}

	activateRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_model_lease_1",
			"device_name": "MacBook",
			"platform":    "macOS",
			"app_version": "0.1.0",
		},
	}, clientToken, "idem-model-lease-device")
	if activateRec.Code != http.StatusOK {
		t.Fatalf("activate status = %d, want %d, body=%s", activateRec.Code, http.StatusOK, activateRec.Body.String())
	}

	leaseRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider":             "openai-compatible",
		"model":                "rewrite-model",
		"purpose":              "realtime_script",
		"max_duration_seconds": 300,
	}, clientToken, "idem-model-lease-create")
	if leaseRec.Code != http.StatusOK {
		t.Fatalf("lease create status = %d, want %d, body=%s", leaseRec.Code, http.StatusOK, leaseRec.Body.String())
	}

	var leasePayload map[string]any
	decodeJSON(t, leaseRec.Body.Bytes(), &leasePayload)
	lease := leasePayload["lease"].(map[string]any)
	if lease["proxy_mode"] != "direct_lease" {
		t.Fatalf("proxy_mode = %v, want direct_lease", lease["proxy_mode"])
	}
	leaseID := lease["id"].(string)
	adminLeasesRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-leases?page_size=10", nil, token, "")
	if adminLeasesRec.Code != http.StatusOK {
		t.Fatalf("admin lease list status = %d, want %d, body=%s", adminLeasesRec.Code, http.StatusOK, adminLeasesRec.Body.String())
	}
	var adminLeasesPayload map[string]any
	decodeJSON(t, adminLeasesRec.Body.Bytes(), &adminLeasesPayload)
	adminLeases := adminLeasesPayload["items"].([]any)
	if len(adminLeases) != 1 {
		t.Fatalf("admin lease list = %v", adminLeasesPayload)
	}
	adminLease := adminLeases[0].(map[string]any)
	if adminLease["id"] != leaseID || adminLease["user_id"] == nil || adminLease["device_id"] == nil || adminLease["account_id"] == nil {
		t.Fatalf("admin lease summary = %v", adminLease)
	}
	if _, leaked := adminLease["direct_base_url"]; leaked {
		t.Fatalf("admin lease response leaked direct base URL: %v", adminLease)
	}
	filteredLeasesRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-leases?status=active&provider=openai-compatible&sort=provider_model&page_size=10", nil, token, "")
	if filteredLeasesRec.Code != http.StatusOK {
		t.Fatalf("filtered admin lease list status = %d, want %d, body=%s", filteredLeasesRec.Code, http.StatusOK, filteredLeasesRec.Body.String())
	}
	var filteredLeasesPayload map[string]any
	decodeJSON(t, filteredLeasesRec.Body.Bytes(), &filteredLeasesPayload)
	if filteredLeasesPayload["pagination"].(map[string]any)["total"].(float64) != 1 || len(filteredLeasesPayload["items"].([]any)) != 1 {
		t.Fatalf("filtered admin lease list = %v", filteredLeasesPayload)
	}
	invalidFilterRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-leases?status=unsupported", nil, token, "")
	if invalidFilterRec.Code != http.StatusBadRequest {
		t.Fatalf("invalid admin lease filter status = %d, want %d, body=%s", invalidFilterRec.Code, http.StatusBadRequest, invalidFilterRec.Body.String())
	}
	callRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/llm/call-records", map[string]any{
		"client_call_id": "call_http01",
		"lease_id":       leaseID,
		"provider":       "openai-compatible",
		"model":          "rewrite-model",
		"input_tokens":   3,
		"output_tokens":  5,
		"latency_ms":     40,
		"status":         "succeeded",
		"usage_source":   "client_reported",
	}, clientToken, "idem-http-call-record")
	if callRec.Code != http.StatusOK {
		t.Fatalf("call record status = %d, want %d, body=%s", callRec.Code, http.StatusOK, callRec.Body.String())
	}
	var callPayload map[string]any
	decodeJSON(t, callRec.Body.Bytes(), &callPayload)
	if callPayload["recorded"] != true {
		t.Fatalf("call record payload = %v", callPayload)
	}
	usageRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-usage?page_size=10", nil, token, "")
	if usageRec.Code != http.StatusOK {
		t.Fatalf("usage list status = %d, want %d, body=%s", usageRec.Code, http.StatusOK, usageRec.Body.String())
	}
	var usagePayload map[string]any
	decodeJSON(t, usageRec.Body.Bytes(), &usagePayload)
	if len(usagePayload["items"].([]any)) != 1 {
		t.Fatalf("usage payload = %v", usagePayload)
	}
	filteredUsageRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-usage?page_size=10&provider=openai-compatible&model=rewrite-model&user_id="+userID+"&sort=created_at_asc", nil, token, "")
	if filteredUsageRec.Code != http.StatusOK {
		t.Fatalf("filtered usage list status = %d, want %d, body=%s", filteredUsageRec.Code, http.StatusOK, filteredUsageRec.Body.String())
	}
	var filteredUsagePayload map[string]any
	decodeJSON(t, filteredUsageRec.Body.Bytes(), &filteredUsagePayload)
	if filteredUsagePayload["pagination"].(map[string]any)["total"].(float64) != 1 || len(filteredUsagePayload["items"].([]any)) != 1 {
		t.Fatalf("filtered usage payload = %v", filteredUsagePayload)
	}
	invalidUsageFilterRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-usage?sort=created_at%20desc", nil, token, "")
	if invalidUsageFilterRec.Code != http.StatusBadRequest {
		t.Fatalf("invalid usage filter status = %d, want %d, body=%s", invalidUsageFilterRec.Code, http.StatusBadRequest, invalidUsageFilterRec.Body.String())
	}

	leaseRecRepeat := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider":             "openai-compatible",
		"model":                "rewrite-model",
		"purpose":              "realtime_script",
		"max_duration_seconds": 300,
	}, clientToken, "idem-model-lease-create")
	if leaseRecRepeat.Code != http.StatusOK {
		t.Fatalf("lease repeat create status = %d, want %d, body=%s", leaseRecRepeat.Code, http.StatusOK, leaseRecRepeat.Body.String())
	}
	var leaseRepeatPayload map[string]any
	decodeJSON(t, leaseRecRepeat.Body.Bytes(), &leaseRepeatPayload)
	if leaseRepeatPayload["lease"].(map[string]any)["id"] != leaseID {
		t.Fatalf("repeat lease id mismatch: %v", leaseRepeatPayload)
	}

	renewRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+leaseID+"/renew", map[string]any{
		"extend_seconds": 120,
	}, clientToken, "idem-model-lease-renew")
	if renewRec.Code != http.StatusOK {
		t.Fatalf("lease renew status = %d, want %d, body=%s", renewRec.Code, http.StatusOK, renewRec.Body.String())
	}

	releaseRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+leaseID+"/release", map[string]any{
		"reason": "done",
	}, clientToken, "idem-model-lease-release")
	if releaseRec.Code != http.StatusOK {
		t.Fatalf("lease release status = %d, want %d, body=%s", releaseRec.Code, http.StatusOK, releaseRec.Body.String())
	}

	releaseAgainRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+leaseID+"/release", map[string]any{
		"reason": "again",
	}, clientToken, "idem-model-lease-release-2")
	if releaseAgainRec.Code != http.StatusOK {
		t.Fatalf("lease release repeat status = %d, want %d, body=%s", releaseAgainRec.Code, http.StatusOK, releaseAgainRec.Body.String())
	}

	renewAfterRelease := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+leaseID+"/renew", map[string]any{
		"extend_seconds": 60,
	}, clientToken, "idem-model-lease-renew-after-release")
	if renewAfterRelease.Code != http.StatusConflict {
		t.Fatalf("renew after release status = %d, want %d, body=%s", renewAfterRelease.Code, http.StatusConflict, renewAfterRelease.Body.String())
	}

	secondLeaseRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider": "openai-compatible",
		"model":    "rewrite-model",
		"purpose":  "realtime_script",
	}, clientToken, "idem-model-lease-create-second")
	if secondLeaseRec.Code != http.StatusOK {
		t.Fatalf("second lease create status = %d, want %d, body=%s", secondLeaseRec.Code, http.StatusOK, secondLeaseRec.Body.String())
	}
	var secondLeasePayload map[string]any
	decodeJSON(t, secondLeaseRec.Body.Bytes(), &secondLeasePayload)
	secondLeaseID := secondLeasePayload["lease"].(map[string]any)["id"].(string)

	renewWithoutBody := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+secondLeaseID+"/renew", nil, clientToken, "idem-model-lease-renew-empty")
	if renewWithoutBody.Code != http.StatusOK {
		t.Fatalf("renew without body status = %d, want %d, body=%s", renewWithoutBody.Code, http.StatusOK, renewWithoutBody.Body.String())
	}
	renewWithNullBody := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+secondLeaseID+"/renew", json.RawMessage("null"), clientToken, "idem-model-lease-renew-null")
	if renewWithNullBody.Code != http.StatusBadRequest {
		t.Fatalf("renew with null body status = %d, want %d, body=%s", renewWithNullBody.Code, http.StatusBadRequest, renewWithNullBody.Body.String())
	}
	releaseWithoutBody := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases/"+secondLeaseID+"/release", nil, clientToken, "idem-model-lease-release-empty")
	if releaseWithoutBody.Code != http.StatusOK {
		t.Fatalf("release without body status = %d, want %d, body=%s", releaseWithoutBody.Code, http.StatusOK, releaseWithoutBody.Body.String())
	}

	conflictCreate := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider":             "openai-compatible",
		"model":                "rewrite-model-v2",
		"purpose":              "realtime_script",
		"max_duration_seconds": 300,
	}, clientToken, "idem-model-lease-create")
	if conflictCreate.Code != http.StatusConflict {
		t.Fatalf("lease idempotency conflict status = %d, want %d, body=%s", conflictCreate.Code, http.StatusConflict, conflictCreate.Body.String())
	}
	var conflictPayload map[string]any
	decodeJSON(t, conflictCreate.Body.Bytes(), &conflictPayload)
	if conflictPayload["code"] != "IDEMPOTENCY_CONFLICT" {
		t.Fatalf("unexpected conflict code: %v", conflictPayload)
	}

	thirdLeaseRec := doJSON(t, handler, http.MethodPost, "/api/v1/client/model-leases", map[string]any{
		"provider": "openai-compatible",
		"model":    "rewrite-model",
		"purpose":  "realtime_script",
	}, clientToken, "idem-model-lease-create-third")
	if thirdLeaseRec.Code != http.StatusOK {
		t.Fatalf("third lease create status = %d, want %d, body=%s", thirdLeaseRec.Code, http.StatusOK, thirdLeaseRec.Body.String())
	}
	var thirdLeasePayload map[string]any
	decodeJSON(t, thirdLeaseRec.Body.Bytes(), &thirdLeasePayload)
	thirdLeaseID := thirdLeasePayload["lease"].(map[string]any)["id"].(string)
	detailRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-leases/"+thirdLeaseID, nil, token, "")
	if detailRec.Code != http.StatusOK {
		t.Fatalf("admin lease detail status = %d, want %d, body=%s", detailRec.Code, http.StatusOK, detailRec.Body.String())
	}
	var detailPayload map[string]any
	decodeJSON(t, detailRec.Body.Bytes(), &detailPayload)
	detailLease := detailPayload["lease"].(map[string]any)
	if detailLease["id"] != thirdLeaseID || detailLease["created_at"] == nil {
		t.Fatalf("admin lease detail = %v", detailPayload)
	}
	if _, leaked := detailLease["direct_base_url"]; leaked {
		t.Fatalf("admin lease detail leaked direct base URL: %v", detailLease)
	}
	reclaimRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-leases/"+thirdLeaseID+"/reclaim", map[string]any{"reason": "admin cleanup"}, token, "idem-admin-reclaim-lease")
	if reclaimRec.Code != http.StatusOK {
		t.Fatalf("admin lease reclaim status = %d, want %d, body=%s", reclaimRec.Code, http.StatusOK, reclaimRec.Body.String())
	}
	reclaimRepeatRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/model-leases/"+thirdLeaseID+"/reclaim", map[string]any{"reason": "admin cleanup"}, token, "idem-admin-reclaim-lease")
	if reclaimRepeatRec.Code != http.StatusOK {
		t.Fatalf("admin lease reclaim repeat status = %d, want %d, body=%s", reclaimRepeatRec.Code, http.StatusOK, reclaimRepeatRec.Body.String())
	}
	detailAfterRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/model-leases/"+thirdLeaseID, nil, token, "")
	var detailAfterPayload map[string]any
	decodeJSON(t, detailAfterRec.Body.Bytes(), &detailAfterPayload)
	if detailAfterPayload["lease"].(map[string]any)["status"] != "released" {
		t.Fatalf("admin lease detail after reclaim = %v", detailAfterPayload)
	}
}
