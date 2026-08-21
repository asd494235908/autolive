package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

func TestLoginReturnsContractResponse(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})

	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"correct-password","product":"autolive"}`))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}

	var payload loginResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}

	if payload.RequestID == "" || payload.Tokens.AccessToken == "" || payload.Tokens.RefreshToken == "" {
		t.Fatalf("login response missing identifiers: %+v", payload)
	}

	if payload.User.Username != "admin" || payload.User.Role != "admin" || payload.User.Status != "active" {
		t.Fatalf("unexpected user summary: %+v", payload.User)
	}
}

func TestLoginStoresOnlyAccessTokenHashInSessionIndex(t *testing.T) {
	auth := newAuthenticator(service.NewControlPlane(store.NewMemoryStore(time.Now)), AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	request := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password","product":"autolive"}`,
	))
	recorder := httptest.NewRecorder()
	loginHandler(auth).ServeHTTP(recorder, request)
	if recorder.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", recorder.Code, http.StatusOK, recorder.Body.String())
	}

	var payload loginResponse
	if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode login response: %v", err)
	}

	auth.mu.Lock()
	defer auth.mu.Unlock()
	if _, ok := auth.sessions[payload.Tokens.AccessToken]; ok {
		t.Fatal("raw access token must not be used as a session map key")
	}
	if _, ok := auth.sessions[hashToken(payload.Tokens.AccessToken)]; !ok {
		t.Fatal("hashed access token must be used as a session map key")
	}
	for _, session := range auth.sessions {
		if session.AccessTokenHash == payload.Tokens.AccessToken {
			t.Fatal("raw access token must not be stored in session record")
		}
	}
}

func TestLoginRejectsInvalidCredentials(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})

	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"wrong-password","product":"autolive"}`))
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestConfiguredAdminPasswordIsPersistedAcrossRouterRecreation(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	first := NewRouterWithRepository("v1.0.0", nil, AuthConfig{Username: "admin", Password: "first-password"}, repository)
	if response := doLoginRequest(t, first, "admin", "first-password"); response.Code != http.StatusOK {
		t.Fatalf("first login status = %d; body=%s", response.Code, response.Body.String())
	}

	second := NewRouterWithRepository("v1.0.0", nil, AuthConfig{Username: "admin", Password: "second-password"}, repository)
	if response := doLoginRequest(t, second, "admin", "first-password"); response.Code != http.StatusOK {
		t.Fatalf("persisted password login status = %d; body=%s", response.Code, response.Body.String())
	}
	if response := doLoginRequest(t, second, "admin", "second-password"); response.Code != http.StatusUnauthorized {
		t.Fatalf("replacement password status = %d; body=%s", response.Code, response.Body.String())
	}

	persisted := NewRouterWithRepository("v1.0.0", nil, AuthConfig{UsePersistedAdmin: true}, repository)
	if response := doLoginRequest(t, persisted, "admin", "first-password"); response.Code != http.StatusOK {
		t.Fatalf("persisted-admin-only login status = %d; body=%s", response.Code, response.Body.String())
	}
}

func TestLocalAdminPasswordChangeRevokesSessionsAndSupportsIdempotentRetry(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "first-password",
	})
	oldToken := loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"first-password","product":"autolive"}`)
	changeRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/auth/change-password", strings.NewReader(`{"password":"rotated-password"}`))
	changeRequest.Header.Set("Authorization", "Bearer "+oldToken)
	changeRequest.Header.Set("Idempotency-Key", "rotate-local-admin")
	changeRecorder := httptest.NewRecorder()
	handler.ServeHTTP(changeRecorder, changeRequest)
	if changeRecorder.Code != http.StatusOK {
		t.Fatalf("change password status = %d, want %d; body=%s", changeRecorder.Code, http.StatusOK, changeRecorder.Body.String())
	}

	oldSessionRequest := httptest.NewRequest(http.MethodGet, "/api/v1/admin/users", nil)
	oldSessionRequest.Header.Set("Authorization", "Bearer "+oldToken)
	oldSessionRecorder := httptest.NewRecorder()
	handler.ServeHTTP(oldSessionRecorder, oldSessionRequest)
	if oldSessionRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("old local admin session status = %d, want %d", oldSessionRecorder.Code, http.StatusUnauthorized)
	}
	if response := doLoginRequest(t, handler, "admin", "first-password"); response.Code != http.StatusUnauthorized {
		t.Fatalf("old local admin password status = %d, want %d", response.Code, http.StatusUnauthorized)
	}
	newToken := loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"rotated-password","product":"autolive"}`)
	if newToken == "" {
		t.Fatal("rotated local admin password did not create a session")
	}

	retryRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/auth/change-password", strings.NewReader(`{"password":"rotated-password"}`))
	retryRequest.Header.Set("Authorization", "Bearer "+newToken)
	retryRequest.Header.Set("Idempotency-Key", "rotate-local-admin")
	retryRecorder := httptest.NewRecorder()
	handler.ServeHTTP(retryRecorder, retryRequest)
	if retryRecorder.Code != http.StatusOK {
		t.Fatalf("idempotent change password status = %d, want %d; body=%s", retryRecorder.Code, http.StatusOK, retryRecorder.Body.String())
	}
}

func doLoginRequest(t *testing.T, handler http.Handler, username, password string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"`+username+`","password":"`+password+`","product":"autolive"}`))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	return rec
}

