package httpapi

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

func newTestRouter(t *testing.T) http.Handler {
	t.Helper()
	// Keep activation-code tests deterministic while preserving the real router path.
	testNow := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return testNow })
	return NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin",
		Password: "password",
	}, repository, store.NewMemorySecretStore(), nil, true)
}

func testActivationExpiresAt() string {
	return time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC).Add(24 * time.Hour).Format(time.RFC3339)
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
