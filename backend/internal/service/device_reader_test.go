package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedDeviceReaderRepository struct {
	*store.MemoryStore
	user             controlplane.UserSummary
	device           controlplane.DeviceSummary
	expiresAt        time.Time
	productStatus    string
	membershipStatus string
}

type normalizedReadOnlyRepository struct{ *store.MemoryStore }

func (r *normalizedReadOnlyRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedDeviceReaderRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedDeviceReaderRepository) GetUserByID(context.Context, string) (controlplane.UserSummary, error) {
	return r.user, nil
}

func (r *normalizedDeviceReaderRepository) GetDevice(context.Context, string) (controlplane.DeviceSummary, error) {
	return r.device, nil
}

func (r *normalizedDeviceReaderRepository) GetOwnedDevice(context.Context, string, string) (controlplane.DeviceSummary, error) {
	return r.device, nil
}

func (r *normalizedDeviceReaderRepository) GetActivationExpiry(context.Context, string, string) (*time.Time, error) {
	return &r.expiresAt, nil
}

func (r *normalizedDeviceReaderRepository) ListProducts(context.Context) ([]controlplane.ProductSummary, error) {
	return nil, nil
}

func (r *normalizedDeviceReaderRepository) GetProduct(_ context.Context, product controlplane.ProductCode) (controlplane.ProductSummary, error) {
	status := r.productStatus
	if status == "" {
		status = "active"
	}
	return controlplane.ProductSummary{Code: product, Status: status}, nil
}

func (r *normalizedDeviceReaderRepository) GetUserProductMembership(_ context.Context, userID string, product controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	status := r.membershipStatus
	if status == "" {
		status = "active"
	}
	return controlplane.UserProductMembership{UserID: userID, Product: product, Status: status}, nil
}

func (r *normalizedDeviceReaderRepository) EnsureUserProductMembership(_ context.Context, userID string, product controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	return r.GetUserProductMembership(context.Background(), userID, product)
}

func TestGetClientProfileUsesNormalizedUserAndDeviceReaders(t *testing.T) {
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository := &normalizedDeviceReaderRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time { return now }),
		user: controlplane.UserSummary{
			ID: "usr_1", Username: "alice", Role: controlplane.RoleUser,
			Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339),
		},
		device: controlplane.DeviceSummary{
			ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductAutoLive, DeviceName: "Studio", Platform: "windows",
			AppVersion: "1.2.3", Status: controlplane.DeviceStatusActive, LastSeenAt: now.Format(time.RFC3339),
		},
		expiresAt: now.Add(24 * time.Hour),
	}
	profile, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetClientProfile() error = %v", err)
	}
	if profile.User.ID != "usr_1" || profile.Device.ID != "dev_1" || profile.Device.ActivationExpiresAt == nil || *profile.Device.ActivationExpiresAt != now.Add(24*time.Hour).Format(time.RFC3339) || len(profile.Permissions) != 1 || profile.Permissions[0] != "client" || !profile.Device.Online {
		t.Fatalf("profile = %+v", profile)
	}
}

func TestGetClientProfileForProductRejectsNormalizedDeviceProductMismatch(t *testing.T) {
	repository := &normalizedDeviceReaderRepository{
		MemoryStore: store.NewMemoryStore(time.Now),
		user:        controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive},
		device:      controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive},
	}
	_, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductAutoLive)
	if err != controlplane.ErrForbidden {
		t.Fatalf("GetClientProfileForProduct() error = %v, want forbidden", err)
	}
}

func TestGetClientProfileForDesktopRejectsExpiredActivation(t *testing.T) {
	now := time.Date(2026, 9, 5, 12, 0, 0, 0, time.UTC)
	repository := &normalizedDeviceReaderRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time { return now }),
		user:        controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive},
		device:      controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive},
		expiresAt:   now,
	}

	_, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductDouyinDesktop)
	if err != controlplane.ErrAccountActivationExpired {
		t.Fatalf("GetClientProfileForProduct() error = %v, want activation expired", err)
	}
}

func TestGetClientProfileForDesktopRejectsInactiveMembership(t *testing.T) {
	now := time.Date(2026, 9, 5, 12, 0, 0, 0, time.UTC)
	repository := &normalizedDeviceReaderRepository{
		MemoryStore:      store.NewMemoryStore(func() time.Time { return now }),
		user:             controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive},
		device:           controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive},
		expiresAt:        now.Add(time.Hour),
		membershipStatus: "disabled",
	}

	_, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductDouyinDesktop)
	if err != controlplane.ErrForbidden {
		t.Fatalf("GetClientProfileForProduct() error = %v, want forbidden", err)
	}
}

