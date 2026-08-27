package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"autoLive/backend/internal/store"
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

func TestReadinessHandlerReturnsOKWhenDependenciesAreReady(t *testing.T) {
	handler := NewRouterWithAuth("v1.0.0", nil, AuthConfig{Username: "admin", Password: testAdminPassword})
	req := httptest.NewRequest(http.MethodGet, "/api/v1/readyz", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d; body=%s", rec.Code, http.StatusOK, rec.Body.String())
	}
	var payload HealthResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}
	if payload.Status != "ok" {
		t.Fatalf("Status = %q, want ok", payload.Status)
	}
}

func TestReadinessHandlerReportsRepositoryFailure(t *testing.T) {
	repository := failingHealthRepository{err: errors.New("database unavailable")}
	handler := NewRouterWithRepositoryAndSecretStore("v1.0.0", nil, AuthConfig{Username: "admin", Password: testAdminPassword}, repository, store.NewMemorySecretStore())
	req := httptest.NewRequest(http.MethodGet, "/api/v1/readyz", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want %d; body=%s", rec.Code, http.StatusServiceUnavailable, rec.Body.String())
	}
	var payload HealthResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}
	if payload.Status != "degraded" {
		t.Fatalf("Status = %q, want degraded", payload.Status)
	}
}

func TestReadinessHandlerReportsSecretStoreFailure(t *testing.T) {
	secretStore := failingHealthSecretStore{err: errors.New("secret store unavailable")}
	handler := NewRouterWithRepositoryAndSecretStore("v1.0.0", nil, AuthConfig{Username: "admin", Password: testAdminPassword}, store.NewMemoryStore(time.Now), secretStore)
	req := httptest.NewRequest(http.MethodGet, "/api/v1/readyz", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want %d; body=%s", rec.Code, http.StatusServiceUnavailable, rec.Body.String())
	}
	var payload HealthResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}
	if payload.Status != "degraded" {
		t.Fatalf("Status = %q, want degraded", payload.Status)
	}
}

func TestReadinessHandlerRejectsMissingAdminCredentials(t *testing.T) {
	handler := NewRouter("v1.0.0", nil)
	req := httptest.NewRequest(http.MethodGet, "/api/v1/readyz", nil)
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("status = %d, want %d; body=%s", rec.Code, http.StatusServiceUnavailable, rec.Body.String())
	}
}

type failingHealthRepository struct {
	err error
}

func (r failingHealthRepository) Now() time.Time {
	return time.Now().UTC()
}

func (r failingHealthRepository) Run(context.Context, store.StateOperation) error {
	return r.err
}

type failingHealthSecretStore struct {
	err error
}

func (s failingHealthSecretStore) Ping(context.Context) error {
	return s.err
}

func (s failingHealthSecretStore) Put(context.Context, string, string) error {
	return s.err
}

func (s failingHealthSecretStore) Get(context.Context, string) (string, error) {
	return "", s.err
}

func (s failingHealthSecretStore) Delete(context.Context, string) error {
	return s.err
}
