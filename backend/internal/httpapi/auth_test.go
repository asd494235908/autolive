package httpapi

import (
	"context"
	"encoding/json"
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

	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"correct-password"}`))
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
		`{"username":"admin","password":"correct-password"}`,
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

	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"wrong-password"}`))
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestLoginReportsMissingDevelopmentConfiguration(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"password"}`))
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

	adminToken := loginWithCredentialsForTest(t, handler, `{"username":"admin","password":"correct-password"}`)
	createRequest := httptest.NewRequest(http.MethodPost, "/api/v1/admin/users", strings.NewReader(`{"username":"client-a","password":"client-password","role":"user"}`))
	createRequest.Header.Set("Authorization", "Bearer "+adminToken)
	createRequest.Header.Set("Idempotency-Key", "create-client-a")
	createRecorder := httptest.NewRecorder()
	handler.ServeHTTP(createRecorder, createRequest)
	if createRecorder.Code != http.StatusCreated {
		t.Fatalf("create user status = %d, want %d; body=%s", createRecorder.Code, http.StatusCreated, createRecorder.Body.String())
	}

	clientToken := loginWithCredentialsForTest(t, handler, `{"username":"client-a","password":"client-password"}`)
	if clientToken == "" || clientToken == adminToken {
		t.Fatalf("expected a distinct client access token")
	}
}

func TestRefreshRotatesTokensAndRevokesPreviousAccessToken(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password"}`)

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
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password"}`)
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

func TestLogoutRevokesAccessAndRefreshTokens(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	initial := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password"}`)

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

func TestLogoutCannotRevokeAnotherSessionRefreshToken(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{
		Username: "admin",
		Password: "correct-password",
	})
	first := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password"}`)
	second := loginTokensForTest(t, handler, `{"username":"admin","password":"correct-password"}`)

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
	loginRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"correct-password"}`))
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
	if session, ok := s.byAccess[hash]; ok {
		s.revoked[session.ID] = true
	}
	return nil
}

func (s *testSessionStore) UpdateDeviceID(_ context.Context, hash, deviceID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if session, ok := s.byAccess[hash]; ok {
		session.DeviceID = deviceID
		s.byAccess[hash] = session
	}
	return nil
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
