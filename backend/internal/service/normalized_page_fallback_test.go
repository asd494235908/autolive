package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

// A normalized repository without a bounded reader must fail closed rather
// than materialize the compatibility State through Repository.Run.
func TestNormalizedListPathsFailClosedWithoutPageReaders(t *testing.T) {
	repository := &normalizedPageFallbackRepository{}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	tests := []struct {
		name string
		want error
		call func() error
	}{
		{name: "users", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, err := service.ListUsers(ctx)
			return err
		}},
		{name: "users_page", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, _, err := service.ListUsersPage(ctx, 1, 20)
			return err
		}},
		{name: "devices", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, err := service.ListDevices(ctx)
			return err
		}},
		{name: "devices_page", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, _, err := service.ListDevicesPage(ctx, 1, 20)
			return err
		}},
		{name: "user_devices", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, err := service.ListDevicesForUser(ctx, "user-1")
			return err
		}},
		{name: "user_devices_page", want: store.ErrNormalizedUserPageReaderRequired, call: func() error {
			_, _, err := service.ListDevicesForUserPage(ctx, "user-1", 1, 20)
			return err
		}},
		{name: "activation_codes", want: store.ErrNormalizedActivationPageReaderRequired, call: func() error {
			_, err := service.ListActivationCodes(ctx)
			return err
		}},
		{name: "activation_codes_page", want: store.ErrNormalizedActivationPageReaderRequired, call: func() error {
			_, _, err := service.ListActivationCodesPage(ctx, 1, 20)
			return err
		}},
		{name: "audit_logs", want: store.ErrNormalizedAuditPageReaderRequired, call: func() error {
			_, err := service.ListAuditLogs(ctx)
			return err
		}},
		{name: "audit_logs_page", want: store.ErrNormalizedAuditPageReaderRequired, call: func() error {
			_, _, err := service.ListAuditLogsPageWithOptions(ctx, 1, 20, AuditLogListOptions{})
			return err
		}},
		{name: "model_usage", want: store.ErrNormalizedModelUsagePageReaderRequired, call: func() error {
			_, err := service.ListModelUsage(ctx)
			return err
		}},
		{name: "model_usage_page", want: store.ErrNormalizedModelUsagePageReaderRequired, call: func() error {
			_, _, err := service.ListModelUsagePageWithOptions(ctx, 1, 20, ModelUsageListOptions{})
			return err
		}},
		{name: "model_leases_page", want: store.ErrNormalizedModelLeasePageReaderRequired, call: func() error {
			_, _, err := service.ListModelLeasesPageWithOptions(ctx, 1, 20, ModelLeaseListOptions{})
			return err
		}},
		{name: "model_pool", want: store.ErrNormalizedModelPoolPageReaderRequired, call: func() error {
			_, err := service.ListModelPoolAccounts(ctx)
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
		t.Fatalf("normalized list paths used StateOperation %d times", repository.runCalls)
	}
}

type normalizedPageFallbackRepository struct {
	runCalls int
}

func (r *normalizedPageFallbackRepository) Now() time.Time {
	return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
}

func (r *normalizedPageFallbackRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized list must not use StateOperation")
}

func (*normalizedPageFallbackRepository) UsesNormalizedReadSource() bool { return true }
