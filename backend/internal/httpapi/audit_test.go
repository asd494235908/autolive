package httpapi

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestAuditTargetExtractsResourceIDOutsideServeMux(t *testing.T) {
	for _, test := range []struct {
		path       string
		wantType   string
		wantTarget string
	}{
		{path: "/api/v1/admin/users/usr_00000001/devices", wantType: "user", wantTarget: "usr_00000001"},
		{path: "/api/v1/admin/model-pool/mpa_00000001/test", wantType: "model_account", wantTarget: "mpa_00000001"},
		{path: "/api/v1/client/model-leases/lease_00000001/release", wantType: "model_lease", wantTarget: "lease_00000001"},
	} {
		req := httptest.NewRequest(http.MethodPost, test.path, nil)
		gotType, gotTarget := auditTarget(req)
		if gotType != test.wantType || gotTarget != test.wantTarget {
			t.Fatalf("auditTarget(%q) = (%q, %q), want (%q, %q)", test.path, gotType, gotTarget, test.wantType, test.wantTarget)
		}
	}
}

func TestAdminAuditLogsCaptureAuthenticatedMutationWithoutRequestBody(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)

	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "audit-create-code")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d, body=%s", createRec.Code, http.StatusCreated, createRec.Body.String())
	}

	listRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs", nil, token, "")
	if listRec.Code != http.StatusOK {
		t.Fatalf("list audit logs status = %d, want %d, body=%s", listRec.Code, http.StatusOK, listRec.Body.String())
	}
	var payload map[string]any
	decodeJSON(t, listRec.Body.Bytes(), &payload)
	items := payload["items"].([]any)
	if len(items) == 0 {
		t.Fatal("audit log list is empty after authenticated mutation")
	}
	var first map[string]any
	for _, item := range items {
		candidate := item.(map[string]any)
		if candidate["action"] == "POST /api/v1/admin/activation-codes" {
			first = candidate
			break
		}
	}
	if first == nil {
		t.Fatalf("activation-code audit item missing: %v", items)
	}
	if first["actor_user_id"] != "usr_local_admin" {
		t.Fatalf("actor_user_id = %v, want usr_local_admin", first["actor_user_id"])
	}
	if first["target_type"] != "activation_code" || first["outcome"] != "success" || first["status_code"] != float64(http.StatusCreated) {
		t.Fatalf("audit semantics = %v", first)
	}
	if first["request_id"] == nil || first["request_id"] == "" {
		t.Fatalf("request_id missing from audit item: %v", first)
	}
	filteredRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs?actor_user_id=usr_local_admin&outcome=success&target_type=activation_code&sort=created_at_asc", nil, token, "")
	if filteredRec.Code != http.StatusOK {
		t.Fatalf("filtered audit logs status = %d, want %d, body=%s", filteredRec.Code, http.StatusOK, filteredRec.Body.String())
	}
	var filteredPayload map[string]any
	decodeJSON(t, filteredRec.Body.Bytes(), &filteredPayload)
	if filteredPayload["pagination"].(map[string]any)["total"].(float64) < 1 || len(filteredPayload["items"].([]any)) < 1 {
		t.Fatalf("filtered audit logs = %v", filteredPayload)
	}
	invalidRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs?outcome=partial", nil, token, "")
	if invalidRec.Code != http.StatusBadRequest {
		t.Fatalf("invalid audit filter status = %d, want %d, body=%s", invalidRec.Code, http.StatusBadRequest, invalidRec.Body.String())
	}
}

func TestLoginAuditCapturesSuccessAndFailureWithoutCredentials(t *testing.T) {
	handler := newTestRouter(t)

	failed := doJSON(t, handler, http.MethodPost, "/api/v1/auth/login", map[string]any{
		"username": "admin",
		"password": "wrong-password",
		"product":  "autolive",
	}, "", "")
	if failed.Code != http.StatusUnauthorized {
		t.Fatalf("failed login status = %d, want %d", failed.Code, http.StatusUnauthorized)
	}
	succeeded := doJSON(t, handler, http.MethodPost, "/api/v1/auth/login", map[string]any{
		"username": "admin",
		"password": "password",
		"product":  "autolive",
	}, "", "")
	if succeeded.Code != http.StatusOK {
		t.Fatalf("successful login status = %d, want %d", succeeded.Code, http.StatusOK)
	}

	listRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs", nil, loginForTest(t, handler), "")
	if listRec.Code != http.StatusOK {
		t.Fatalf("list audit logs status = %d, want %d, body=%s", listRec.Code, http.StatusOK, listRec.Body.String())
	}
	var payload map[string]any
	decodeJSON(t, listRec.Body.Bytes(), &payload)
	items := payload["items"].([]any)
	var foundFailure, foundSuccess bool
	for _, item := range items {
		entry := item.(map[string]any)
		if entry["action"] != "POST /api/v1/auth/login" {
			continue
		}
		if entry["outcome"] == "failure" && entry["status_code"] == float64(http.StatusUnauthorized) && entry["error_code"] == "UNAUTHENTICATED" {
			foundFailure = true
		}
		if entry["outcome"] == "success" && entry["status_code"] == float64(http.StatusOK) && entry["actor_user_id"] == "usr_local_admin" {
			foundSuccess = true
		}
		if _, leaked := entry["password"]; leaked {
			t.Fatalf("audit entry contains password field: %v", entry)
		}
	}
	if !foundFailure || !foundSuccess {
		t.Fatalf("login audit entries missing failure=%t success=%t: %v", foundFailure, foundSuccess, items)
	}
}
