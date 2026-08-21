package store

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestMemoryStoreRunHonorsContextAndPersistsState(t *testing.T) {
	repository := NewMemoryStore(func() time.Time { return time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC) })

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := repository.Run(ctx, func(state *State) error {
		state.Users["usr_cancelled"] = controlplane.UserSummary{ID: "usr_cancelled"}
		return nil
	}); !errors.Is(err, context.Canceled) {
		t.Fatalf("Run() cancelled error = %v, want context.Canceled", err)
	}

	if err := repository.Run(context.Background(), func(state *State) error {
		state.Users["usr_committed"] = controlplane.UserSummary{ID: "usr_committed"}
		return nil
	}); err != nil {
		t.Fatalf("Run() error = %v", err)
	}

	_, err := WithState(repository, func(state *State) (struct{}, error) {
		if _, ok := state.Users["usr_cancelled"]; ok {
			t.Fatal("cancelled transaction changed state")
		}
		if _, ok := state.Users["usr_committed"]; !ok {
			t.Fatal("committed transaction did not change state")
		}
		return struct{}{}, nil
	})
	if err != nil {
		t.Fatalf("WithState() error = %v", err)
	}
}

func TestMemoryStorePageReadersReturnStableBoundedResults(t *testing.T) {
	repository := NewMemoryStore(func() time.Time { return time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC) })
	if err := repository.Run(context.Background(), func(state *State) error {
		state.Users["usr_00000002"] = controlplane.UserSummary{ID: "usr_00000002", Username: "bravo"}
		state.Users["usr_00000001"] = controlplane.UserSummary{ID: "usr_00000001", Username: "alpha"}
		state.Devices["dev_00000002"] = controlplane.DeviceSummary{ID: "dev_00000002", UserID: "usr_00000001"}
		state.Devices["dev_00000001"] = controlplane.DeviceSummary{ID: "dev_00000001", UserID: "usr_00000001"}
		return nil
	}); err != nil {
		t.Fatalf("seed state error = %v", err)
	}

	users, err := repository.ListUsersPage(context.Background(), 1, 1)
	if err != nil {
		t.Fatalf("ListUsersPage() error = %v", err)
	}
	if users.Total != 2 || len(users.Items) != 1 || users.Items[0].ID != "usr_00000002" {
		t.Fatalf("users page = %+v", users)
	}
	devices, err := repository.ListDevicesForUserPage(context.Background(), "usr_00000001", 0, 1)
	if err != nil {
		t.Fatalf("ListDevicesForUserPage() error = %v", err)
	}
	if devices.Total != 2 || len(devices.Items) != 1 || devices.Items[0].ID != "dev_00000001" {
		t.Fatalf("devices page = %+v", devices)
	}
	if err := repository.Run(context.Background(), func(state *State) error {
		state.ModelUsageRecords["usage_00000001"] = controlplane.ModelUsageRecord{ID: "usage_00000001", CreatedAt: "2026-08-20T00:00:00Z"}
		state.AuditLogs["audit_00000001"] = controlplane.AuditLog{ID: "audit_00000001", CreatedAt: "2026-08-20T00:00:00Z"}
		state.ModelLeases["lease_00000001"] = controlplane.ModelLease{ID: "lease_00000001", AccountID: "mpa_00000001", UserID: "usr_00000001", DeviceID: "dev_00000001", Purpose: "client", Provider: "openai", Model: "rewrite", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: "2026-08-20T02:00:00Z", ProxyMode: controlplane.ModelLeaseProxyModeDirectLease, ConcurrencyLimit: 2}
		state.ModelLeases["lease_00000002"] = controlplane.ModelLease{ID: "lease_00000002", AccountID: "mpa_00000002", UserID: "usr_00000002", DeviceID: "dev_00000002", Purpose: "client", Provider: "openai", Model: "rewrite", Status: controlplane.ModelLeaseStatusReleased, ExpiresAt: "2026-08-19T02:00:00Z", ProxyMode: controlplane.ModelLeaseProxyModeDirectLease, ConcurrencyLimit: 1}
		return nil
	}); err != nil {
		t.Fatalf("seed usage/audit state error = %v", err)
	}
	usage, err := repository.ListModelUsagePage(context.Background(), 0, 1)
	if err != nil || usage.Total != 1 || len(usage.Items) != 1 {
		t.Fatalf("usage page = %+v, error = %v", usage, err)
	}
	audit, err := repository.ListAuditLogsPage(context.Background(), 0, 1)
	if err != nil || audit.Total != 1 || len(audit.Items) != 1 {
		t.Fatalf("audit page = %+v, error = %v", audit, err)
	}
	leases, err := repository.ListModelLeasesPage(context.Background(), 0, 1)
	if err != nil || leases.Total != 2 || len(leases.Items) != 1 || leases.Items[0].ID != "lease_00000001" || leases.Items[0].UserID != "usr_00000001" {
		t.Fatalf("leases page = %+v, error = %v", leases, err)
	}
	if _, err := repository.ListUsersPage(context.Background(), -1, 1); err == nil {
		t.Fatal("ListUsersPage() accepted invalid offset")
	}
}

