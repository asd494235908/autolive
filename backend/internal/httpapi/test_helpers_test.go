package httpapi

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func newTestRouter(t *testing.T) http.Handler {
	t.Helper()
	return NewRouterWithAuth("test", nil, AuthConfig{
		Username: "admin",
		Password: "password",
	})
}

func loginForTest(t *testing.T, handler http.Handler) string {
	t.Helper()
	return loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"password"}`)
}

func doJSON(t *testing.T, handler http.Handler, method, path string, body any, token, idempotencyKey string) *httptest.ResponseRecorder {
	t.Helper()

	var payload []byte
	if body != nil {
		var err error
		payload, err = json.Marshal(body)
		if err != nil {
			t.Fatalf("marshal request body: %v", err)
		}
	}
	req := httptest.NewRequest(method, path, bytes.NewReader(payload))
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if idempotencyKey != "" {
		req.Header.Set("Idempotency-Key", idempotencyKey)
	}
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	return rec
}

func decodeJSON(t *testing.T, data []byte, target any) {
	t.Helper()
	if err := json.Unmarshal(data, target); err != nil {
		t.Fatalf("decode JSON: %v; body=%s", err, data)
	}
}
