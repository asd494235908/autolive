package httpapi

import (
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestRateLimitMiddlewareRejectsBurstAndRecoversAfterRefill(t *testing.T) {
	now := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	limiter := newRequestRateLimiter(func() time.Time { return now })
	next := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	})
	handler := rateLimitMiddleware(limiter, next)

	for index := 0; index < protectedRateLimitPolicies[0].burst; index++ {
		recorder := httptest.NewRecorder()
		request := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", nil)
		request.RemoteAddr = "192.0.2.10:1234"
		handler.ServeHTTP(recorder, request)
		if recorder.Code != http.StatusNoContent {
			t.Fatalf("request %d status = %d, want 204", index+1, recorder.Code)
		}
	}

	rejected := httptest.NewRecorder()
	rejectedRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", nil)
	rejectedRequest.RemoteAddr = "192.0.2.10:1234"
	handler.ServeHTTP(rejected, rejectedRequest)
	if rejected.Code != http.StatusTooManyRequests {
		t.Fatalf("burst status = %d, want 429", rejected.Code)
	}
	wantRetryAfter := formatRetryAfter(protectedRateLimitPolicies[0].retry)
	if rejected.Header().Get("Retry-After") != wantRetryAfter {
		t.Fatalf("Retry-After = %q, want %s", rejected.Header().Get("Retry-After"), wantRetryAfter)
	}

	now = now.Add(2 * time.Second)
	recovered := httptest.NewRecorder()
	recoveredRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", nil)
	recoveredRequest.RemoteAddr = "192.0.2.10:1234"
	handler.ServeHTTP(recovered, recoveredRequest)
	if recovered.Code != http.StatusNoContent {
		t.Fatalf("recovered status = %d, want 204", recovered.Code)
	}
}

func TestLoginRateLimitSeparatesAccountAndAddressBursts(t *testing.T) {
	now := time.Date(2026, 8, 28, 12, 0, 0, 0, time.UTC)
	limiter := newRequestRateLimiter(func() time.Time { return now })
	handler := rateLimitMiddleware(limiter, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))

	login := func(username string) int {
		recorder := httptest.NewRecorder()
		request := httptest.NewRequest(http.MethodPost, "/api/v1/client/auth/login", strings.NewReader(`{"username":"`+username+`","password":"password","product":"autolive"}`))
		request.RemoteAddr = "192.0.2.10:1234"
		handler.ServeHTTP(recorder, request)
		return recorder.Code
	}
	for attempt := 0; attempt < authLoginAccountRateLimitPolicy.burst; attempt++ {
		if status := login("alice"); status != http.StatusNoContent {
			t.Fatalf("alice request %d status = %d, want 204", attempt+1, status)
		}
	}
	if status := login("alice"); status != http.StatusTooManyRequests {
		t.Fatalf("alice account burst status = %d, want 429", status)
	}
	if status := login("bob"); status != http.StatusNoContent {
		t.Fatalf("bob behind shared address status = %d, want 204", status)
	}
}

func TestLoginRateLimitStillCapsAggregateAddressBurst(t *testing.T) {
	now := time.Date(2026, 8, 28, 12, 0, 0, 0, time.UTC)
	limiter := newRequestRateLimiter(func() time.Time { return now })
	handler := rateLimitMiddleware(limiter, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))

	for attempt := 0; attempt < protectedRateLimitPolicies[0].burst; attempt++ {
		recorder := httptest.NewRecorder()
		request := httptest.NewRequest(http.MethodPost, "/api/v1/client/auth/login", strings.NewReader(`{"username":"user-`+strconv.Itoa(attempt)+`","password":"password","product":"autolive"}`))
		request.RemoteAddr = "192.0.2.10:1234"
		handler.ServeHTTP(recorder, request)
		if recorder.Code != http.StatusNoContent {
			t.Fatalf("address request %d status = %d, want 204", attempt+1, recorder.Code)
		}
	}
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/api/v1/client/auth/login", strings.NewReader(`{"username":"overflow","password":"password","product":"autolive"}`))
	request.RemoteAddr = "192.0.2.10:1234"
	handler.ServeHTTP(recorder, request)
	if recorder.Code != http.StatusTooManyRequests {
		t.Fatalf("aggregate address burst status = %d, want 429", recorder.Code)
	}
}

func TestRateLimiterIsBoundedAndEvictsIdleEntries(t *testing.T) {
	now := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	limiter := newRequestRateLimiter(func() time.Time { return now })
	limiter.maxKeys = 2
	policy := protectedRateLimitPolicies[0]
	if !limiter.allow("auth-login:one", policy) || !limiter.allow("auth-login:two", policy) {
		t.Fatal("initial limiter entries were unexpectedly rejected")
	}
	now = now.Add(11 * time.Minute)
	if !limiter.allow("auth-login:three", policy) {
		t.Fatal("new entry was rejected after idle cleanup")
	}
	limiter.mu.Lock()
	defer limiter.mu.Unlock()
	if len(limiter.entries) != 1 {
		t.Fatalf("entries = %d, want one active entry after idle cleanup", len(limiter.entries))
	}
}

func TestRateLimitPolicyTargetsOnlyHighRiskEndpoints(t *testing.T) {
	tests := []struct {
		method string
		path   string
		want   bool
	}{
		{method: http.MethodPost, path: "/api/v1/auth/login", want: true},
		{method: http.MethodPost, path: "/api/v1/auth/refresh", want: true},
		{method: http.MethodPost, path: "/api/v1/admin/model-pool/mpa_00000001/test", want: true},
		{method: http.MethodPost, path: "/api/v1/admin/model-pool/mpa_00000001/rotate-secret", want: true},
		{method: http.MethodPost, path: "/api/v1/admin/auth/change-password", want: true},
		{method: http.MethodGet, path: "/api/v1/auth/login", want: false},
		{method: http.MethodPost, path: "/api/v1/health", want: false},
	}
	for _, test := range tests {
		request := httptest.NewRequest(test.method, test.path, nil)
		_, ok := rateLimitPolicyForRequest(request)
		if ok != test.want {
			t.Errorf("policy for %s %s = %v, want %v", test.method, test.path, ok, test.want)
		}
	}
}