func TestLoginReportsMissingDevelopmentConfiguration(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"password","product":"autolive"}`))
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusServiceUnavailable)
	}
}

func TestCreatedUserCanLoginWithTheSameControlPlaneCredentials(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})

	adminToken := loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)
	createRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/users", strings.NewReader(`{"username":"client-a","password":"client-password","role":"user"}`))
	createRequest.Header.Set("Authorization", "Bearer "+adminToken)
	createRequest.Header.Set("Idempotency-Key", "create-client-a")
	createRecorder := httptest.NewRecorder()
	handler.ServeHTTP(createRecorder, createRequest)
	if createRecorder.Code != http.StatusCreated {
		t.Fatalf("create user status = %d, want %d; body=%s", createRecorder.Code, http.StatusCreated, createRecorder.Body.String())
	}

	clientToken := loginWithCredentialsForTest(t, handler, `{"username":"client-a","password":"client-password","product":"autolive"}`)
	if clientToken == "" || clientToken == adminToken {
		t.Fatalf("expected a distinct client access token")
	}
}

func TestResetUserPasswordRevokesExistingSessions(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	adminToken := loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)
	createRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/users", strings.NewReader(`{"username":"client-b","password":"client-password","role":"user"}`))
	createRequest.Header.Set("Authorization", "Bearer "+adminToken)
	createRequest.Header.Set("Idempotency-Key", "create-client-b")
	createRecorder := httptest.NewRecorder()
	handler.ServeHTTP(createRecorder, createRequest)
	if createRecorder.Code != http.StatusCreated {
		t.Fatalf("create user status = %d, want %d; body=%s", createRecorder.Code, http.StatusCreated, createRecorder.Body.String())
	}
	clientToken := loginWithCredentialsForTest(t, handler, `{"username":"client-b","password":"client-password","product":"autolive"}`)

	resetRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/users/usr_00000001/reset-password", strings.NewReader(`{"password":"changed-password"}`))
	resetRequest.Header.Set("Authorization", "Bearer "+adminToken)
	resetRequest.Header.Set("Idempotency-Key", "reset-client-b")
	resetRecorder := httptest.NewRecorder()
	handler.ServeHTTP(resetRecorder, resetRequest)
	if resetRecorder.Code != http.StatusOK {
		t.Fatalf("reset password status = %d, want %d; body=%s", resetRecorder.Code, http.StatusOK, resetRecorder.Body.String())
	}

	oldSessionRequest := httptest.NewRequest(http.MethodGet, "/api/v1/client/profile", nil)
	oldSessionRequest.Header.Set("Authorization", "Bearer "+clientToken)
	oldSessionRecorder := httptest.NewRecorder()
	handler.ServeHTTP(oldSessionRecorder, oldSessionRequest)
	if oldSessionRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("old session status = %d, want %d", oldSessionRecorder.Code, http.StatusUnauthorized)
	}
	if loginWithCredentialsForTest(t, handler, `{"username":"client-b","password":"changed-password","product":"autolive"}`) == "" {
		t.Fatal("new password did not create a session")
	}
}

func TestRefreshRotatesTokensAndRevokesPreviousAccessToken(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)

	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(
		`{"refresh_token":"`+initial.RefreshToken+`"}`,
	))
	refreshRecorder := httptest.NewRecorder()
	handler.ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusOK {
		t.Fatalf("refresh status = %d, want %d; body=%s", refreshRecorder.Code, http.StatusOK, refreshRecorder.Body.String())
	}

	var refreshed refreshTokenResponse
	if err := json.Unmarshal(refreshRecorder.Body.Bytes(), &refreshed); err != nil {
		t.Fatalf("decode refresh response: %v", err)
	}
	if refreshed.Tokens.AccessToken == initial.AccessToken || refreshed.Tokens.RefreshToken == initial.RefreshToken {
		t.Fatalf("refresh must rotate both tokens: initial=%+v refreshed=%+v", initial, refreshed.Tokens)
	}

	oldAccessRequest := httptest.NewRequest(http.MethodGet, "/api/v1/admin/users", nil)
	oldAccessRequest.Header.Set("Authorization", "Bearer "+initial.AccessToken)
	oldAccessRecorder := httptest.NewRecorder()
	handler.ServeHTTP(oldAccessRecorder, oldAccessRequest)
	if oldAccessRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("old access status = %d, want %d", oldAccessRecorder.Code, http.StatusUnauthorized)
	}

	newAccessRequest := httptest.NewRequest(http.MethodGet, "/api/v1/admin/users", nil)
	newAccessRequest.Header.Set("Authorization", "Bearer "+refreshed.Tokens.AccessToken)
	newAccessRecorder := httptest.NewRecorder()
	handler.ServeHTTP(newAccessRecorder, newAccessRequest)
	if newAccessRecorder.Code != http.StatusOK {
		t.Fatalf("new access status = %d, want %d; body=%s", newAccessRecorder.Code, http.StatusOK, newAccessRecorder.Body.String())
	}
}

func TestRefreshRejectsReusedRefreshTokenAfterRotation(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)
	refreshBody := `{"refresh_token":"` + initial.RefreshToken + `"}`

	firstRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(refreshBody))
	firstRecorder := httptest.NewRecorder()
	handler.ServeHTTP(firstRecorder, firstRequest)
	if firstRecorder.Code != http.StatusOK {
		t.Fatalf("first refresh status = %d, want %d", firstRecorder.Code, http.StatusOK)
	}

	secondRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(refreshBody))
	secondRecorder := httptest.NewRecorder()
	handler.ServeHTTP(secondRecorder, secondRequest)
	if secondRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("reused refresh status = %d, want %d", secondRecorder.Code, http.StatusUnauthorized)
	}
}

func TestLoginBindsProductAndRefreshIgnoresProductReplacement(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	auth := newAuthenticator(service.NewControlPlane(repository), AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})

	loginRecorder := httptest.NewRecorder()
	loginRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password","product":"douyin_desktop"}`,
	))
	loginHandler(auth).ServeHTTP(loginRecorder, loginRequest)
	if loginRecorder.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", loginRecorder.Code, http.StatusOK, loginRecorder.Body.String())
	}
	var login loginResponse
	if err := json.Unmarshal(loginRecorder.Body.Bytes(), &login); err != nil {
		t.Fatalf("decode login response: %v", err)
	}
	if got := auth.sessions[hashToken(login.Tokens.AccessToken)].Actor.Product; got != controlplane.ProductDouyinDesktop {
		t.Fatalf("session product = %q, want %q", got, controlplane.ProductDouyinDesktop)
	}

	refreshRecorder := httptest.NewRecorder()
	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh?product=autolive", strings.NewReader(
		`{"refresh_token":"`+login.Tokens.RefreshToken+`"}`,
	))
	refreshRequest.Header.Set("X-Client-Product", string(controlplane.ProductAutoLive))
	refreshHandler(auth).ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusOK {
		t.Fatalf("refresh status = %d, want %d; body=%s", refreshRecorder.Code, http.StatusOK, refreshRecorder.Body.String())
	}
	var refreshed refreshTokenResponse
	if err := json.Unmarshal(refreshRecorder.Body.Bytes(), &refreshed); err != nil {
		t.Fatalf("decode refresh response: %v", err)
	}
	if got := auth.sessions[hashToken(refreshed.Tokens.AccessToken)].Actor.Product; got != controlplane.ProductDouyinDesktop {
		t.Fatalf("rotated session product = %q, want %q", got, controlplane.ProductDouyinDesktop)
	}
}

