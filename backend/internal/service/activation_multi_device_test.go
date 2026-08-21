package service

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestActivationCodeSupportsMultipleDevices(t *testing.T) {
	now := time.Date(2026, 8, 21, 20, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()

	code, err := svc.CreateActivationCode(ctx, "multi-device-code", controlplane.CreateActivationCodeInput{
		ExpiresAt:  now.Add(time.Hour),
		MaxDevices: 2,
	})
	if err != nil || code.PlainCode == nil {
		t.Fatalf("CreateActivationCode() = %+v, error = %v", code, err)
	}

	for index, deviceID := range []string{"dev_multi01", "dev_multi02"} {
		device, activateErr := svc.ActivateDevice(ctx, "multi-device-activation-"+deviceID, "usr_local_admin", controlplane.ActivateDeviceInput{
			ActivationCode: *code.PlainCode,
			Device: controlplane.DeviceRegistration{
				DeviceID:   deviceID,
				DeviceName: "Multi Device",
				Platform:   "windows",
				AppVersion: "1.0.0",
			},
		})
		if activateErr != nil {
			t.Fatalf("activation %d error = %v", index+1, activateErr)
		}
		if device.ID != deviceID {
			t.Fatalf("activation %d device = %+v", index+1, device)
		}
	}

	items, err := svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() error = %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("activation list length = %d, want 1", len(items))
	}
	if items[0].Status != controlplane.ActivationCodeStatusUsed {
		t.Fatalf("activation status = %q, want used", items[0].Status)
	}
	payload, err := json.Marshal(items[0])
	if err != nil {
		t.Fatalf("marshal activation code = %v", err)
	}
	var fields map[string]any
	if err := json.Unmarshal(payload, &fields); err != nil {
		t.Fatalf("unmarshal activation code = %v", err)
	}
	if fields["max_devices"] != float64(2) || fields["bound_devices"] != float64(2) {
		t.Fatalf("activation capacity fields = %v", fields)
	}
	if fields["plain_code"] != nil {
		t.Fatalf("activation list leaked plaintext: %v", fields["plain_code"])
	}

	_, err = svc.ActivateDevice(ctx, "multi-device-activation-dev_multi03", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: *code.PlainCode,
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_multi03",
			DeviceName: "Multi Device",
			Platform:   "windows",
			AppVersion: "1.0.0",
		},
	})
	if !controlplane.IsErrorCode(err, "ACTIVATION_CODE_USED") {
		t.Fatalf("third activation error = %v, want ACTIVATION_CODE_USED", err)
	}
}

func TestActivationCodeMaxDevicesRange(t *testing.T) {
	now := time.Date(2026, 8, 21, 20, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	for _, maxDevices := range []int{-1, 101} {
		_, err := svc.CreateActivationCode(context.Background(), fmt.Sprintf("range-code-%d", maxDevices), controlplane.CreateActivationCodeInput{
			ExpiresAt:  now.Add(time.Hour),
			MaxDevices: maxDevices,
		})
		if !controlplane.IsErrorCode(err, "INVALID_ARGUMENT") {
			t.Fatalf("max_devices=%d error = %v, want INVALID_ARGUMENT", maxDevices, err)
		}
	}
}
