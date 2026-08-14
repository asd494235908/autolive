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
	if items[0].ActorUserID != "usr_admin" || items[0].DeviceID != "dev_12345678" || items[0].RequestID != "req_12345678" {
		t.Fatalf("audit item = %+v", items[0])
	}
}
