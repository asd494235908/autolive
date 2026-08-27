package httpapi

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

func TestLoginEntrypointsAndProtectedRoutesEnforceSessionAudience(t *testing.T) {
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin",
		Password: "secure-admin-password",
	}, store.NewMemoryStore(time.Now), store.NewMemorySecretStore(), nil, true)

	admin := loginTokensAtPathForTest(t, handler, "/api/v1/auth/login", `{"username":"admin","password":"secure-admin-password","product":"autolive"}`)
	if admin.Audience != string(store.SessionAudienceAdmin) {
		t.Fatalf("admin audience = %q, want %q", admin.Audience, store.SessionAudienceAdmin)
	}
	created := doJSON(t, handler, http.MethodPost, "/api/v1/admin/users", map[string]any{
		"username": "desktop_user",
		"password": "secure-desktop-password",
		"role":     "user",
	}, admin.AccessToken, "create-desktop-user")
	if created.Code != http.StatusCreated {
		t.Fatalf("create user status = %d; body=%s", created.Code, created.Body.String())
	}

	adminAtDesktop := doRawAuthRequest(handler, "/api/v1/client/auth/login", `{"username":"admin","password":"secure-admin-password","product":"autolive"}`)
	if adminAtDesktop.Code != http.StatusUnauthorized {
		t.Fatalf("admin desktop login status = %d, want %d; body=%s", adminAtDesktop.Code, http.StatusUnauthorized, adminAtDesktop.Body.String())
	}
	userAtAdmin := doRawAuthRequest(handler, "/api/v1/auth/login", `{"username":"desktop_user","password":"secure-desktop-password","product":"autolive"}`)
	if userAtAdmin.Code != http.StatusUnauthorized {
		t.Fatalf("user admin login status = %d, want %d; body=%s", userAtAdmin.Code, http.StatusUnauthorized, userAtAdmin.Body.String())
	}

	desktop := loginTokensAtPathForTest(t, handler, "/api/v1/client/auth/login", `{"username":"desktop_user","password":"secure-desktop-password","product":"autolive"}`)
	if desktop.Audience != string(store.SessionAudienceDesktop) {
		t.Fatalf("desktop audience = %q, want %q", desktop.Audience, store.SessionAudienceDesktop)
	}

	adminToClient := doJSON(t, handler, http.MethodGet, "/api/v1/client/profile", nil, admin.AccessToken, "")
	assertAuthSessionErrorCode(t, adminToClient, http.StatusForbidden, "AUTH_SESSION_AUDIENCE_MISMATCH")
	desktopToAdmin := doJSON(t, handler, http.MethodGet, "/api/v1/admin/users?page_size=1", nil, desktop.AccessToken, "")
	assertAuthSessionErrorCode(t, desktopToAdmin, http.StatusForbidden, "AUTH_SESSION_AUDIENCE_MISMATCH")
}

func TestAccessTokenExpiresInFifteenMinutes(t *testing.T) {
	handler := NewRouterWithAuth("test", nil, AuthConfig{Username: "admin", Password: "correct-password"})
	before := time.Now().UTC()
	tokens := loginTokensAtPathForTest(t, handler, "/api/v1/auth/login", `{"username":"admin","password":"correct-password"}`)
	expiresAt, err := time.Parse(time.RFC3339, tokens.ExpiresAt)
	if err != nil {
		t.Fatalf("parse expires_at: %v", err)
	}
	if expiresAt.Before(before.Add(14*time.Minute+59*time.Second)) || expiresAt.After(before.Add(15*time.Minute+2*time.Second)) {
		t.Fatalf("access expiry = %s, want about 15 minutes after %s", expiresAt, before)
	}
}

func TestLogoutUsesRefreshTokenWithoutBearerAndIsIdempotent(t *testing.T) {
	handler := NewRouterWithAuth("test", nil, AuthConfig{Username: "admin", Password: "correct-password"})
	tokens := loginTokensAtPathForTest(t, handler, "/api/v1/auth/login", `{"username":"admin","password":"correct-password"}`)
	for attempt := 0; attempt < 2; attempt++ {
		logout := doRawAuthRequest(handler, "/api/v1/auth/logout", `{"refresh_token":"`+tokens.RefreshToken+`"}`)
		if logout.Code != http.StatusOK {
			t.Fatalf("logout attempt %d status = %d; body=%s", attempt+1, logout.Code, logout.Body.String())
		}
	}
	refresh := doRawAuthRequest(handler, "/api/v1/auth/refresh", `{"refresh_token":"`+tokens.RefreshToken+`"}`)
	if refresh.Code != http.StatusUnauthorized {
		t.Fatalf("refresh after logout status = %d, want %d", refresh.Code, http.StatusUnauthorized)
	}
}

func TestRefreshReplayRevokesWholeTokenFamily(t *testing.T) {
	handler := NewRouterWithAuth("test", nil, AuthConfig{Username: "admin", Password: "correct-password"})
	initial := loginTokensAtPathForTest(t, handler, "/api/v1/auth/login", `{"username":"admin","password":"correct-password"}`)
	rotatedRecorder := doRawAuthRequest(handler, "/api/v1/auth/refresh", `{"refresh_token":"`+initial.RefreshToken+`"}`)
	if rotatedRecorder.Code != http.StatusOK {
		t.Fatalf("first refresh status = %d; body=%s", rotatedRecorder.Code, rotatedRecorder.Body.String())
	}
	var rotated refreshTokenResponse
	decodeJSON(t, rotatedRecorder.Body.Bytes(), &rotated)
	if rotated.Tokens.Audience != initial.Audience {
		t.Fatalf("rotated audience = %q, want %q", rotated.Tokens.Audience, initial.Audience)
	}

	replay := doRawAuthRequest(handler, "/api/v1/auth/refresh", `{"refresh_token":"`+initial.RefreshToken+`"}`)
	if replay.Code != http.StatusUnauthorized {
		t.Fatalf("replayed refresh status = %d, want %d", replay.Code, http.StatusUnauthorized)
	}
	current := doRawAuthRequest(handler, "/api/v1/auth/refresh", `{"refresh_token":"`+rotated.Tokens.RefreshToken+`"}`)
	if current.Code != http.StatusUnauthorized {
		t.Fatalf("current family refresh status = %d, want %d after replay", current.Code, http.StatusUnauthorized)
	}
}

type authTestTokens struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresAt    string `json:"expires_at"`
	Audience     string `json:"audience"`
}

func loginTokensAtPathForTest(t *testing.T, handler http.Handler, path, body string) authTestTokens {
	t.Helper()
	recorder := doRawAuthRequest(handler, path, body)
	if recorder.Code != http.StatusOK {
		t.Fatalf("login %s status = %d; body=%s", path, recorder.Code, recorder.Body.String())
	}
	var payload struct {
		Tokens authTestTokens `json:"tokens"`
	}
	if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode login response: %v", err)
	}
	return payload.Tokens
}

func doRawAuthRequest(handler http.Handler, path, body string) *httptest.ResponseRecorder {
	request := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	handler.ServeHTTP(recorder, request)
	return recorder
}

func assertAuthSessionErrorCode(t *testing.T, recorder *httptest.ResponseRecorder, status int, code string) {
	t.Helper()
	if recorder.Code != status {
		t.Fatalf("status = %d, want %d; body=%s", recorder.Code, status, recorder.Body.String())
	}
	var payload struct {
		Code string `json:"code"`
	}
	decodeJSON(t, recorder.Body.Bytes(), &payload)
	if payload.Code != code {
		t.Fatalf("error code = %q, want %q", payload.Code, code)
	}
}