func TestGetClientProfileForDesktopRejectsRevokedMemoryActivation(t *testing.T) {
	now := time.Date(2026, 9, 5, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Products[string(controlplane.ProductDouyinDesktop)] = controlplane.ProductSummary{Code: controlplane.ProductDouyinDesktop, Status: "active"}
		state.UserProducts["usr_1:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		state.Users["usr_1"] = controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		state.Devices["dev_1"] = controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ActivationCodes["ac_1"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "ac_1", Product: controlplane.ProductDouyinDesktop, UserID: "usr_1", Status: controlplane.ActivationCodeStatusRevoked, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}}
		state.ActivationDeviceBindings["dev_1"] = "ac_1"
		return nil
	}); err != nil {
		t.Fatalf("seed profile state: %v", err)
	}

	_, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductDouyinDesktop)
	if err != controlplane.ErrDeviceBindingRequired {
		t.Fatalf("GetClientProfileForProduct() error = %v, want binding required", err)
	}
}

func TestGetClientProfileForProductRejectsMemoryDeviceProductMismatchWithoutMutation(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_1"] = controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		state.Devices["dev_1"] = controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		return nil
	}); err != nil {
		t.Fatalf("seed profile state: %v", err)
	}
	_, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductAutoLive)
	if err != controlplane.ErrForbidden {
		t.Fatalf("GetClientProfileForProduct() error = %v, want forbidden", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		device := state.Devices["dev_1"]
		if device.Product != controlplane.ProductDouyinDesktop || device.UserID != "usr_1" {
			t.Fatalf("profile mismatch mutated device = %+v", device)
		}
		return nil
	}); err != nil {
		t.Fatalf("verify profile state: %v", err)
	}
}

func TestGetClientProfileForProductAcceptsLegacyAutoliveDeviceWithoutProduct(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_1"] = controlplane.UserSummary{ID: "usr_1", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}
		state.Devices["dev_1"] = controlplane.DeviceSummary{ID: "dev_1", UserID: "usr_1", Status: controlplane.DeviceStatusActive}
		return nil
	}); err != nil {
		t.Fatalf("seed legacy profile state: %v", err)
	}

	profile, err := NewControlPlaneWithRepository(repository).GetClientProfileForProduct(context.Background(), "usr_1", "dev_1", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetClientProfileForProduct() error = %v", err)
	}
	if profile.Product != controlplane.ProductAutoLive || profile.Device.ID != "dev_1" || profile.Device.Product != controlplane.ProductAutoLive {
		t.Fatalf("profile = %+v", profile)
	}
}

func TestNormalizedUserMutationFailsClosedWithoutRepository(t *testing.T) {
	repository := &normalizedReadOnlyRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	_, err := svc.CreateUser(context.Background(), "create-user", controlplane.CreateUserInput{
		Username: "alice", Password: "correct horse battery", Role: controlplane.RoleUser,
	})
	if err != store.ErrNormalizedUserRepositoryRequired {
		t.Fatalf("CreateUser() error = %v, want normalized repository requirement", err)
	}
	if _, err := svc.GetDevice(context.Background(), "dev_1"); err != store.ErrNormalizedDeviceReaderRequired {
		t.Fatalf("GetDevice() error = %v, want normalized device reader requirement", err)
	}
	if _, err := svc.GetDeviceForProduct(context.Background(), "dev_1", controlplane.ProductAutoLive); err != store.ErrNormalizedDeviceReaderRequired {
		t.Fatalf("GetDeviceForProduct() error = %v, want normalized device reader requirement", err)
	}
}

func TestGetDeviceForProductRejectsSnapshotProductMismatch(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices["dev_1"] = controlplane.DeviceSummary{ID: "dev_1", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		return nil
	}); err != nil {
		t.Fatalf("seed device state: %v", err)
	}
	_, err := NewControlPlaneWithRepository(repository).GetDeviceForProduct(context.Background(), "dev_1", controlplane.ProductAutoLive)
	if err != controlplane.ErrDeviceNotFound {
		t.Fatalf("GetDeviceForProduct() error = %v, want device not found", err)
	}
}
