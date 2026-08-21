package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestRecordHeartbeatPersistsCurrentMediaAndPlaybackState(t *testing.T) {
	now := time.Date(2026, 8, 20, 10, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "heartbeat-code", controlplane.CreateActivationCodeInput{
		ExpiresAt:  now.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	if code.PlainCode == nil {
		t.Fatal("CreateActivationCode() returned no plain code")
	}
	device, err := svc.ActivateDevice(ctx, "heartbeat-device", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: *code.PlainCode,
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_heartbeat01",
			DeviceName: "Test Device",
			Platform:   "windows",
			AppVersion: "1.0.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}

	result, err := svc.RecordHeartbeat(ctx, "heartbeat-request", "usr_local_admin", controlplane.HeartbeatInput{
		DeviceID: device.ID,
		SentAt:   now,
		Status: controlplane.HeartbeatStatus{
			DiskFreeBytes:    100,
			CurrentMediaName: "demo.mp4",
			PlaybackState:    "playing",
		},
	})
	if err != nil {
		t.Fatalf("RecordHeartbeat() error = %v", err)
	}
	if result.DeviceStatus != controlplane.DeviceStatusActive || result.AcceptedAt == "" {
		t.Fatalf("heartbeat result = %+v", result)
	}

	profile, err := svc.GetClientProfile(ctx, "usr_local_admin", device.ID)
	if err != nil {
		t.Fatalf("GetClientProfile() error = %v", err)
	}
	if profile.Device.CurrentMediaName != "demo.mp4" || profile.Device.PlaybackState != "playing" {
		t.Fatalf("persisted playback state = %+v", profile.Device)
	}
	if !profile.Device.Online {
		t.Fatal("profile device should be online immediately after heartbeat")
	}
	devices, err := svc.ListDevicesForUser(ctx, "usr_local_admin")
	if err != nil {
		t.Fatalf("ListDevicesForUser() error = %v", err)
	}
	if len(devices) != 1 || devices[0].ID != device.ID || !devices[0].Online {
		t.Fatalf("user devices = %+v", devices)
	}

	now = now.Add(3 * time.Minute)
	detail, err := svc.GetDevice(ctx, device.ID)
	if err != nil {
		t.Fatalf("GetDevice() error = %v", err)
	}
	if detail.Online {
		t.Fatalf("stale device should be offline: %+v", detail)
	}
}

func TestDisableDeviceReleasesLeases(t *testing.T) {
	now := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "disable-device-code", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(ctx, "disable-device-device", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: *code.PlainCode,
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_disable01", DeviceName: "Test Device", Platform: "windows", AppVersion: "1.0.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	if _, err := svc.CreateModelPoolAccount(ctx, "disable-device-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", APIKey: "sk-model-secret", Priority: 1, DailyLimit: 100, ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	lease, err := svc.CreateModelLease(ctx, "disable-device-lease", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{
		Provider: "openai-compatible", Model: "rewrite-model", Purpose: "realtime_script", MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	if _, err := svc.DisableDevice(ctx, "disable-device", device.ID); err != nil {
		t.Fatalf("DisableDevice() error = %v", err)
	}
	if err := svc.repository.Run(ctx, func(state *store.State) error {
		if state.ModelLeases[lease.ID].Status != controlplane.ModelLeaseStatusReleased {
			t.Fatalf("lease status = %q, want released", state.ModelLeases[lease.ID].Status)
		}
		if state.Devices[device.ID].Status != controlplane.DeviceStatusDisabled {
			t.Fatalf("device status = %q, want disabled", state.Devices[device.ID].Status)
		}
		return nil
	}); err != nil {
		t.Fatalf("inspect state: %v", err)
	}
}

func TestActivationCodeListExposesRedactedUsageDetails(t *testing.T) {
	now := time.Date(2026, 8, 20, 14, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "activation-details-code", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	items, err := svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() before activation error = %v", err)
	}
	if len(items) != 1 || items[0].PlainCode != nil || items[0].CodePrefix == "" || items[0].UsedAt != "" {
		t.Fatalf("unactivated code details = %+v", items)
	}
	device, err := svc.ActivateDevice(ctx, "activation-details-device", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: *code.PlainCode,
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_activation01", DeviceName: "Details Device", Platform: "windows", AppVersion: "1.0.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	items, err = svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() after activation error = %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("activation code count = %d, want 1", len(items))
	}
	used := items[0]
	if used.PlainCode != nil || used.CodePrefix == *code.PlainCode || used.UsedByUserID != "usr_local_admin" || used.UsedByDeviceID != device.ID || used.UsedAt != now.Format(time.RFC3339) {
		t.Fatalf("used code details = %+v", used)
	}
	if _, err := svc.UnbindDevice(ctx, "activation-details-unbind", device.ID); err != nil {
		t.Fatalf("UnbindDevice() error = %v", err)
	}
	items, err = svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() after unbind error = %v", err)
	}
	if items[0].UsedByUserID != "usr_local_admin" || items[0].UsedByDeviceID != device.ID {
		t.Fatalf("historical usage details lost after unbind = %+v", items[0])
	}
}
