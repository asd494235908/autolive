package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestAccountActivationBindsOwnedDevicesWithinCapacity(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}

	code, err := svc.CreateActivationCode(ctx, "account-code", controlplane.CreateActivationCodeInput{
		UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 2,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	if code.UserID != "usr_local_admin" {
		t.Fatalf("activation user = %q, want usr_local_admin", code.UserID)
	}

	firstInput := controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account01")}
	first, err := svc.ActivateDevice(ctx, "account-device-1", "usr_local_admin", firstInput)
	if err != nil || first.ID != firstInput.Device.DeviceID {
		t.Fatalf("first activation = %+v, error = %v", first, err)
	}
	if _, err := svc.ActivateDevice(ctx, "account-device-1-relogin", "usr_local_admin", firstInput); err != nil {
		t.Fatalf("same device relogin error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "account-device-2", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account02")}); err != nil {
		t.Fatalf("second activation error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "account-device-3", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account03")}); !controlplane.IsErrorCode(err, "DEVICE_LIMIT_EXCEEDED") {
		t.Fatalf("third activation error = %v, want DEVICE_LIMIT_EXCEEDED", err)
	}

	items, err := svc.ListActivationCodes(ctx)
	if err != nil || len(items) != 1 {
		t.Fatalf("ListActivationCodes() = %+v, error = %v", items, err)
	}
	if items[0].BoundDevices != 2 || items[0].Status != controlplane.ActivationCodeStatusUsed {
		t.Fatalf("activation capacity = %+v", items[0])
	}
}

func TestAccountActivationCannotUseAnotherUsersAuthorization(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	user, err := svc.CreateUser(ctx, "account-user", controlplane.CreateUserInput{Username: "account_user", Password: "password-123-001", Role: controlplane.RoleUser})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if _, err := svc.CreateActivationCode(ctx, "admin-only-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1}); err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}

	_, err = svc.ActivateDevice(ctx, "other-user-device", user.ID, controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account_other")})
	if !controlplane.IsErrorCode(err, "ACCOUNT_ACTIVATION_REQUIRED") {
		t.Fatalf("other user activation error = %v, want ACCOUNT_ACTIVATION_REQUIRED", err)
	}
}

func TestAccountActivationExpiredAndUnbindReleasesCapacity(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	if _, err := svc.CreateActivationCode(ctx, "unbind-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1}); err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "unbind-device-1", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account_unbind1")}); err != nil {
		t.Fatalf("activate first device: %v", err)
	}
	if _, err := svc.UnbindDevice(ctx, "unbind-account-device", "dev_account_unbind1"); err != nil {
		t.Fatalf("UnbindDevice() error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "unbind-device-2", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account_unbind2")}); err != nil {
		t.Fatalf("activation after unbind error = %v", err)
	}

	now = now.Add(2 * time.Hour)
	_, err := svc.ActivateDevice(ctx, "expired-device", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_account_expired")})
	if !controlplane.IsErrorCode(err, "ACCOUNT_ACTIVATION_EXPIRED") {
		t.Fatalf("expired activation error = %v, want ACCOUNT_ACTIVATION_EXPIRED", err)
	}
}

func TestCreateActivationCodeRequiresBoundUser(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	_, err := svc.CreateActivationCode(context.Background(), "missing-account", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("missing user error = %v, want INVALID_ARGUMENT", err)
	}
}

func TestCreateActivationCodeRequiresExplicitDeviceLimit(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	_, err := svc.CreateActivationCode(context.Background(), "missing-limit", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour)})
	if !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
		t.Fatalf("missing max_devices error = %v, want INVALID_ARGUMENT", err)
	}
}

