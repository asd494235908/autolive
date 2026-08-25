package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestTask4ActivationCodeProductMismatchDoesNotConsumeCapacity(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	svc := NewControlPlane(repository)
	ctx := context.Background()
	if err := repository.Run(ctx, func(state *store.State) error {
		state.Users["usr_product01"] = controlplane.UserSummary{ID: "usr_product01", Status: controlplane.UserStatusActive}
		state.ActivationCodes["code_product01"] = store.ActivationCodeRecord{
			ActivationCode: controlplane.ActivationCode{
				ID: "code_product01", Product: controlplane.ProductAutoLive, UserID: "usr_product01",
				Status: controlplane.ActivationCodeStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339),
				MaxDevices: 1,
			},
			PlainCode: "AUTO-CODE-01",
		}
		state.ActivationCodeIndex[secretDigest("AUTO-CODE-01")] = "code_product01"
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	_, err := svc.ActivateDevice(ctx, "activate-product-mismatch", "usr_product01", controlplane.ActivateDeviceInput{
		Device: controlplane.DeviceRegistration{
			Product: controlplane.ProductDouyinDesktop, DeviceID: "dev_product01",
			DeviceName: "desktop", Platform: "windows", AppVersion: "2.0.0",
		},
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("ActivateDevice() error = %v, want forbidden", err)
	}
	if err := repository.Run(ctx, func(state *store.State) error {
		code := state.ActivationCodes["code_product01"]
		if code.ActivationCode.BoundDevices != 0 || code.ActivationCode.Status != controlplane.ActivationCodeStatusActive {
			t.Fatalf("activation code mutated after product rejection: %+v", code.ActivationCode)
		}
		if _, exists := state.Devices["dev_product01"]; exists {
			t.Fatal("device was created after product rejection")
		}
		if _, exists := state.IdempotencyRecords["activate-device:usr_product01:activate-product-mismatch"]; exists {
			t.Fatal("activation idempotency record was written after product rejection")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4HeartbeatProductMismatchDoesNotUpdateDeviceOrIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	ctx := context.Background()
	if err := repository.Run(ctx, func(state *store.State) error {
		state.Users["usr_product02"] = controlplane.UserSummary{ID: "usr_product02", Status: controlplane.UserStatusActive}
		state.Devices["dev_product02"] = controlplane.DeviceSummary{
			ID: "dev_product02", UserID: "usr_product02", Product: controlplane.ProductAutoLive,
			Status: controlplane.DeviceStatusActive, LastSeenAt: now.Add(-time.Hour).Format(time.RFC3339),
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	_, err := NewControlPlane(repository).RecordHeartbeat(ctx, "heartbeat-product-mismatch", "usr_product02", controlplane.HeartbeatInput{
		Product: controlplane.ProductDouyinDesktop, DeviceID: "dev_product02", SentAt: now,
		Status: controlplane.HeartbeatStatus{DiskFreeBytes: 999, PlaybackState: "playing"},
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordHeartbeat() error = %v, want forbidden", err)
	}
	if err := repository.Run(ctx, func(state *store.State) error {
		device := state.Devices["dev_product02"]
		if device.LastSeenAt != now.Add(-time.Hour).Format(time.RFC3339) || device.DiskFreeBytes != 0 || device.PlaybackState != "" {
			t.Fatalf("device mutated after product rejection: %+v", device)
		}
		if _, exists := state.IdempotencyRecords["heartbeat:usr_product02:dev_product02:heartbeat-product-mismatch"]; exists {
			t.Fatal("heartbeat idempotency record was written after product rejection")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4LeaseProductMismatchDoesNotConsumeSlotOrWriteAudit(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	secretStore := store.NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/mpa_product01", "secret-value"); err != nil {
		t.Fatal(err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_product03"] = controlplane.UserSummary{ID: "usr_product03", Status: controlplane.UserStatusActive}
		state.Devices["dev_product03"] = controlplane.DeviceSummary{
			ID: "dev_product03", UserID: "usr_product03", Product: controlplane.ProductDouyinDesktop,
			Status: controlplane.DeviceStatusActive,
		}
		state.ModelPoolAccounts["mpa_product01"] = controlplane.ModelPoolAccountSummary{
			ID: "mpa_product01", Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive,
			SecretConfigured: true, SecretRef: "model-account/mpa_product01", ConcurrencyLimit: 1,
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	_, err := NewControlPlaneWithRepositoryAndSecretStore(repository, nil, secretStore).CreateModelLeaseWithAudit(
		context.Background(), "lease-product-mismatch", "usr_product03", "dev_product03",
		controlplane.CreateModelLeaseInput{Provider: "openai", Model: "gpt", Purpose: "chat"},
		controlplane.AuditLogInput{Product: controlplane.ProductAutoLive, ActorUserID: "usr_product03", Action: "POST /lease", TargetType: "model_lease", Outcome: "success", StatusCode: 200},
	)
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("CreateModelLeaseWithAudit() error = %v, want forbidden", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if len(state.ModelLeases) != 0 || len(state.IdempotencyRecords) != 0 || len(state.AuditLogs) != 0 {
			t.Fatalf("lease rejection changed state: leases=%d idempotency=%d audits=%d", len(state.ModelLeases), len(state.IdempotencyRecords), len(state.AuditLogs))
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4UsageProductMismatchDoesNotWriteUsageOrSuccessAudit(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_product04"] = controlplane.UserSummary{ID: "usr_product04", Status: controlplane.UserStatusActive}
		state.Devices["dev_product04"] = controlplane.DeviceSummary{ID: "dev_product04", UserID: "usr_product04", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelLeases["lease_product04"] = controlplane.ModelLease{ID: "lease_product04", Product: controlplane.ProductDouyinDesktop, UserID: "usr_product04", DeviceID: "dev_product04", AccountID: "account_product04", Provider: "openai", Model: "gpt", Status: controlplane.ModelLeaseStatusActive}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	input := controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call-product04", LeaseID: "lease_product04", Provider: "openai", Model: "gpt",
		Status: "succeeded", UsageSource: "client_reported", TotalTokens: 1,
	}
	_, err := NewControlPlane(repository).RecordDirectLLMCallWithAudit(
		context.Background(), "usage-product-mismatch", "usr_product04", "dev_product04", "request-product04", input,
		controlplane.AuditLogInput{Product: controlplane.ProductAutoLive, ActorUserID: "usr_product04", Action: "POST /usage", TargetType: "model_usage", Outcome: "success", StatusCode: 200},
	)
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordDirectLLMCallWithAudit() error = %v, want forbidden", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if len(state.ModelUsageRecords) != 0 || len(state.IdempotencyRecords) != 0 || len(state.AuditLogs) != 0 {
			t.Fatalf("usage rejection changed state: usage=%d idempotency=%d audits=%d", len(state.ModelUsageRecords), len(state.IdempotencyRecords), len(state.AuditLogs))
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4AuditTargetProductMismatchIsRejected(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices["dev_product05"] = controlplane.DeviceSummary{ID: "dev_product05", UserID: "usr_product05", Product: controlplane.ProductAutoLive, Status: controlplane.DeviceStatusActive}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	err := NewControlPlane(repository).RecordAudit(context.Background(), controlplane.AuditLogInput{
		Product: controlplane.ProductDouyinDesktop, DeviceID: "dev_product05", Action: "POST /device", TargetType: "device", TargetID: "dev_product05", Outcome: "success", StatusCode: 200,
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordAudit() error = %v, want forbidden", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if len(state.AuditLogs) != 0 {
			t.Fatalf("audit target rejection wrote %d audit logs", len(state.AuditLogs))
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4AuditAcceptsLegacyAutoliveDeviceWithoutProduct(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices["dev_legacy"] = controlplane.DeviceSummary{ID: "dev_legacy", UserID: "usr_legacy", Status: controlplane.DeviceStatusActive}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	err := NewControlPlane(repository).RecordAuditForProduct(context.Background(), controlplane.ProductAutoLive, controlplane.AuditLogInput{
		DeviceID: "dev_legacy", Action: "POST /api/v1/client/heartbeat", TargetType: "device", TargetID: "dev_legacy", Outcome: "failure", StatusCode: 400,
	})
	if err != nil {
		t.Fatalf("RecordAuditForProduct() error = %v", err)
	}
}

func TestTask4MemoryAuditMissingDeviceMatchesPostgresOutcomeSemantics(t *testing.T) {
	tests := []struct {
		name    string
		outcome string
		wantErr error
	}{
		{name: "success rejects", outcome: "success", wantErr: controlplane.ErrForbidden},
		{name: "unknown rejects", outcome: "unknown", wantErr: controlplane.ErrForbidden},
		{name: "failure is recorded", outcome: "failure"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			repository := store.NewMemoryStore(time.Now)
			err := NewControlPlane(repository).RecordAuditForProduct(context.Background(), controlplane.ProductAutoLive, controlplane.AuditLogInput{
				DeviceID: "missing-device", Action: "POST /device", TargetType: "audit", Outcome: tt.outcome, StatusCode: 200,
			})
			if !errors.Is(err, tt.wantErr) {
				t.Fatalf("RecordAuditForProduct() error = %v, want %v", err, tt.wantErr)
			}
			if err := repository.Run(context.Background(), func(state *store.State) error {
				wantAudits := 0
				if tt.wantErr == nil {
					wantAudits = 1
				}
				if len(state.AuditLogs) != wantAudits {
					t.Fatalf("audit log count = %d, want %d", len(state.AuditLogs), wantAudits)
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestTask4ModelAccountAuditTargetIsStrictlyProductScoped(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelPoolAccounts["account_douyin"] = controlplane.ModelPoolAccountSummary{
			ID: "account_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive,
		}
		state.ModelPoolAccounts["account_null"] = controlplane.ModelPoolAccountSummary{
			ID: "account_null", Product: "", Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive,
		}
		state.ModelPoolAccounts["account_invalid"] = controlplane.ModelPoolAccountSummary{
			ID: "account_invalid", Product: controlplane.ProductCode("invalid_product"), Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive,
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name   string
		target string
		want   error
	}{
		{name: "cross product", target: "account_douyin", want: controlplane.ErrForbidden},
		{name: "null product", target: "account_null", want: controlplane.ErrForbidden},
		{name: "invalid product", target: "account_invalid", want: controlplane.ErrForbidden},
		{name: "missing success target", target: "account_missing", want: controlplane.ErrForbidden},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := NewControlPlane(repository).RecordAuditForProduct(context.Background(), controlplane.ProductAutoLive, controlplane.AuditLogInput{
				Action: "POST /api/v1/admin/model-pool", TargetType: "model_account", TargetID: tt.target, Outcome: "success", StatusCode: 200,
			})
			if !errors.Is(err, tt.want) {
				t.Fatalf("RecordAuditForProduct() error = %v, want %v", err, tt.want)
			}
		})
	}

	if err := NewControlPlane(repository).RecordAuditForProduct(context.Background(), controlplane.ProductAutoLive, controlplane.AuditLogInput{
		Action: "POST /api/v1/admin/model-pool", TargetType: "model_account", TargetID: "account_missing", Outcome: "failure", StatusCode: 404,
	}); err != nil {
		t.Fatalf("RecordAuditForProduct(failure) error = %v, want nil", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if len(state.AuditLogs) != 1 {
			t.Fatalf("audit log count = %d, want 1", len(state.AuditLogs))
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4ProductFilteredPagesDoNotMixProducts(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelLeases["lease_page_auto"] = controlplane.ModelLease{ID: "lease_page_auto", Product: controlplane.ProductAutoLive, Provider: "openai", Model: "gpt", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: time.Now().Add(time.Hour).Format(time.RFC3339)}
		state.ModelLeases["lease_page_douyin"] = controlplane.ModelLease{ID: "lease_page_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai", Model: "gpt", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: time.Now().Add(time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_page_auto"] = controlplane.ModelUsageRecord{ID: "usage_page_auto", Product: controlplane.ProductAutoLive, LeaseID: "lease_page_auto", Provider: "openai", Model: "gpt", CreatedAt: time.Now().Format(time.RFC3339)}
		state.ModelUsageRecords["usage_page_douyin"] = controlplane.ModelUsageRecord{ID: "usage_page_douyin", Product: controlplane.ProductDouyinDesktop, LeaseID: "lease_page_douyin", Provider: "openai", Model: "gpt", CreatedAt: time.Now().Format(time.RFC3339)}
		state.AuditLogs["audit_page_auto"] = controlplane.AuditLog{ID: "audit_page_auto", Product: controlplane.ProductAutoLive, Action: "GET /auto", TargetType: "audit", Outcome: "success", CreatedAt: time.Now().Format(time.RFC3339)}
		state.AuditLogs["audit_page_douyin"] = controlplane.AuditLog{ID: "audit_page_douyin", Product: controlplane.ProductDouyinDesktop, Action: "GET /douyin", TargetType: "audit", Outcome: "success", CreatedAt: time.Now().Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	leasePage, err := repository.ListModelLeasesPageWithOptions(context.Background(), store.ModelLeasePageOptions{Product: controlplane.ProductDouyinDesktop, Offset: 0, Limit: 20})
	if err != nil || len(leasePage.Items) != 1 || leasePage.Items[0].ID != "lease_page_douyin" {
		t.Fatalf("product lease page = %+v, error = %v", leasePage, err)
	}
	usagePage, err := repository.ListModelUsagePageWithOptions(context.Background(), store.ModelUsagePageOptions{Product: controlplane.ProductDouyinDesktop, Offset: 0, Limit: 20})
	if err != nil || len(usagePage.Items) != 1 || usagePage.Items[0].ID != "usage_page_douyin" {
		t.Fatalf("product usage page = %+v, error = %v", usagePage, err)
	}
	auditPage, err := repository.ListAuditLogsPageWithOptions(context.Background(), store.AuditLogPageOptions{Product: controlplane.ProductDouyinDesktop, Offset: 0, Limit: 20})
	if err != nil || len(auditPage.Items) != 1 || auditPage.Items[0].ID != "audit_page_douyin" {
		t.Fatalf("product audit page = %+v, error = %v", auditPage, err)
	}
}

func TestTask4ProductBoundLeaseRejectsAuditProductSpoof(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	secretStore := store.NewMemorySecretStore()
	if err := secretStore.Put(context.Background(), "model-account/douyin", "secret-value"); err != nil {
		t.Fatal(err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_bound01"] = controlplane.UserSummary{ID: "usr_bound01", Status: controlplane.UserStatusActive}
		state.Devices["dev_bound01"] = controlplane.DeviceSummary{ID: "dev_bound01", UserID: "usr_bound01", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelPoolAccounts["mpa_douyin"] = controlplane.ModelPoolAccountSummary{
			ID: "mpa_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai", Model: "gpt",
			Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/douyin", ConcurrencyLimit: 1,
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	_, err := NewControlPlaneWithRepositoryAndSecretStore(repository, nil, secretStore).CreateModelLeaseForProduct(
		context.Background(), controlplane.ProductDouyinDesktop, "lease-bound01", "usr_bound01", "dev_bound01",
		controlplane.CreateModelLeaseInput{Provider: "openai", Model: "gpt", Purpose: "chat"},
		controlplane.AuditLogInput{Product: controlplane.ProductAutoLive, Action: "POST /lease", TargetType: "model_lease", Outcome: "success", StatusCode: 200},
	)
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("CreateModelLeaseForProduct() error = %v, want forbidden", err)
	}
}

func TestTask4ProductBoundModelSelectionAndSweepStayInProduct(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	secretStore := store.NewMemorySecretStore()
	for _, ref := range []string{"model-account/auto", "model-account/douyin"} {
		if err := secretStore.Put(context.Background(), ref, "secret-value"); err != nil {
			t.Fatal(err)
		}
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_bound02"] = controlplane.UserSummary{ID: "usr_bound02", Status: controlplane.UserStatusActive}
		state.Devices["dev_bound02"] = controlplane.DeviceSummary{ID: "dev_bound02", UserID: "usr_bound02", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelPoolAccounts["mpa_auto"] = controlplane.ModelPoolAccountSummary{ID: "mpa_auto", Product: controlplane.ProductAutoLive, Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/auto", ConcurrencyLimit: 1}
		state.ModelPoolAccounts["mpa_douyin"] = controlplane.ModelPoolAccountSummary{ID: "mpa_douyin", Product: controlplane.ProductDouyinDesktop, Provider: "openai", Model: "gpt", Status: controlplane.ModelAccountStatusActive, SecretConfigured: true, SecretRef: "model-account/douyin", ConcurrencyLimit: 1}
		state.ModelLeases["lease_auto_expired"] = controlplane.ModelLease{ID: "lease_auto_expired", Product: controlplane.ProductAutoLive, AccountID: "mpa_auto", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(-time.Minute).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	lease, err := NewControlPlaneWithRepositoryAndSecretStore(repository, nil, secretStore).CreateModelLeaseForProduct(
		context.Background(), controlplane.ProductDouyinDesktop, "lease-bound02", "usr_bound02", "dev_bound02",
		controlplane.CreateModelLeaseInput{Provider: "openai", Model: "gpt", Purpose: "chat"}, controlplane.AuditLogInput{},
	)
	if err != nil {
		t.Fatalf("CreateModelLeaseForProduct() error = %v", err)
	}
	if lease.AccountID != "mpa_douyin" || lease.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("lease = %+v, want douyin account/product", lease)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if state.ModelLeases["lease_auto_expired"].Status != controlplane.ModelLeaseStatusActive {
			t.Fatalf("autolive lease was swept by douyin operation: %+v", state.ModelLeases["lease_auto_expired"])
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4ProductBoundDeviceLifecycleKeepsOtherProductLeases(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices["device_auto_lifecycle"] = controlplane.DeviceSummary{ID: "device_auto_lifecycle", UserID: "user_lifecycle", Product: controlplane.ProductAutoLive, Status: controlplane.DeviceStatusActive}
		state.Devices["device_douyin_lifecycle"] = controlplane.DeviceSummary{ID: "device_douyin_lifecycle", UserID: "user_lifecycle", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelLeases["lease_auto_lifecycle"] = controlplane.ModelLease{ID: "lease_auto_lifecycle", Product: controlplane.ProductAutoLive, DeviceID: "device_auto_lifecycle", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelLeases["lease_douyin_lifecycle"] = controlplane.ModelLease{ID: "lease_douyin_lifecycle", Product: controlplane.ProductDouyinDesktop, DeviceID: "device_douyin_lifecycle", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	svc := NewControlPlane(repository)
	if _, err := svc.DisableDeviceForProduct(context.Background(), controlplane.ProductDouyinDesktop, "disable-douyin-lifecycle", "device_douyin_lifecycle", controlplane.AuditLogInput{}); err != nil {
		t.Fatalf("DisableDeviceForProduct() error = %v", err)
	}
	if _, err := svc.UnbindDeviceForProduct(context.Background(), controlplane.ProductDouyinDesktop, "unbind-douyin-lifecycle", "device_douyin_lifecycle", controlplane.AuditLogInput{}); err != nil {
		t.Fatalf("UnbindDeviceForProduct() error = %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if state.ModelLeases["lease_auto_lifecycle"].Status != controlplane.ModelLeaseStatusActive {
			return errors.New("autolive lease was changed by douyin lifecycle")
		}
		if state.ModelLeases["lease_douyin_lifecycle"].Status != controlplane.ModelLeaseStatusReleased {
			return errors.New("douyin lease was not released")
		}
		if state.Devices["device_auto_lifecycle"].Status != controlplane.DeviceStatusActive {
			return errors.New("autolive device was changed by douyin lifecycle")
		}
		if state.Devices["device_douyin_lifecycle"].Status != controlplane.DeviceStatusPendingActivation {
			return errors.New("douyin device was not unbound")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4ProductBoundDeviceLifecycleDoesNotReleaseOtherProductLease(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Devices["dev_bound03"] = controlplane.DeviceSummary{ID: "dev_bound03", Product: controlplane.ProductDouyinDesktop, Status: controlplane.DeviceStatusActive}
		state.ModelLeases["lease_auto_bound03"] = controlplane.ModelLease{ID: "lease_auto_bound03", Product: controlplane.ProductAutoLive, DeviceID: "dev_bound03", Status: controlplane.ModelLeaseStatusActive}
		state.ModelLeases["lease_douyin_bound03"] = controlplane.ModelLease{ID: "lease_douyin_bound03", Product: controlplane.ProductDouyinDesktop, DeviceID: "dev_bound03", Status: controlplane.ModelLeaseStatusActive}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	if _, err := NewControlPlane(repository).DisableDeviceForProduct(context.Background(), controlplane.ProductDouyinDesktop, "disable-bound03", "dev_bound03", controlplane.AuditLogInput{}); err != nil {
		t.Fatalf("DisableDeviceForProduct() error = %v", err)
	}
	if err := repository.Run(context.Background(), func(state *store.State) error {
		if state.ModelLeases["lease_auto_bound03"].Status != controlplane.ModelLeaseStatusActive {
			t.Fatalf("autolive lease was released by douyin lifecycle: %+v", state.ModelLeases["lease_auto_bound03"])
		}
		if state.ModelLeases["lease_douyin_bound03"].Status != controlplane.ModelLeaseStatusReleased {
			t.Fatalf("douyin lease was not released: %+v", state.ModelLeases["lease_douyin_bound03"])
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestTask4ProductBoundAuditValidatesExplicitProduct(t *testing.T) {
	repository := store.NewMemoryStore(time.Now)
	svc := NewControlPlane(repository)
	err := svc.RecordAuditForProduct(context.Background(), controlplane.ProductAutoLive, controlplane.AuditLogInput{
		Product: controlplane.ProductDouyinDesktop, Action: "POST /audit", TargetType: "audit", Outcome: "success", StatusCode: 200,
	})
	if !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordAuditForProduct() error = %v, want forbidden", err)
	}
}