func TestMemoryStoreAuditLogPageWithOptionsFiltersAndSorts(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *State) error {
		state.AuditLogs["audit-old"] = controlplane.AuditLog{
			ID: "audit-old", ActorUserID: "usr-a", DeviceID: "dev-a", Action: "POST /users", TargetType: "user", Outcome: "success", CreatedAt: "2026-08-20T00:00:00Z",
		}
		state.AuditLogs["audit-new"] = controlplane.AuditLog{
			ID: "audit-new", ActorUserID: "usr-a", DeviceID: "dev-a", Action: "PATCH /users", TargetType: "user", Outcome: "failure", ErrorCode: "CONFLICT", CreatedAt: "2026-08-21T00:00:00Z",
		}
		state.AuditLogs["audit-other"] = controlplane.AuditLog{
			ID: "audit-other", ActorUserID: "usr-b", DeviceID: "dev-b", Action: "POST /devices", TargetType: "device", Outcome: "failure", CreatedAt: "2026-08-21T01:00:00Z",
		}
		return nil
	}); err != nil {
		t.Fatalf("seed audit logs error = %v", err)
	}
	page, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{
		Offset: 0, Limit: 20, ActorUserID: "usr-a", Outcome: "failure", CreatedAfter: mustAuditTime(t, "2026-08-20T12:00:00Z"), Sort: AuditLogSortCreatedAsc,
	})
	if err != nil {
		t.Fatalf("ListAuditLogsPageWithOptions() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].ID != "audit-new" {
		t.Fatalf("filtered audit page = %+v", page)
	}
	if _, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{Offset: 0, Limit: 20, Sort: "created_at"}); err == nil {
		t.Fatal("ListAuditLogsPageWithOptions() accepted unsupported sort")
	}
}

func TestCompareAuditLogCreatedAtUsesTimeInstant(t *testing.T) {
	if got := CompareAuditLogCreatedAt("2026-08-21T01:00:00+02:00", "2026-08-20T22:30:00Z"); got <= 0 {
		t.Fatalf("expected first timestamp to be later, got %d", got)
	}
}

func TestNewStateInitializesProductRegistries(t *testing.T) {
	state := NewState()
	if state.Products == nil || state.UserProducts == nil {
		t.Fatal("NewState() did not initialize product registries")
	}

	state.Products = nil
	state.UserProducts = nil
	if got := ensureStateMaps(state); got.Products == nil || got.UserProducts == nil {
		t.Fatal("ensureStateMaps() did not restore product registries")
	}
}

func mustAuditTime(t *testing.T, value string) *time.Time {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339, value)
	if err != nil {
		t.Fatalf("time.Parse(%q) error = %v", value, err)
	}
	return &parsed
}
