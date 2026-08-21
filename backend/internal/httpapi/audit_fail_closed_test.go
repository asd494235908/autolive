package httpapi

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

type auditFailingRepository struct {
	*store.MemoryStore
	failAudit bool
}

func (r *auditFailingRepository) Run(ctx context.Context, fn store.StateOperation) error {
	return r.MemoryStore.Run(ctx, func(state *store.State) error {
		before := len(state.AuditLogs)
		if err := fn(state); err != nil {
			return err
		}
		if r.failAudit && len(state.AuditLogs) > before {
			return errors.New("audit store unavailable")
		}
		return nil
	})
}

func TestAuditFailureFailsClosedWithoutLeakingBusinessResponse(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &auditFailingRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin", Password: "password",
	}, repository, store.NewMemorySecretStore(), nil, true)
	token := loginForTest(t, handler)
	repository.failAudit = true

	failed := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  now.Add(time.Hour).Format(time.RFC3339),
		"max_devices": 1,
	}, token, "audit-fail-closed")
	if failed.Code != http.StatusServiceUnavailable {
		t.Fatalf("audit failure status = %d, want %d; body=%s", failed.Code, http.StatusServiceUnavailable, failed.Body.String())
	}
	var errorPayload ErrorResponse
	decodeJSON(t, failed.Body.Bytes(), &errorPayload)
	if errorPayload.Code != "AUDIT_UNAVAILABLE" || errorPayload.RequestID == "" {
		t.Fatalf("audit failure payload = %+v", errorPayload)
	}
	if strings.Contains(failed.Body.String(), "activation_code") {
		t.Fatalf("business success payload leaked through audit failure: %s", failed.Body.String())
	}

	repository.failAudit = false
	retry := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"expires_at":  now.Add(time.Hour).Format(time.RFC3339),
		"max_devices": 1,
	}, token, "audit-fail-closed")
	if retry.Code != http.StatusCreated {
		t.Fatalf("same idempotency retry status = %d, want %d; body=%s", retry.Code, http.StatusCreated, retry.Body.String())
	}

}

var _ store.Repository = (*auditFailingRepository)(nil)
