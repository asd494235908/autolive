package httpapi

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

const testAdminPassword = "correct-password"

func newTestRouter(t *testing.T) http.Handler {
	t.Helper()
	// Keep activation-code tests deterministic while preserving the real router path.
	testNow := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return testNow })
	return NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin",
		Password: testAdminPassword,
	}, repository, store.NewMemorySecretStore(), nil, true)
}

func testActivationExpiresAt() string {
	return time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC).Add(24 * time.Hour).Format(time.RFC3339)
}

func loginForTest(t *testing.T, handler http.Handler) string {
	t.Helper()
	return loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)
}

func createDesktopUserForTest(t *testing.T, handler http.Handler, adminToken, username string) (string, string) {
	return createDesktopUserForProductForTest(t, handler, adminToken, username, controlplane.ProductAutoLive)
}

func createDesktopUserForProductForTest(t *testing.T, handler http.Handler, adminToken, username string, product controlplane.ProductCode) (string, string) {
	t.Helper()
	password := "desktop-test-password"
	created := doJSON(t, handler, http.MethodPost, "/api/v1/admin/users", map[string]any{
		"username": username,
		"password": password,
		"role":     "user",
	}, adminToken, "create-"+username)
	if created.Code != http.StatusCreated {
		t.Fatalf("create desktop test user status = %d; body=%s", created.Code, created.Body.String())
	}
	var payload userEnvelope
	decodeJSON(t, created.Body.Bytes(), &payload)
	token := loginWithCredentialsAtPathForTest(t, handler, "/api/v1/client/auth/login", `{"username":"`+username+`","password":"`+password+`","product":"`+string(product)+`"}`)
	return token, payload.User.ID
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
