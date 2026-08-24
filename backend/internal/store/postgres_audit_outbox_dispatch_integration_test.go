//go:build postgres_integration

package store

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresRepositoryDispatchAuditOutboxDeliversPendingRow(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	dispatchAt := time.Unix(0, 0).UTC()
	suffix := fmt.Sprintf("%d", now.UnixNano())
	outboxID := "audit_outbox_dispatch_integration_" + suffix
	auditID := "audit_dispatch_integration_" + suffix
	dedupeKey := "audit-dispatch-integration-" + suffix
	requestID := "audit-dispatch-request-" + suffix

	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if _, err := database.ExecContext(cleanupCtx, `DELETE FROM audit_logs WHERE id = $1`, auditID); err != nil {
			t.Errorf("cleanup audit log: %v", err)
		}
		if _, err := database.ExecContext(cleanupCtx, `DELETE FROM audit_outbox WHERE id = $1`, outboxID); err != nil {
			t.Errorf("cleanup audit outbox: %v", err)
		}
	})

	payload, err := json.Marshal(controlplane.AuditLogInput{
		Product:    controlplane.ProductAutoLive,
		Action:     "integration.audit_outbox.dispatch",
		TargetType: "integration",
		TargetID:   outboxID,
		RequestID:  requestID,
		Outcome:    "success",
		StatusCode: 200,
	})
	if err != nil {
		t.Fatalf("marshal audit outbox payload: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO audit_outbox (
			id, dedupe_key, payload, product, status, attempts,
			next_attempt_at, created_at, updated_at
		) VALUES ($1, $2, $3::jsonb, $4, 'pending', 0, $5, $6, $6)
	`, outboxID, dedupeKey, string(payload), controlplane.ProductAutoLive, dispatchAt, now); err != nil {
		t.Fatalf("seed pending audit outbox row: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error: %v", err)
	}
	delivered, err := repository.DispatchAuditOutbox(context.Background(), 1)
	if err != nil {
		t.Fatalf("DispatchAuditOutbox() error: %v", err)
	}
	if delivered != 1 {
		t.Fatalf("DispatchAuditOutbox() delivered = %d, want 1", delivered)
	}

	var status string
	if err := database.QueryRowContext(ctx, `SELECT status FROM audit_outbox WHERE id = $1`, outboxID).Scan(&status); err != nil {
		t.Fatalf("read dispatched audit outbox status: %v", err)
	}
	if status != "sent" {
		t.Fatalf("audit outbox status = %q, want sent", status)
	}

	var auditCount int
	if err := database.QueryRowContext(ctx, `
		SELECT COUNT(*)
		FROM audit_logs
		WHERE id = $1
		  AND product = $2
		  AND action = $3
		  AND request_id = $4
	`, auditID, controlplane.ProductAutoLive, "integration.audit_outbox.dispatch", requestID).Scan(&auditCount); err != nil {
		t.Fatalf("count delivered audit log: %v", err)
	}
	if auditCount != 1 {
		t.Fatalf("delivered audit log count = %d, want 1", auditCount)
	}
}
