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
