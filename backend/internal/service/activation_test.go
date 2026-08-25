package service

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestActivateDeviceConcurrentSingleUseAndIdempotentReplay(t *testing.T) {
	now := time.Date(2026, 8, 21, 16, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "activation-concurrent-code", controlplane.CreateActivationCodeInput{
		UserID:      "usr_local_admin",
		ExpiresAt:  now.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil || code.PlainCode == nil {
		t.Fatalf("CreateActivationCode() = %+v, error = %v", code, err)
	}

	start := make(chan struct{})
	type result struct {
		device controlplane.DeviceSummary
		err    error
		key    string
	}
	results := make(chan result, 2)
	var wait sync.WaitGroup
	for _, test := range []struct {
		key      string
		deviceID string
	}{
		{key: "activation-concurrent-1", deviceID: "dev_concurrent01"},
		{key: "activation-concurrent-2", deviceID: "dev_concurrent02"},
	} {
		test := test
		wait.Add(1)
		go func() {
			defer wait.Done()
			<-start
			device, activateErr := svc.ActivateDevice(ctx, test.key, "usr_local_admin", controlplane.ActivateDeviceInput{
				Device: controlplane.DeviceRegistration{
					DeviceID:   test.deviceID,
					DeviceName: "Concurrent Device",
					Platform:   "windows",
					AppVersion: "1.0.0",
				},
			})
			results <- result{device: device, err: activateErr, key: test.key}
		}()
	}
	close(start)
	wait.Wait()
	close(results)

	var winner result
	var failures int
	for item := range results {
		if item.err == nil {
			winner = item
			continue
		}
		failures++
		if !controlplane.IsErrorCode(item.err, "DEVICE_LIMIT_EXCEEDED") {
			t.Fatalf("concurrent loser error = %v, want DEVICE_LIMIT_EXCEEDED", item.err)
		}
	}
	if winner.err != nil || winner.device.ID == "" || failures != 1 {
		t.Fatalf("concurrent activation winner=%+v failures=%d", winner, failures)
	}

	replay, err := svc.ActivateDevice(ctx, winner.key, "usr_local_admin", controlplane.ActivateDeviceInput{
		Device: controlplane.DeviceRegistration{
			DeviceID:   winner.device.ID,
			DeviceName: "Concurrent Device",
			Platform:   "windows",
			AppVersion: "1.0.0",
		},
	})
	if err != nil {
		t.Fatalf("idempotent replay error = %v", err)
	}
	if replay.ID != winner.device.ID {
		t.Fatalf("idempotent replay device = %+v, want %q", replay, winner.device.ID)
	}

	if err := repository.Run(ctx, func(state *store.State) error {
		if len(state.Devices) != 1 {
			t.Fatalf("activated devices = %d, want 1", len(state.Devices))
		}
		for _, record := range state.ActivationCodes {
			if record.ActivationCode.Status != controlplane.ActivationCodeStatusUsed || record.PlainCode != "" || record.UsedByDeviceID == "" || record.UsedAt == "" {
				t.Fatalf("activation record leaked or has invalid usage state: %+v", record)
			}
		}
		return nil
	}); err != nil {
		t.Fatalf("inspect activation state: %v", err)
	}
	items, err := svc.ListActivationCodes(ctx)
	if err != nil {
		t.Fatalf("ListActivationCodes() error = %v", err)
	}
	if len(items) != 1 || items[0].PlainCode != nil || items[0].UsedByDeviceID == "" {
		t.Fatalf("redacted activation list = %+v", items)
	}
}

func TestRevokeActivationCodeClearsPlaintextFromMemoryState(t *testing.T) {
	now := time.Date(2026, 8, 21, 18, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "activation-revoke-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil || code.PlainCode == nil {
		t.Fatalf("CreateActivationCode() = %+v, error = %v", code, err)
	}
	if _, err := svc.RevokeActivationCode(ctx, "activation-revoke", code.ID); err != nil {
		t.Fatalf("RevokeActivationCode() error = %v", err)
	}
	if err := repository.Run(ctx, func(state *store.State) error {
		if record := state.ActivationCodes[code.ID]; record.PlainCode != "" || record.ActivationCode.PlainCode != nil {
			t.Fatalf("revoked activation code retained plaintext: %+v", record)
		}
		return nil
	}); err != nil {
		t.Fatalf("inspect revoked activation code: %v", err)
	}
}

func TestProductActivationCodeCreateAndRevokeStayInProduct(t *testing.T) {
	now := time.Now().UTC()
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	if err := svc.EnsureLocalAdmin(context.Background(), "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.UserProducts["usr_local_admin:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_local_admin", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		return nil
	}); err != nil {
		t.Fatalf("seed product membership: %v", err)
	}
	code, err := svc.CreateActivationCodeForProduct(context.Background(), controlplane.ProductDouyinDesktop, "product-create-code", controlplane.CreateActivationCodeInput{UserID: "usr_local_admin", ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCodeForProduct() error = %v", err)
	}
	if code.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("created product = %q", code.Product)
	}
	if _, err := svc.RevokeActivationCodeForProduct(context.Background(), controlplane.ProductAutoLive, "product-revoke-wrong", code.ID); !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("cross-product revoke error = %v, want forbidden", err)
	}
	revoked, err := svc.RevokeActivationCodeForProduct(context.Background(), controlplane.ProductDouyinDesktop, "product-revoke-code", code.ID)
	if err != nil {
		t.Fatalf("RevokeActivationCodeForProduct() error = %v", err)
	}
	if revoked.Product != controlplane.ProductDouyinDesktop || revoked.Status != controlplane.ActivationCodeStatusRevoked {
		t.Fatalf("revoked code = %+v", revoked)
	}
}
