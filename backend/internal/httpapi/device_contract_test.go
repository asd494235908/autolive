package httpapi

import (
	"encoding/json"
	"net/http"
	"testing"
)

func TestUnbindDeviceUsesUnboundResponseWithoutUserID(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	clientToken, userID := createDesktopUserForTest(t, handler, token, "unbind-contract-user")

	create := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"user_id":     userID,
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "unbind-contract-code")
	if create.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d; body=%s", create.Code, http.StatusCreated, create.Body.String())
	}
	activate := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_unbind1",
			"device_name": "Contract Device",
			"platform":    "windows",
			"app_version": "1.0.0",
		},
	}, clientToken, "unbind-contract-activate")
	if activate.Code != http.StatusOK {
		t.Fatalf("activate status = %d, want %d; body=%s", activate.Code, http.StatusOK, activate.Body.String())
	}

	unbind := doJSON(t, handler, http.MethodPost, "/api/v1/admin/devices/dev_unbind1/unbind", nil, token, "unbind-contract-request")
	if unbind.Code != http.StatusOK {
		t.Fatalf("unbind status = %d, want %d; body=%s", unbind.Code, http.StatusOK, unbind.Body.String())
	}
	var payload map[string]any
	decodeJSON(t, unbind.Body.Bytes(), &payload)
	if _, exists := payload["device"]; exists {
		t.Fatalf("unbind response must not use DeviceEnvelope: %s", unbind.Body.String())
	}
	if payload["device_id"] != "dev_unbind1" || payload["status"] != "pending_activation" {
		t.Fatalf("unbind response = %s", unbind.Body.String())
	}
	if _, exists := payload["user_id"]; exists {
		t.Fatalf("unbind response leaked an empty user_id field: %s", unbind.Body.String())
	}
	if !json.Valid(unbind.Body.Bytes()) {
		t.Fatal("unbind response is not valid JSON")
	}
}

func TestDeviceActivationAuditUsesDeviceTargetWithoutPlainCode(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	clientToken, userID := createDesktopUserForTest(t, handler, token, "audit-activation-user")
	create := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"user_id":     userID,
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "audit-activation-code")
	if create.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d; body=%s", create.Code, http.StatusCreated, create.Body.String())
	}
	activate := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_audit_activation",
			"device_name": "Audit Device",
			"platform":    "windows",
			"app_version": "1.0.0",
		},
	}, clientToken, "audit-activation-request")
	if activate.Code != http.StatusOK {
		t.Fatalf("activate status = %d, want %d; body=%s", activate.Code, http.StatusOK, activate.Body.String())
	}

	audit := doJSON(t, handler, http.MethodGet, "/api/v1/admin/audit-logs?action=POST%20%2Fapi%2Fv1%2Fclient%2Factivate&target_type=device", nil, token, "")
	if audit.Code != http.StatusOK {
		t.Fatalf("audit list status = %d, want %d; body=%s", audit.Code, http.StatusOK, audit.Body.String())
	}
	var payload struct {
		Items []map[string]any `json:"items"`
	}
	decodeJSON(t, audit.Body.Bytes(), &payload)
	if len(payload.Items) == 0 {
		t.Fatalf("device activation audit missing: %s", audit.Body.String())
	}
	for _, item := range payload.Items {
		if item["target_id"] != "dev_audit_activation" || item["outcome"] != "success" || item["status_code"] != float64(http.StatusOK) {
			t.Fatalf("device activation audit semantics = %v", item)
		}
		if _, leaked := item["activation_code"]; leaked {
			t.Fatalf("device activation audit leaked activation code: %v", item)
		}
	}
}

func TestRevokeFullAccountAuthorizationAllowsUsedStatus(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	clientToken, userID := createDesktopUserForTest(t, handler, token, "revoke-full-user")
	create := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"user_id":     userID,
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "revoke-full-code")
	if create.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d; body=%s", create.Code, http.StatusCreated, create.Body.String())
	}
	var created activationCodeEnvelope
	decodeJSON(t, create.Body.Bytes(), &created)
	activate := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_revoke_full",
			"device_name": "Revoked Device",
			"platform":    "windows",
			"app_version": "1.0.0",
		},
	}, clientToken, "revoke-full-activate")
	if activate.Code != http.StatusOK {
		t.Fatalf("activate status = %d, want %d; body=%s", activate.Code, http.StatusOK, activate.Body.String())
	}
	revoke := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes/"+created.ActivationCode.ID+"/revoke", nil, token, "revoke-full-request")
	if revoke.Code != http.StatusOK {
		t.Fatalf("revoke used authorization status = %d, want %d; body=%s", revoke.Code, http.StatusOK, revoke.Body.String())
	}
	var revoked activationCodeEnvelope
	decodeJSON(t, revoke.Body.Bytes(), &revoked)
	if revoked.ActivationCode.Status != "revoked" {
		t.Fatalf("revoke used authorization response = %+v", revoked.ActivationCode)
	}
}
