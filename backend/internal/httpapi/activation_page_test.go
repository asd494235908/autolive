package httpapi

import (
	"net/http"
	"testing"
)

func TestAdminActivationCodeListEndpointPaginatesAndRedacts(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	createRec := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "idem-activation-page-create")
	if createRec.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, body=%s", createRec.Code, createRec.Body.String())
	}
	listRec := doJSON(t, handler, http.MethodGet, "/api/v1/admin/activation-codes?page=1&page_size=1", nil, token, "")
	if listRec.Code != http.StatusOK {
		t.Fatalf("list activation codes status = %d, body=%s", listRec.Code, listRec.Body.String())
	}
	var payload struct {
		Items      []map[string]any `json:"items"`
		Pagination struct {
			Page     float64 `json:"page"`
			PageSize float64 `json:"page_size"`
			Total    float64 `json:"total"`
		} `json:"pagination"`
	}
	decodeJSON(t, listRec.Body.Bytes(), &payload)
	if len(payload.Items) != 1 || payload.Pagination.Page != 1 || payload.Pagination.PageSize != 1 || payload.Pagination.Total != 1 {
		t.Fatalf("activation pagination = %+v", payload)
	}
	if plainCode, ok := payload.Items[0]["plain_code"]; !ok || plainCode != nil {
		t.Fatalf("activation list leaked plaintext: %v", payload.Items[0])
	}
	if payload.Items[0]["max_devices"] != float64(1) {
		t.Fatalf("activation max_devices = %v", payload.Items[0]["max_devices"])
	}
}
