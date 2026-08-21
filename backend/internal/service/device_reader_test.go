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
	user   controlplane.UserSummary
	device controlplane.DeviceSummary
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

func TestGetClientProfileUsesNormalizedUserAndDeviceReaders(t *testing.T) {
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository := &normalizedDeviceReaderRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time { return now }),
		user: controlplane.UserSummary{
			ID: "usr_1", Username: "alice", Role: controlplane.RoleUser,
			Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339),
		},
		device: controlplane.DeviceSummary{
			ID: "dev_1", UserID: "usr_1", DeviceName: "Studio", Platform: "windows",
			AppVersion: "1.2.3", Status: controlplane.DeviceStatusActive, LastSeenAt: now.Format(time.RFC3339),
		},
	}
	profile, err := NewControlPlaneWithRepository(repository).GetClientProfile(context.Background(), "usr_1", "dev_1")
	if err != nil {
		t.Fatalf("GetClientProfile() error = %v", err)
	}
	if profile.User.ID != "usr_1" || profile.Device.ID != "dev_1" || len(profile.Permissions) != 1 || profile.Permissions[0] != "client" || !profile.Device.Online {
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
}
