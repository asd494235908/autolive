package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type productPageFallbackRepository struct {
	state       *store.State
	now         time.Time
	legacyCalls map[string]int
}

func (r *productPageFallbackRepository) Now() time.Time { return r.now }

func (r *productPageFallbackRepository) Run(ctx context.Context, fn store.StateOperation) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return fn(r.state)
}

func (r *productPageFallbackRepository) ListModelLeasesPage(context.Context, int, int) (store.ModelLeasePage, error) {
	r.legacyCalls["leases"]++
	return store.ModelLeasePage{Items: []controlplane.ModelLeaseAdminSummary{{ID: "legacy-lease"}, {ID: "douyin-lease"}}, Total: 2}, nil
}

func (r *productPageFallbackRepository) ListModelUsagePage(context.Context, int, int) (store.ModelUsagePage, error) {
	r.legacyCalls["usage"]++
	return store.ModelUsagePage{Items: []controlplane.ModelUsageRecord{{ID: "legacy-usage"}, {ID: "douyin-usage"}}, Total: 2}, nil
}

func (r *productPageFallbackRepository) ListAuditLogsPage(context.Context, int, int) (store.AuditPage, error) {
	r.legacyCalls["audit"]++
	return store.AuditPage{Items: []controlplane.AuditLog{{ID: "legacy-audit"}, {ID: "douyin-audit"}}, Total: 2}, nil
}

type runOnlyProductPageRepository struct {
	state *store.State
	now   time.Time
}

func (r *runOnlyProductPageRepository) Now() time.Time { return r.now }

func (r *runOnlyProductPageRepository) Run(ctx context.Context, fn store.StateOperation) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return fn(r.state)
}

func newProductPageFallbackRepository() *productPageFallbackRepository {
	state := store.NewState()
	state.ModelLeases["legacy-lease"] = controlplane.ModelLease{ID: "legacy-lease", Provider: "openai", Model: "legacy", Status: controlplane.ModelLeaseStatusActive}
	state.ModelLeases["douyin-lease"] = controlplane.ModelLease{ID: "douyin-lease", Product: controlplane.ProductDouyinDesktop, Provider: "openai", Model: "douyin", Status: controlplane.ModelLeaseStatusActive}
	state.ModelUsageRecords["legacy-usage"] = controlplane.ModelUsageRecord{ID: "legacy-usage", LeaseID: "legacy-lease", Provider: "openai", Model: "legacy", CreatedAt: "2026-08-22T10:00:00Z"}
	state.ModelUsageRecords["douyin-usage"] = controlplane.ModelUsageRecord{ID: "douyin-usage", Product: controlplane.ProductDouyinDesktop, LeaseID: "douyin-lease", Provider: "openai", Model: "douyin", CreatedAt: "2026-08-22T10:01:00Z"}
	state.AuditLogs["legacy-audit"] = controlplane.AuditLog{ID: "legacy-audit", Action: "seed", Outcome: "success", CreatedAt: "2026-08-22T10:00:00Z"}
	state.AuditLogs["douyin-audit"] = controlplane.AuditLog{ID: "douyin-audit", Product: controlplane.ProductDouyinDesktop, Action: "seed", Outcome: "success", CreatedAt: "2026-08-22T10:01:00Z"}
	return &productPageFallbackRepository{
		state:       state,
		now:         time.Date(2026, 8, 22, 10, 2, 0, 0, time.UTC),
		legacyCalls: map[string]int{},
	}
}

func TestProductScopedPagesSkipLegacyReadersAndFilterState(t *testing.T) {
	repository := newProductPageFallbackRepository()
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	leases, total, err := service.ListModelLeasesPageWithOptions(ctx, 1, 20, ModelLeaseListOptions{Product: controlplane.ProductDouyinDesktop})
	if err != nil || total != 1 || len(leases) != 1 || leases[0].ID != "douyin-lease" {
		t.Fatalf("product leases = (%+v, %d, %v)", leases, total, err)
	}
	usage, total, err := service.ListModelUsagePageWithOptions(ctx, 1, 20, ModelUsageListOptions{Product: controlplane.ProductDouyinDesktop})
	if err != nil || total != 1 || len(usage) != 1 || usage[0].ID != "douyin-usage" {
		t.Fatalf("product usage = (%+v, %d, %v)", usage, total, err)
	}
	audit, total, err := service.ListAuditLogsPageWithOptions(ctx, 1, 20, AuditLogListOptions{Product: controlplane.ProductDouyinDesktop})
	if err != nil || total != 1 || len(audit) != 1 || audit[0].ID != "douyin-audit" {
		t.Fatalf("product audit = (%+v, %d, %v)", audit, total, err)
	}
	for name, calls := range repository.legacyCalls {
		if calls != 0 {
			t.Fatalf("legacy %s page reader calls = %d, want 0 for product-scoped read", name, calls)
		}
	}
}

