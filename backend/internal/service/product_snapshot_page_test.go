package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// snapshotProductPageRepository models the compatibility PostgreSQL source:
// it exposes the normalized reader interfaces but rejects using them while its
// authoritative data is still the serialized control-plane state.
type snapshotProductPageRepository struct {
	*store.MemoryStore
}

func (snapshotProductPageRepository) UsesNormalizedReadSource() bool { return false }

func (snapshotProductPageRepository) ListUsersPageForProduct(context.Context, int, int, controlplane.ProductCode) (store.UserPage, error) {
	return store.UserPage{}, controlplane.ErrInvalidRequest
}

func (snapshotProductPageRepository) ListDevicesPageForProduct(context.Context, int, int, controlplane.ProductCode) (store.DevicePage, error) {
	return store.DevicePage{}, controlplane.ErrInvalidRequest
}

func (snapshotProductPageRepository) ListModelPoolAccountsPageForProduct(context.Context, int, int, controlplane.ProductCode) (store.ModelPoolPage, error) {
	return store.ModelPoolPage{}, controlplane.ErrInvalidRequest
}

func TestProductPagesUseSnapshotStateInsteadOfNormalizedReaders(t *testing.T) {
	t.Parallel()
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := snapshotProductPageRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_douyin"] = controlplane.UserSummary{ID: "usr_douyin", Username: "douyin", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		state.UserProducts["usr_douyin:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_douyin", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		state.Devices["dev_douyin"] = controlplane.DeviceSummary{ID: "dev_douyin", UserID: "usr_douyin", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelPoolAccounts["mpa_douyin"] = controlplane.ModelPoolAccountSummary{ID: "mpa_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai-compatible", Model: "gpt", Status: controlplane.ModelAccountStatusActive}
		state.ModelPoolAccounts["mpa_legacy"] = controlplane.ModelPoolAccountSummary{ID: "mpa_legacy", Provider: "openai-compatible", Model: "legacy", Status: controlplane.ModelAccountStatusActive}
		return nil
	}); err != nil {
		t.Fatalf("seed snapshot state: %v", err)
	}

	service := NewControlPlaneWithRepository(repository)
	if items, total, err := service.ListUsersPageForProduct(context.Background(), 1, 20, controlplane.ProductDouyinDesktop); err != nil || total != 1 || len(items) != 1 || items[0].ID != "usr_douyin" {
		t.Fatalf("snapshot users page = (%+v, %d, %v)", items, total, err)
	}
	if items, total, err := service.ListDevicesPageForProduct(context.Background(), 1, 20, controlplane.ProductDouyinDesktop); err != nil || total != 1 || len(items) != 1 || items[0].ID != "dev_douyin" {
		t.Fatalf("snapshot devices page = (%+v, %d, %v)", items, total, err)
	}
	if items, total, err := service.ListModelPoolAccountsPageForProduct(context.Background(), 1, 20, controlplane.ProductDouyinDesktop); err != nil || total != 1 || len(items) != 1 || items[0].ID != "mpa_douyin" {
		t.Fatalf("snapshot model pool page = (%+v, %d, %v)", items, total, err)
	}
	if items, total, err := service.ListModelPoolAccountsPageForProduct(context.Background(), 1, 20, controlplane.ProductAutoLive); err != nil || total != 1 || len(items) != 1 || items[0].ID != "mpa_legacy" {
		t.Fatalf("legacy snapshot model pool page = (%+v, %d, %v)", items, total, err)
	}
}
