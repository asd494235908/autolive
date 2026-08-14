package httpapi

import (
	"net/http"
	"testing"
)

func TestAdminAuditLogsCaptureAuthenticatedMutationWithoutRequestBody(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)

	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  "2026-08-14T11:00:00Z",
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
	first := items[0].(map[string]any)
	if first["actor_user_id"] != "usr_local_admin" {
		t.Fatalf("actor_user_id = %v, want usr_local_admin", first["actor_user_id"])
	}
	if first["action"] != "POST /api/v1/admin/activation-codes" {
		t.Fatalf("action = %v", first["action"])
	}
	if first["request_id"] == nil || first["request_id"] == "" {
		t.Fatalf("request_id missing from audit item: %v", first)
	}
}