func TestRunOnlyProductPagesNormalizeLegacyAutoliveRecords(t *testing.T) {
	legacy := newProductPageFallbackRepository()
	repository := &runOnlyProductPageRepository{state: legacy.state, now: legacy.now}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	leases, total, err := service.ListModelLeasesPageWithOptions(ctx, 1, 20, ModelLeaseListOptions{Product: controlplane.ProductAutoLive})
	if err != nil || total != 1 || len(leases) != 1 || leases[0].ID != "legacy-lease" || leases[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy autolive leases = (%+v, %d, %v)", leases, total, err)
	}
	usage, total, err := service.ListModelUsagePageWithOptions(ctx, 1, 20, ModelUsageListOptions{Product: controlplane.ProductAutoLive})
	if err != nil || total != 1 || len(usage) != 1 || usage[0].ID != "legacy-usage" || usage[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy autolive usage = (%+v, %d, %v)", usage, total, err)
	}
	audit, total, err := service.ListAuditLogsPageWithOptions(ctx, 1, 20, AuditLogListOptions{Product: controlplane.ProductAutoLive})
	if err != nil || total != 1 || len(audit) != 1 || audit[0].ID != "legacy-audit" || audit[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy autolive audit = (%+v, %d, %v)", audit, total, err)
	}
}

func TestLegacyPageReadersRemainUsedForUnscopedReads(t *testing.T) {
	repository := newProductPageFallbackRepository()
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	if _, _, err := service.ListModelLeasesPageWithOptions(ctx, 1, 20, ModelLeaseListOptions{}); err != nil {
		t.Fatalf("unscoped leases error = %v", err)
	}
	if _, _, err := service.ListModelUsagePageWithOptions(ctx, 1, 20, ModelUsageListOptions{}); err != nil {
		t.Fatalf("unscoped usage error = %v", err)
	}
	if _, _, err := service.ListAuditLogsPageWithOptions(ctx, 1, 20, AuditLogListOptions{}); err != nil {
		t.Fatalf("unscoped audit error = %v", err)
	}
	for _, name := range []string{"leases", "usage", "audit"} {
		if repository.legacyCalls[name] != 1 {
			t.Fatalf("legacy %s page reader calls = %d, want 1", name, repository.legacyCalls[name])
		}
	}
}

func TestMemoryActivationPageNormalizesLegacyAutoliveProduct(t *testing.T) {
	repository := store.NewMemoryStore(func() time.Time { return time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC) })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ActivationCodes["legacy"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "legacy", Status: controlplane.ActivationCodeStatusActive, ExpiresAt: "2026-08-23T10:00:00Z"}}
		state.ActivationCodes["douyin"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "douyin", Product: controlplane.ProductDouyinDesktop, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: "2026-08-23T10:00:00Z"}}
		return nil
	}); err != nil {
		t.Fatalf("seed activation state: %v", err)
	}
	service := NewControlPlaneWithRepository(repository)
	items, total, err := service.ListActivationCodesPageForProduct(context.Background(), 1, 20, controlplane.ProductAutoLive)
	if err != nil || total != 1 || len(items) != 1 || items[0].ID != "legacy" || items[0].Product != controlplane.ProductAutoLive {
		t.Fatalf("legacy autolive activation page = (%+v, %d, %v)", items, total, err)
	}
}

func TestNormalizedProductScopedPageWithoutReaderFailsClosed(t *testing.T) {
	repository := &normalizedPageFallbackRepository{}
	service := NewControlPlaneWithRepository(repository)
	for _, test := range []struct {
		name string
		want error
		call func() error
	}{
		{name: "usage", want: store.ErrNormalizedModelUsagePageReaderRequired, call: func() error {
			_, _, err := service.ListModelUsagePageWithOptions(context.Background(), 1, 20, ModelUsageListOptions{Product: controlplane.ProductAutoLive})
			return err
		}},
		{name: "lease", want: store.ErrNormalizedModelLeasePageReaderRequired, call: func() error {
			_, _, err := service.ListModelLeasesPageWithOptions(context.Background(), 1, 20, ModelLeaseListOptions{Product: controlplane.ProductAutoLive})
			return err
		}},
		{name: "audit", want: store.ErrNormalizedAuditPageReaderRequired, call: func() error {
			_, _, err := service.ListAuditLogsPageWithOptions(context.Background(), 1, 20, AuditLogListOptions{Product: controlplane.ProductAutoLive})
			return err
		}},
	} {
		t.Run(test.name, func(t *testing.T) {
			if err := test.call(); !errors.Is(err, test.want) {
				t.Fatalf("normalized product page error = %v, want %v", err, test.want)
			}
		})
	}
}