type loginProductRepositoryStub struct {
	membership controlplane.UserProductMembership
	err        error
}

func (s loginProductRepositoryStub) ListProducts(context.Context) ([]controlplane.ProductSummary, error) {
	return nil, nil
}

func (s loginProductRepositoryStub) GetProduct(context.Context, controlplane.ProductCode) (controlplane.ProductSummary, error) {
	return controlplane.ProductSummary{}, nil
}

func (s loginProductRepositoryStub) GetUserProductMembership(context.Context, string, controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	return s.membership, s.err
}

func (s loginProductRepositoryStub) EnsureUserProductMembership(context.Context, string, controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	return controlplane.UserProductMembership{}, errors.New("EnsureUserProductMembership must not be called during login")
}

func TestLoginRejectsMissingOrDisabledProductMembership(t *testing.T) {
	for _, test := range []struct {
		name       string
		membership controlplane.UserProductMembership
		err        error
	}{
		{name: "missing", err: controlplane.ErrForbidden},
		{name: "disabled", membership: controlplane.UserProductMembership{Status: "disabled"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			repository := store.NewMemoryStore(time.Now)
			auth := newAuthenticatorWithProductRepository(service.NewControlPlane(repository), AuthConfig{Username: "admin", Password: "correct-password"}, loginProductRepositoryStub{membership: test.membership, err: test.err})
			recorder := httptest.NewRecorder()
			request := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"correct-password","product":"douyin_desktop"}`))
			loginHandler(auth).ServeHTTP(recorder, request)
			if recorder.Code != http.StatusForbidden {
				t.Fatalf("login status = %d, want %d; body=%s", recorder.Code, http.StatusForbidden, recorder.Body.String())
			}
		})
	}
}

func TestLoginRequiresProductUnlessExplicitLegacyCompatibility(t *testing.T) {
	auth := newAuthenticator(service.NewControlPlane(store.NewMemoryStore(time.Now)), AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})

	strictRecorder := httptest.NewRecorder()
	strictRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password"}`,
	))
	loginHandler(auth).ServeHTTP(strictRecorder, strictRequest)
	if strictRecorder.Code != http.StatusBadRequest {
		t.Fatalf("strict login status = %d, want %d", strictRecorder.Code, http.StatusBadRequest)
	}
	var strictError ErrorResponse
	if err := json.Unmarshal(strictRecorder.Body.Bytes(), &strictError); err != nil {
		t.Fatalf("decode strict login error: %v", err)
	}
	if strictError.Code != "INVALID_REQUEST" {
		t.Fatalf("strict login error code = %q, want INVALID_REQUEST", strictError.Code)
	}

	invalidRecorder := httptest.NewRecorder()
	invalidRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password","product":"unknown"}`,
	))
	loginHandler(auth).ServeHTTP(invalidRecorder, invalidRequest)
	if invalidRecorder.Code != http.StatusBadRequest {
		t.Fatalf("invalid product login status = %d, want %d", invalidRecorder.Code, http.StatusBadRequest)
	}

	legacyRecorder := httptest.NewRecorder()
	legacyRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password"}`,
	))
	legacyRequest.Header.Set("X-Client-Compatibility", "legacy")
	loginHandler(auth).ServeHTTP(legacyRecorder, legacyRequest)
	if legacyRecorder.Code != http.StatusOK {
		t.Fatalf("legacy login status = %d, want %d; body=%s", legacyRecorder.Code, http.StatusOK, legacyRecorder.Body.String())
	}
	var legacy loginResponse
	if err := json.Unmarshal(legacyRecorder.Body.Bytes(), &legacy); err != nil {
		t.Fatalf("decode legacy login response: %v", err)
	}
	if got := auth.sessions[hashToken(legacy.Tokens.AccessToken)].Actor.Product; got != controlplane.ProductAutoLive {
		t.Fatalf("legacy session product = %q, want %q", got, controlplane.ProductAutoLive)
	}
}

