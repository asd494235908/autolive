package httpapi

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestRouterNotFoundReturnsStructuredJSON(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)

	req := httptest.NewRequest(http.MethodGet, "/missing", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusNotFound)
	}

	if got := rec.Header().Get("Content-Type"); !strings.Contains(got, "application/json") {
		t.Fatalf("Content-Type = %q", got)
	}

	if body := rec.Body.String(); !strings.Contains(body, "\"code\":\"NOT_FOUND\"") {
		t.Fatalf("body = %s", body)
	}
}

func TestRouterPreservesValidRequestIDAndRegeneratesInvalidID(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)

	validRequest := httptest.NewRequest(http.MethodGet, "/missing", nil)
	validRequest.Header.Set("X-Request-Id", "client-request-123")
	validRecorder := httptest.NewRecorder()
	handler.ServeHTTP(validRecorder, validRequest)
	if got := validRecorder.Header().Get("X-Request-Id"); got != "client-request-123" {
		t.Fatalf("valid request id = %q, want client-request-123", got)
	}

	invalidRequest := httptest.NewRequest(http.MethodGet, "/missing", nil)
	invalidRequest.Header.Set("X-Request-Id", "bad value")
	invalidRecorder := httptest.NewRecorder()
	handler.ServeHTTP(invalidRecorder, invalidRequest)
	if got := invalidRecorder.Header().Get("X-Request-Id"); got == "bad value" || got == "" {
		t.Fatalf("invalid request id was not regenerated: %q", got)
	}
}
