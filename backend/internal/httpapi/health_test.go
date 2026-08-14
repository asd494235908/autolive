package httpapi

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestHealthHandler(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)

	req := httptest.NewRequest(http.MethodGet, "/api/v1/health", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}

	if got := rec.Header().Get("X-Request-Id"); got == "" {
		t.Fatal("X-Request-Id is empty")
	}

	var payload HealthResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}

	if payload.Status != "ok" {
		t.Fatalf("Status = %q", payload.Status)
	}

	if payload.Version != "v1.0.0" {
		t.Fatalf("Version = %q", payload.Version)
	}

	if payload.Service != "autolive-control-plane" {
		t.Fatalf("Service = %q", payload.Service)
	}

	if payload.Now == "" {
		t.Fatal("Now is empty")
	}

	if payload.RequestID == "" {
		t.Fatal("RequestID is empty")
	}
}

func TestHealthHandlerRejectsWrongMethod(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)

	req := httptest.NewRequest(http.MethodPost, "/api/v1/health", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusMethodNotAllowed)
	}
}