func TestRequireBearerRestoresPersistedSessionProduct(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	sessionStore := newTestSessionStore()
	first := newAuthenticator(service.NewControlPlane(repository), AuthConfig{
		Username: "admin",
		Password: "correct-password",
	}, sessionStore)
	loginRecorder := httptest.NewRecorder()
	loginRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password","product":"douyin_desktop"}`,
	))
	loginHandler(first).ServeHTTP(loginRecorder, loginRequest)
	if loginRecorder.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d", loginRecorder.Code, http.StatusOK)
	}
	var login loginResponse
	if err := json.Unmarshal(loginRecorder.Body.Bytes(), &login); err != nil {
		t.Fatalf("decode login response: %v", err)
	}

	second := newAuthenticator(service.NewControlPlane(repository), AuthConfig{UsePersistedAdmin: true}, sessionStore)
	var actor controlplane.Actor
	record := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodGet, "/api/v1/client/profile", nil)
	request.Header.Set("Authorization", "Bearer "+login.Tokens.AccessToken)
	second.requireBearer(func(w http.ResponseWriter, _ *http.Request, got controlplane.Actor) {
		actor = got
		w.WriteHeader(http.StatusNoContent)
	}).ServeHTTP(record, request)
	if record.Code != http.StatusNoContent {
		t.Fatalf("requireBearer status = %d, want %d; body=%s", record.Code, http.StatusNoContent, record.Body.String())
	}
	if actor.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("restored actor product = %q, want %q", actor.Product, controlplane.ProductDouyinDesktop)
	}
}