func TestRevokedFullAuthorizationKeepsExistingDeviceButRejectsNewDevice(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "revoke-full-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	existing := controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_revoked_existing")}
	if _, err := svc.ActivateDevice(ctx, "revoke-existing-first", "usr_local_admin", existing); err != nil {
		t.Fatalf("activate existing device: %v", err)
	}
	revoked, err := svc.RevokeActivationCode(ctx, "revoke-used-code", code.ID)
	if err != nil || revoked.Status != controlplane.ActivationCodeStatusRevoked {
		t.Fatalf("revoke used authorization = %+v, error = %v", revoked, err)
	}
	if _, err := svc.ActivateDevice(ctx, "revoke-existing-relogin", "usr_local_admin", existing); err != nil {
		t.Fatalf("existing device relogin after revoke: %v", err)
	}
	_, err = svc.ActivateDevice(ctx, "revoke-new-device", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_revoked_new")})
	if !controlplane.IsErrorCode(err, "ACCOUNT_ACTIVATION_REQUIRED") {
		t.Fatalf("new device after revoke error = %v, want ACCOUNT_ACTIVATION_REQUIRED", err)
	}
}

func TestAccountActivationReturnsStableExistingDeviceErrors(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	if _, err := svc.CreateActivationCode(ctx, "stable-errors-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 2}); err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device := testAccountDevice("dev_stable_errors")
	if _, err := svc.ActivateDevice(ctx, "stable-errors-activate", "usr_local_admin", controlplane.ActivateDeviceInput{Device: device}); err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	if _, err := svc.DisableDevice(ctx, "stable-errors-disable", device.DeviceID); err != nil {
		t.Fatalf("DisableDevice() error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "stable-errors-disabled-relogin", "usr_local_admin", controlplane.ActivateDeviceInput{Device: device}); !controlplane.IsErrorCode(err, "DEVICE_DISABLED") {
		t.Fatalf("disabled device error = %v, want DEVICE_DISABLED", err)
	}
	other, err := svc.CreateUser(ctx, "stable-errors-user", controlplane.CreateUserInput{Username: "stable_error_user", Password: "password-123-001", Role: controlplane.RoleUser})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "stable-errors-conflict", other.ID, controlplane.ActivateDeviceInput{Device: device}); !controlplane.IsErrorCode(err, "DEVICE_BINDING_CONFLICT") {
		t.Fatalf("other account device error = %v, want DEVICE_BINDING_CONFLICT", err)
	}
}

func TestAccountActivationSelectsEarliestExpiryThenID(t *testing.T) {
	now := time.Date(2026, 8, 24, 8, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	later, err := svc.CreateActivationCode(ctx, "later-grant", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(2 * time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("create later grant: %v", err)
	}
	earlier, err := svc.CreateActivationCode(ctx, "earlier-grant", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("create earlier grant: %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "ordered-grant-device", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_ordered_grant")}); err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	equalFirst, err := svc.CreateActivationCode(ctx, "equal-first-grant", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(3 * time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("create first equal-expiry grant: %v", err)
	}
	equalSecond, err := svc.CreateActivationCode(ctx, "equal-second-grant", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(3 * time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("create second equal-expiry grant: %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "ordered-grant-device-2", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_ordered_grant_2")}); err != nil {
		t.Fatalf("activate second ordered device: %v", err)
	}
	if _, err := svc.ActivateDevice(ctx, "ordered-grant-device-3", "usr_local_admin", controlplane.ActivateDeviceInput{Device: testAccountDevice("dev_ordered_grant_3")}); err != nil {
		t.Fatalf("activate third ordered device: %v", err)
	}
	items, err := svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() error = %v", err)
	}
	boundByID := make(map[string]int, len(items))
	for _, item := range items {
		boundByID[item.ID] = item.BoundDevices
	}
	if boundByID[earlier.ID] != 1 || boundByID[later.ID] != 1 || boundByID[equalFirst.ID] != 1 || boundByID[equalSecond.ID] != 0 {
		t.Fatalf("ordered grant capacity = %+v, earlier=%s later=%s equal=(%s,%s)", boundByID, earlier.ID, later.ID, equalFirst.ID, equalSecond.ID)
	}
}

func testAccountDevice(deviceID string) controlplane.DeviceRegistration {
	return controlplane.DeviceRegistration{
		Product: controlplane.ProductAutoLive, DeviceID: deviceID, DeviceName: "Account Device", Platform: "windows", AppVersion: "1.0.0",
	}
}
