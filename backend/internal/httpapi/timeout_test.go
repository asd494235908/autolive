package httpapi

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestWithRequestTimeoutPropagatesDeadlineAndKeepsJSONErrorContract(t *testing.T) {
	handler := WithRequestTimeout(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
		writeAppError(w, r, r.Context().Err())
	}), 5*time.Millisecond)
	req := httptest.NewRequest(http.MethodGet, "/slow", nil)
	req = req.WithContext(context.Background())
	rec := httptest.NewRecorder()

	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusGatewayTimeout {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusGatewayTimeout)
	}
	var payload ErrorResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("json.Unmarshal() error = %v", err)
	}
	if payload.Code != "REQUEST_TIMEOUT" {
		t.Fatalf("code = %q, want REQUEST_TIMEOUT", payload.Code)
	}
}

func TestWithRequestTimeoutLeavesDisabledBudgetUntouched(t *testing.T) {
	called := false
	handler := WithRequestTimeout(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
		if _, ok := r.Context().Deadline(); ok {
			t.Fatal("unexpected deadline for disabled timeout")
		}
		w.WriteHeader(http.StatusNoContent)
	}), 0)
	handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest(http.MethodGet, "/", nil))
	if !called {
		t.Fatal("handler was not called")
	}
}