func TestProductMismatchIsRejectedBeforeActivationOrHeartbeatBinding(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	sessionStore := newTestSessionStore()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	}, repository, store.NewMemorySecretStore(), sessionStore, true)
	login := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"douyin_desktop"}`)

	activation := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"activation_code": "code_01234567",
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_product01",
			"device_name": "desktop",
			"platform":    "windows",
			"app_version": "1.0.0",
		},
	}, login.AccessToken, "activate-product-mismatch")
	if activation.Code != http.StatusForbidden {
		t.Fatalf("activation mismatch status = %d, want %d; body=%s", activation.Code, http.StatusForbidden, activation.Body.String())
	}

	heartbeat := doJSON(t, handler, http.MethodPost, "/api/v1/client/heartbeat", map[string]any{
		"product":   "autolive",
		"device_id": "dev_product01",
		"sent_at":   time.Now().UTC().Format(time.RFC3339),
		"status":    map[string]any{"disk_free_bytes": 1024},
	}, login.AccessToken, "heartbeat-product-mismatch")
	if heartbeat.Code != http.StatusForbidden {
		t.Fatalf("heartbeat mismatch status = %d, want %d; body=%s", heartbeat.Code, http.StatusForbidden, heartbeat.Body.String())
	}

	sessionStore.mu.Lock()
	defer sessionStore.mu.Unlock()
	if session := sessionStore.byAccess[hashToken(login.AccessToken)]; session.DeviceID != "" {
		t.Fatalf("mismatched product requests bound device %q", session.DeviceID)
	}
}

func TestRevokeDeviceSessionsForProductDoesNotRevokeOtherProduct(t *testing.T) {
	sessions := newTestSessionStore()
	now := time.Now().UTC()
	for _, session := range []store.AuthSession{
		{ID: "session-auto", UserID: "user-1", Product: controlplane.ProductAutoLive, DeviceID: "device-shared", AccessTokenHash: "access-auto", RefreshTokenHash: "refresh-auto", AccessExpiresAt: now.Add(time.Hour), RefreshExpiresAt: now.Add(2 * time.Hour)},
		{ID: "session-douyin", UserID: "user-1", Product: controlplane.ProductDouyinDesktop, DeviceID: "device-shared", AccessTokenHash: "access-douyin", RefreshTokenHash: "refresh-douyin", AccessExpiresAt: now.Add(time.Hour), RefreshExpiresAt: now.Add(2 * time.Hour)},
	} {
		if err := sessions.Create(context.Background(), session); err != nil {
			t.Fatalf("Create() error = %v", err)
		}
	}
	auth := newAuthenticator(service.NewControlPlane(store.NewMemoryStore(time.Now)), AuthConfig{}, sessions)
	if err := auth.revokeDeviceSessionsForProduct(context.Background(), "device-shared", controlplane.ProductDouyinDesktop); err != nil {
		t.Fatalf("revokeDeviceSessionsForProduct() error = %v", err)
	}
	sessions.mu.Lock()
	defer sessions.mu.Unlock()
	if sessions.revoked["session-auto"] {
		t.Fatal("autolive session was revoked by douyin device lifecycle")
	}
	if !sessions.revoked["session-douyin"] {
		t.Fatal("douyin session was not revoked")
	}
}

func TestClientProfileReturnsActorAndDeviceProduct(t *testing.T) {
	handler := newTestRouter(t)
	token := loginForTest(t, handler)
	create := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  testActivationExpiresAt(),
		"max_devices": 1,
	}, token, "profile-product-code")
	if create.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d, want %d; body=%s", create.Code, http.StatusCreated, create.Body.String())
	}
	var codePayload struct {
		ActivationCode struct {
			PlainCode string `json:"plain_code"`
		} `json:"activation_code"`
	}
	decodeJSON(t, create.Body.Bytes(), &codePayload)
	activate := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{
		"activation_code": codePayload.ActivationCode.PlainCode,
		"device": map[string]any{
			"product":     "autolive",
			"device_id":   "dev_profile_product",
			"device_name": "Profile Device",
			"platform":    "macOS",
			"app_version": "1.0.0",
		},
	}, token, "profile-product-activate")
	if activate.Code != http.StatusOK {
		t.Fatalf("activate status = %d, want %d; body=%s", activate.Code, http.StatusOK, activate.Body.String())
	}

	profile := doJSON(t, handler, http.MethodGet, "/api/v1/client/profile", nil, token, "")
	if profile.Code != http.StatusOK {
		t.Fatalf("profile status = %d, want %d; body=%s", profile.Code, http.StatusOK, profile.Body.String())
	}
	var payload clientProfileResponse
	decodeJSON(t, profile.Body.Bytes(), &payload)
	if payload.Product != controlplane.ProductAutoLive || payload.Device.Product != controlplane.ProductAutoLive {
		t.Fatalf("profile product = (%q, %q), want (%q, %q)", payload.Product, payload.Device.Product, controlplane.ProductAutoLive, controlplane.ProductAutoLive)
	}
}

func TestClientProfileRejectsBoundDeviceFromAnotherProductWithoutMutation(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	sessions := newTestSessionStore()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin",
		Password: "password",
	}, repository, store.NewMemorySecretStore(), sessions, true)
	token := loginForTest(t, handler)
	deviceID := "dev_profile_mismatch"
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices[deviceID] = controlplane.DeviceSummary{
			ID: deviceID, UserID: "usr_local_admin", Product: controlplane.ProductDouyinDesktop,
			Status: controlplane.DeviceStatusActive,
		}
		return nil
	}); err != nil {
		t.Fatalf("seed mismatched device: %v", err)
	}
	if err := sessions.UpdateDeviceID(context.Background(), hashToken(token), deviceID); err != nil {
		t.Fatalf("bind session device: %v", err)
	}

	profile := doJSON(t, handler, http.MethodGet, "/api/v1/client/profile", nil, token, "")
	if profile.Code != http.StatusForbidden {
		t.Fatalf("profile mismatch status = %d, want %d; body=%s", profile.Code, http.StatusForbidden, profile.Body.String())
	}
	session, found, err := sessions.GetByAccessTokenHash(context.Background(), hashToken(token))
	if err != nil || !found || session.DeviceID != deviceID {
		t.Fatalf("profile mismatch changed session binding: session=%+v found=%t err=%v", session, found, err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if got := state.Devices[deviceID].Product; got != controlplane.ProductDouyinDesktop {
			t.Fatalf("profile mismatch changed device product = %q", got)
		}
		return nil
	}); err != nil {
		t.Fatalf("verify mismatched device: %v", err)
	}
}

func TestLogoutRevokesAccessAndRefreshTokens(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)

	logoutRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/logout", nil)
	logoutRequest.Header.Set("Authorization", "Bearer "+initial.AccessToken)
	logoutRecorder := httptest.NewRecorder()
	handler.ServeHTTP(logoutRecorder, logoutRequest)
	if logoutRecorder.Code != http.StatusOK {
		t.Fatalf("logout status = %d, want %d; body=%s", logoutRecorder.Code, http.StatusOK, logoutRecorder.Body.String())
	}

	accessRequest := httptest.NewRequest(http.MethodGet, "/api/v1/admin/users", nil)
	accessRequest.Header.Set("Authorization", "Bearer "+initial.AccessToken)
	accessRecorder := httptest.NewRecorder()
	handler.ServeHTTP(accessRecorder, accessRequest)
	if accessRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("logged-out access status = %d, want %d", accessRecorder.Code, http.StatusUnauthorized)
	}

	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(
		`{"refresh_token":"`+initial.RefreshToken+`"}`,
	))
	refreshRecorder := httptest.NewRecorder()
	handler.ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusUnauthorized {
		t.Fatalf("logged-out refresh status = %d, want %d", refreshRecorder.Code, http.StatusUnauthorized)
	}
}

func TestLogoutReportsPersistentRevocationFailure(t *testing.T) {
	sessionStore := newTestSessionStore()
	sessionStore.revokeErr = errors.New("database unavailable")
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStore("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	}, store.NewMemoryStore(time.Now), store.NewMemorySecretStore(), sessionStore)
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)

	logoutRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/logout", nil)
	logoutRequest.Header.Set("Authorization", "Bearer "+initial.AccessToken)
	logoutRecorder := httptest.NewRecorder()
	handler.ServeHTTP(logoutRecorder, logoutRequest)
	if logoutRecorder.Code != http.StatusServiceUnavailable {
		t.Fatalf("logout status = %d, want %d; body=%s", logoutRecorder.Code, http.StatusServiceUnavailable, logoutRecorder.Body.String())
	}

	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(
		`{"refresh_token":"`+initial.RefreshToken+`"}`,
	))
	refreshRecorder := httptest.NewRecorder()
	handler.ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusOK {
		t.Fatalf("refresh after failed logout status = %d, want %d", refreshRecorder.Code, http.StatusOK)
	}
}

func TestBindDeviceReportsPersistentUpdateFailure(t *testing.T) {
	sessionStore := newTestSessionStore()
	sessionStore.updateErr = errors.New("database unavailable")
	repository := store.NewMemoryStore(time.Now)
	auth := newAuthenticator(service.NewControlPlane(repository), AuthConfig{
		Username: "admin",
		Password: "correct-password",
	}, sessionStore)
	loginRecorder := httptest.NewRecorder()
	loginRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"correct-password","product":"autolive"}`,
	))
	loginHandler(auth).ServeHTTP(loginRecorder, loginRequest)
	if loginRecorder.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", loginRecorder.Code, http.StatusOK, loginRecorder.Body.String())
	}
	var payload loginResponse
	if err := json.Unmarshal(loginRecorder.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode login response: %v", err)
	}
	request := httptest.NewRequest(http.MethodPost, "/api/v1/client/activate", nil)
	request = request.WithContext(context.WithValue(request.Context(), sessionTokenContextKey, payload.Tokens.AccessToken))
	if err := auth.bindDevice(request, "dev_binding_failure"); err == nil {
		t.Fatal("bindDevice() unexpectedly succeeded when session update failed")
	}
}

