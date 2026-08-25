package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// Normalized device/session mutations must not silently execute the legacy
// StateOperation when their atomic domain Repository capability is missing.
func TestNormalizedDeviceMutationsFailClosedWithoutTransactionalRepositories(t *testing.T) {
	repository := &normalizedDeviceMutationFallbackRepository{
		MemoryStore: store.NewMemoryStore(func() time.Time {
			return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
		}),
	}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()
	now := repository.Now()
	heartbeat := controlplane.HeartbeatInput{DeviceID: "device_1", SentAt: now}
	activation := controlplane.ActivateDeviceInput{
		Device: controlplane.DeviceRegistration{
			DeviceID: "device_1", DeviceName: "Studio", Platform: "windows", AppVersion: "1.0.0",
		},
	}

	tests := []struct {
		name string
		want error
		call func() error
	}{
		{name: "heartbeat", want: store.ErrNormalizedTransactionalHeartbeatRecorderRequired, call: func() error {
			_, err := service.RecordHeartbeat(ctx, "heartbeat-id", "user_1", heartbeat)
			return err
		}},
		{name: "heartbeat_session", want: store.ErrNormalizedTransactionalHeartbeatRecorderRequired, call: func() error {
			_, err := service.RecordHeartbeatWithSessionBindingAndAudit(ctx, "heartbeat-id", "user_1", "token-hash", heartbeat, controlplane.AuditLogInput{})
			return err
		}},
		{name: "activate", want: store.ErrNormalizedTransactionalDeviceActivatorRequired, call: func() error {
			_, err := service.ActivateDevice(ctx, "activate-id", "user_1", activation)
			return err
		}},
		{name: "activate_session", want: store.ErrNormalizedTransactionalDeviceActivatorRequired, call: func() error {
			_, err := service.ActivateDeviceWithSessionBindingAndAudit(ctx, "activate-id", "user_1", "token-hash", activation, controlplane.AuditLogInput{})
			return err
		}},
		{name: "disable", want: store.ErrNormalizedDeviceLifecycleRepositoryRequired, call: func() error {
			_, err := service.DisableDeviceWithAudit(ctx, "disable-id", "device_1", controlplane.AuditLogInput{})
			return err
		}},
		{name: "unbind", want: store.ErrNormalizedDeviceLifecycleRepositoryRequired, call: func() error {
			_, err := service.UnbindDeviceWithAudit(ctx, "unbind-id", "device_1", controlplane.AuditLogInput{})
			return err
		}},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := test.call(); !errors.Is(err, test.want) {
				t.Fatalf("error = %v, want %v", err, test.want)
			}
		})
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized device mutations used StateOperation %d times", repository.runCalls)
	}
}

type normalizedDeviceMutationFallbackRepository struct {
	*store.MemoryStore
	runCalls int
}

func (r *normalizedDeviceMutationFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized device mutation must not use StateOperation")
}

func (*normalizedDeviceMutationFallbackRepository) UsesNormalizedReadSource() bool { return true }
