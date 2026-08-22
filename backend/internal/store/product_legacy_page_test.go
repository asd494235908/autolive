package store

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestMemoryProductPagesNormalizeLegacyZeroProduct(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *State) error {
		state.ModelLeases["legacy-lease"] = controlplane.ModelLease{ID: "legacy-lease", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelLeases["douyin-lease"] = controlplane.ModelLease{ID: "douyin-lease", Product: controlplane.ProductDouyinDesktop, Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["legacy-usage"] = controlplane.ModelUsageRecord{ID: "legacy-usage", LeaseID: "legacy-lease", CreatedAt: now.Format(time.RFC3339)}
		state.ModelUsageRecords["douyin-usage"] = controlplane.ModelUsageRecord{ID: "douyin-usage", Product: controlplane.ProductDouyinDesktop, LeaseID: "douyin-lease", CreatedAt: now.Add(time.Minute).Format(time.RFC3339)}
		state.AuditLogs["legacy-audit"] = controlplane.AuditLog{ID: "legacy-audit", Action: "legacy", Outcome: "success", CreatedAt: now.Format(time.RFC3339)}
		state.AuditLogs["douyin-audit"] = controlplane.AuditLog{ID: "douyin-audit", Product: controlplane.ProductDouyinDesktop, Action: "douyin", Outcome: "success", CreatedAt: now.Add(time.Minute).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed state: %v", err)
	}

	leasePage, err := repository.ListModelLeasesPageWithOptions(context.Background(), ModelLeasePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductAutoLive})
	if err != nil || leasePage.Total != 1 || len(leasePage.Items) != 1 || leasePage.Items[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy lease page = (%+v, %v)", leasePage, err)
	}
	usagePage, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductAutoLive})
	if err != nil || usagePage.Total != 1 || len(usagePage.Items) != 1 || usagePage.Items[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy usage page = (%+v, %v)", usagePage, err)
	}
	auditPage, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductAutoLive})
	if err != nil || auditPage.Total != 1 || len(auditPage.Items) != 1 || auditPage.Items[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy audit page = (%+v, %v)", auditPage, err)
	}

	douyinLeases, err := repository.ListModelLeasesPageWithOptions(context.Background(), ModelLeasePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
	if err != nil || douyinLeases.Total != 1 || douyinLeases.Items[0].ID != "douyin-lease" {
		t.Fatalf("douyin leases = (%+v, %v)", douyinLeases, err)
	}
	douyinUsage, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
	if err != nil || douyinUsage.Total != 1 || douyinUsage.Items[0].ID != "douyin-usage" {
		t.Fatalf("douyin usage = (%+v, %v)", douyinUsage, err)
	}
	douyinAudit, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
	if err != nil || douyinAudit.Total != 1 || douyinAudit.Items[0].ID != "douyin-audit" {
		t.Fatalf("douyin audit = (%+v, %v)", douyinAudit, err)
	}
}