func TestLogoutCannotRevokeAnotherSessionRefreshToken(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	first := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)
	second := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password","product":"autolive"}`)

	logoutRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/logout", strings.NewReader(
		`{"refresh_token":"`+second.RefreshToken+`"}`,
	))
	logoutRequest.Header.Set("Authorization", "Bearer "+first.AccessToken)
	logoutRecorder := httptest.NewRecorder()
	handler.ServeHTTP(logoutRecorder, logoutRequest)
	if logoutRecorder.Code != http.StatusOK {
		t.Fatalf("logout status = %d, want %d; body=%s", logoutRecorder.Code, http.StatusOK, logoutRecorder.Body.String())
	}

	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(
		`{"refresh_token":"`+second.RefreshToken+`"}`,
	))
	refreshRecorder := httptest.NewRecorder()
	handler.ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusOK {
		t.Fatalf("another session refresh status = %d, want %d; body=%s", refreshRecorder.Code, http.StatusOK, refreshRecorder.Body.String())
	}
}

func TestPersistentSessionSurvivesAuthenticatorRecreation(t *testing.T) {
	sessionStore := newTestSessionStore()
	repository := store.NewMemoryStore(time.Now)
	controlPlane := service.NewControlPlane(repository)
	config := AuthConfig{Username: "admin", Password: "correct-password"}
	first := newAuthenticator(controlPlane, config, sessionStore)

	loginRecorder := httptest.NewRecorder()
	loginRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"correct-password","product":"autolive"}`))
	loginHandler(first).ServeHTTP(loginRecorder, loginRequest)
	if loginRecorder.Code != http.StatusOK {
		t.Fatalf("persistent login status = %d; body=%s", loginRecorder.Code, loginRecorder.Body.String())
	}
	var loggedIn loginResponse
	if err := json.Unmarshal(loginRecorder.Body.Bytes(), &loggedIn); err != nil {
		t.Fatalf("decode persistent login: %v", err)
	}

	second := newAuthenticator(controlPlane, config, sessionStore)
	accessRequest := httptest.NewRequest(http.MethodGet, "/api/v1/admin/users", nil)
	accessRequest.Header.Set("Authorization", "Bearer "+loggedIn.Tokens.AccessToken)
	accessRecorder := httptest.NewRecorder()
	second.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		if actor.UserID != "usr_local_admin" || actor.Role != controlplane.RoleAdmin {
			t.Fatalf("restored actor = %+v", actor)
		}
		w.WriteHeader(http.StatusNoContent)
	}).ServeHTTP(accessRecorder, accessRequest)
	if accessRecorder.Code != http.StatusNoContent {
		t.Fatalf("restored access status = %d; body=%s", accessRecorder.Code, accessRecorder.Body.String())
	}

	refreshRecorder := httptest.NewRecorder()
	refreshRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/refresh", strings.NewReader(
		`{"refresh_token":"`+loggedIn.Tokens.RefreshToken+`"}`,
	))
	refreshHandler(second).ServeHTTP(refreshRecorder, refreshRequest)
	if refreshRecorder.Code != http.StatusOK {
		t.Fatalf("restored refresh status = %d; body=%s", refreshRecorder.Code, refreshRecorder.Body.String())
	}
}

type testSessionStore struct {
	mu        sync.Mutex
	byAccess  map[string]store.AuthSession
	byRefresh map[string]string
	revoked   map[string]bool
	revokeErr error
	updateErr error
}

type snapshotProductRepositoryStub struct {
	*store.MemoryStore
	membershipCalls int
}

func (s *snapshotProductRepositoryStub) ListProducts(context.Context) ([]controlplane.ProductSummary, error) {
	return nil, store.ErrNormalizedProductRepositoryRequired
}

func (s *snapshotProductRepositoryStub) GetProduct(context.Context, controlplane.ProductCode) (controlplane.ProductSummary, error) {
	return controlplane.ProductSummary{}, store.ErrNormalizedProductRepositoryRequired
}

func (s *snapshotProductRepositoryStub) GetUserProductMembership(context.Context, string, controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	s.membershipCalls++
	return controlplane.UserProductMembership{}, store.ErrNormalizedProductRepositoryRequired
}

func (s *snapshotProductRepositoryStub) EnsureUserProductMembership(context.Context, string, controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	return controlplane.UserProductMembership{}, store.ErrNormalizedProductRepositoryRequired
}

func (s *snapshotProductRepositoryStub) UsesNormalizedReadSource() bool { return false }

func TestSnapshotProductRepositoryIsNotInjectedIntoAuthenticator(t *testing.T) {
	repository := &snapshotProductRepositoryStub{MemoryStore: store.NewMemoryStore(time.Now)}
	handler := NewRouterWithRepositoryAndSecretStore("test", nil, AuthConfig{
		Username: "admin",
		Password: "password",
	}, repository, store.NewMemorySecretStore())
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(
		`{"username":"admin","password":"password","product":"autolive"}`,
	))
	handler.ServeHTTP(recorder, request)
	if recorder.Code != http.StatusOK {
		t.Fatalf("snapshot login status = %d, want %d; body=%s", recorder.Code, http.StatusOK, recorder.Body.String())
	}
	if repository.membershipCalls != 0 {
		t.Fatalf("snapshot membership calls = %d, want 0", repository.membershipCalls)
	}
}

func newTestSessionStore() *testSessionStore {
	return &testSessionStore{
		byAccess:  map[string]store.AuthSession{},
		byRefresh: map[string]string{},
		revoked:   map[string]bool{},
	}
}

func (s *testSessionStore) Create(_ context.Context, session store.AuthSession) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.byAccess[session.AccessTokenHash] = session
	s.byRefresh[session.RefreshTokenHash] = session.AccessTokenHash
	return nil
}

func (s *testSessionStore) GetByAccessTokenHash(_ context.Context, hash string) (store.AuthSession, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.byAccess[hash]
	if !ok || s.revoked[session.ID] {
		return store.AuthSession{}, false, nil
	}
	return session, true, nil
}

func (s *testSessionStore) GetByRefreshTokenHash(_ context.Context, hash string) (store.AuthSession, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	accessHash, ok := s.byRefresh[hash]
	if !ok {
		return store.AuthSession{}, false, nil
	}
	session, ok := s.byAccess[accessHash]
	if !ok || s.revoked[session.ID] {
		return store.AuthSession{}, false, nil
	}
	return session, true, nil
}

func (s *testSessionStore) Rotate(_ context.Context, refreshHash string, next store.AuthSession) (store.AuthSession, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	accessHash, ok := s.byRefresh[refreshHash]
	if !ok {
		return store.AuthSession{}, false, nil
	}
	old, ok := s.byAccess[accessHash]
	if !ok || s.revoked[old.ID] {
		return store.AuthSession{}, false, nil
	}
	s.revoked[old.ID] = true
	s.byAccess[next.AccessTokenHash] = next
	s.byRefresh[next.RefreshTokenHash] = next.AccessTokenHash
	return old, true, nil
}

func (s *testSessionStore) RevokeByAccessTokenHash(_ context.Context, hash string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.revokeErr != nil {
		return s.revokeErr
	}
	if session, ok := s.byAccess[hash]; ok {
		s.revoked[session.ID] = true
	}
	return nil
}

func (s *testSessionStore) RevokeByUserID(_ context.Context, userID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.revokeErr != nil {
		return s.revokeErr
	}
	for _, session := range s.byAccess {
		if session.UserID == userID {
			s.revoked[session.ID] = true
		}
	}
	return nil
}

func (s *testSessionStore) RevokeByDeviceID(_ context.Context, deviceID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.revokeErr != nil {
		return s.revokeErr
	}
	for _, session := range s.byAccess {
		if session.DeviceID == deviceID {
			s.revoked[session.ID] = true
		}
	}
	return nil
}

func (s *testSessionStore) RevokeByDeviceIDForProduct(_ context.Context, deviceID string, product controlplane.ProductCode) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.revokeErr != nil {
		return s.revokeErr
	}
	for _, session := range s.byAccess {
		if session.DeviceID == deviceID && session.Product == product {
			s.revoked[session.ID] = true
		}
	}
	return nil
}

func (s *testSessionStore) UpdateDeviceID(_ context.Context, hash, deviceID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.updateErr != nil {
		return s.updateErr
	}
	if session, ok := s.byAccess[hash]; ok {
		session.DeviceID = deviceID
		s.byAccess[hash] = session
	}
	return nil
}

func (s *testSessionStore) ClearDeviceID(_ context.Context, hash, deviceID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.updateErr != nil {
		return s.updateErr
	}
	if session, ok := s.byAccess[hash]; ok && session.DeviceID == deviceID {
		session.DeviceID = ""
		s.byAccess[hash] = session
		return nil
	}
	return errors.New("session device binding not found")
}

type testSessionTokens struct {
	AccessToken  string
	RefreshToken string
}

func loginTokensForTest(t *testing.T, handler http.Handler, body string) testSessionTokens {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(body))
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", rec.Code, http.StatusOK, rec.Body.String())
	}

	var payload loginResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode login response: %v", err)
	}
	return testSessionTokens{
		AccessToken:  payload.Tokens.AccessToken,
		RefreshToken: payload.Tokens.RefreshToken,
	}
}

func loginWithCredentialsForTest(t *testing.T, handler http.Handler, body string) string {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", rec.Code, http.StatusOK, rec.Body.String())
	}

	var payload loginResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode login response: %v", err)
	}
	return payload.Tokens.AccessToken
}
