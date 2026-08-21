package service_test

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

func TestAuditLogRecordsActorTargetAndRequestID(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()

	if err := svc.RecordAudit(ctx, controlplane.AuditLogInput{
		ActorUserID: "usr_admin",
		DeviceID:    "dev_12345678",
		Action:      "POST /api/v1/admin/model-pool",
		TargetType:  "http_endpoint",
		TargetID:    "mpa_00000001",
		Outcome:     "success",
		StatusCode:  201,
		RequestID:   "req_12345678",
	}); err != nil {
		t.Fatalf("RecordAudit() error = %v", err)
	}
	items, err := svc.ListAuditLogs(ctx)
	if err != nil {
		t.Fatalf("ListAuditLogs() error = %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("len(items) = %d, want 1", len(items))
	}
	if items[0].ActorUserID != "usr_admin" || items[0].DeviceID != "dev_12345678" || items[0].RequestID != "req_12345678" || items[0].Outcome != "success" || items[0].StatusCode != 201 {
		t.Fatalf("audit item = %+v", items[0])
	}
}

func TestAuditLogPageWithOptionsFiltersTimeAndResult(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.AuditLogs["audit-old"] = controlplane.AuditLog{
			ID: "audit-old", ActorUserID: "usr-a", Outcome: "failure", CreatedAt: "2026-08-20T00:00:00Z",
		}
		state.AuditLogs["audit-new"] = controlplane.AuditLog{
			ID: "audit-new", ActorUserID: "usr-a", Outcome: "failure", CreatedAt: "2026-08-21T00:00:00Z",
		}
		state.AuditLogs["audit-success"] = controlplane.AuditLog{
			ID: "audit-success", ActorUserID: "usr-a", Outcome: "success", CreatedAt: "2026-08-22T00:00:00Z",
		}
		return nil
	}); err != nil {
		t.Fatalf("seed audit logs error = %v", err)
	}
	svc := service.NewControlPlane(repository)
	items, total, err := svc.ListAuditLogsPageWithOptions(context.Background(), 1, 20, service.AuditLogListOptions{
		ActorUserID: "usr-a", Outcome: "failure", CreatedAfter: "2026-08-20T12:00:00Z", Sort: store.AuditLogSortCreatedAsc,
	})
	if err != nil {
		t.Fatalf("ListAuditLogsPageWithOptions() error = %v", err)
	}
	if total != 1 || len(items) != 1 || items[0].ID != "audit-new" {
		t.Fatalf("filtered audit page = total %d items %+v", total, items)
	}
	if _, _, err := svc.ListAuditLogsPageWithOptions(context.Background(), 1, 20, service.AuditLogListOptions{CreatedAfter: "2026-08-22T00:00:00Z", CreatedBefore: "2026-08-21T00:00:00Z"}); !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("invalid audit time range error = %v, want INVALID_ARGUMENT", err)
	}
}
