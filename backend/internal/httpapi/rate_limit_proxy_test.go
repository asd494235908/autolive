package httpapi

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

func TestClientAddressResolverOnlyTrustsForwardedForFromConfiguredProxy(t *testing.T) {
	resolver, err := newClientAddressResolver([]string{"10.0.0.0/8", "2001:db8::/32"})
	if err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name       string
		remoteAddr string
		forwarded  string
		want       string
	}{
		{name: "untrusted ignores header", remoteAddr: "198.51.100.10:1234", forwarded: "203.0.113.5", want: "198.51.100.10"},
		{name: "trusted selects first untrusted hop", remoteAddr: "10.0.0.2:443", forwarded: "203.0.113.5, 10.0.0.1", want: "203.0.113.5"},
		{name: "trusted rejects malformed chain", remoteAddr: "10.0.0.2:443", forwarded: "bad-value, 10.0.0.1", want: "10.0.0.2"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequest("POST", "/api/v1/auth/login", nil)
			req.RemoteAddr = tt.remoteAddr
			req.Header.Set("X-Forwarded-For", tt.forwarded)
			if got := resolver.clientAddress(req); got != tt.want {
				t.Fatalf("clientAddress() = %q, want %q", got, tt.want)
			}
		})
	}
}

type loginThrottleStoreSpy struct {
	recorded []store.LoginThrottleBucket
	reset    []store.LoginThrottleBucket
}

type loginThrottleRepositorySpy struct {
	*store.MemoryStore
	*loginThrottleStoreSpy
}

func (s *loginThrottleStoreSpy) CheckLoginThrottle(context.Context, []store.LoginThrottleBucket, time.Time) (time.Duration, error) {
	return 0, nil
}
func (s *loginThrottleStoreSpy) RecordLoginFailure(_ context.Context, buckets []store.LoginThrottleBucket, _ time.Time) error {
	s.recorded = append([]store.LoginThrottleBucket(nil), buckets...)
	return nil
}
func (s *loginThrottleStoreSpy) ResetLoginFailures(_ context.Context, buckets []store.LoginThrottleBucket) error {
	s.reset = append([]store.LoginThrottleBucket(nil), buckets...)
	return nil
}

func TestLoginThrottleSuccessOnlyClearsAccountBucket(t *testing.T) {
	spy := &loginThrottleStoreSpy{}
	handler := loginThrottleMiddleware(loginThrottleOptions{
		store:   spy,
		hmacKey: []byte("0123456789abcdef0123456789abcdef"),
	}, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) }))
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", io.NopCloser(strings.NewReader(`{"username":"alice","password":"secret"}`)))
	req.RemoteAddr = "203.0.113.9:1234"
	handler.ServeHTTP(httptest.NewRecorder(), req)
	if len(spy.reset) != 1 || spy.reset[0].Type != store.LoginThrottleBucketAccount {
		t.Fatalf("reset buckets = %+v, want account only", spy.reset)
	}
	if len(spy.recorded) != 0 {
		t.Fatalf("recorded buckets = %+v, want none", spy.recorded)
	}
}

func TestLoginThrottleMalformedLoginStillRecordsAddressBucket(t *testing.T) {
	spy := &loginThrottleStoreSpy{}
	handler := loginThrottleMiddleware(loginThrottleOptions{
		store:   spy,
		hmacKey: []byte("0123456789abcdef0123456789abcdef"),
	}, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusUnauthorized) }))
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"password":"secret"}`))
	req.RemoteAddr = "203.0.113.9:1234"
	handler.ServeHTTP(httptest.NewRecorder(), req)
	if len(spy.recorded) != 1 || spy.recorded[0].Type != store.LoginThrottleBucketAddress {
		t.Fatalf("recorded buckets = %+v, want address only", spy.recorded)
	}
}

func TestLoginThrottleDigestDoesNotExposeUsernameOrAddress(t *testing.T) {
	digest, err := loginThrottleDigest([]byte("0123456789abcdef0123456789abcdef"), "Alice", "203.0.113.7")
	if err != nil {
		t.Fatal(err)
	}
	if len(digest.Username) != 64 || len(digest.Address) != 64 || digest.Username == "alice" || digest.Address == "203.0.113.7" {
		t.Fatalf("loginThrottleDigest() returned unsafe digest: %+v", digest)
	}
}

func TestRouterConnectsPersistentLoginThrottle(t *testing.T) {
	throttle := &loginThrottleStoreSpy{}
	repository := &loginThrottleRepositorySpy{
		MemoryStore:           store.NewMemoryStore(time.Now),
		loginThrottleStoreSpy: throttle,
	}
	handler := NewRouterWithRepository("test", nil, AuthConfig{
		Username:            "admin",
		Password:            "correct-password",
		AuthThrottleHMACKey: []byte("0123456789abcdef0123456789abcdef"),
		TrustedProxyCIDRs:   []string{"10.0.0.0/8"},
	}, repository)
	req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"wrong-password","product":"autolive"}`))
	req.RemoteAddr = "10.0.0.2:443"
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Forwarded-For", "203.0.113.7")
	recorder := httptest.NewRecorder()

	handler.ServeHTTP(recorder, req)

	if recorder.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", recorder.Code, http.StatusUnauthorized)
	}
	if len(throttle.recorded) != 2 {
		t.Fatalf("recorded buckets = %+v, want account and address", throttle.recorded)
	}
	wantAddress := hmacDigest([]byte("0123456789abcdef0123456789abcdef"), "address\x00203.0.113.7")
	var address store.LoginThrottleBucket
	for _, bucket := range throttle.recorded {
		if bucket.Type == store.LoginThrottleBucketAddress {
			address = bucket
		}
	}
	if address.Hash != wantAddress {
		t.Fatalf("address bucket = %+v, want trusted forwarded client digest", address)
	}
}
