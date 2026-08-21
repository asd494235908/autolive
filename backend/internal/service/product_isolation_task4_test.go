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
				ID: "code_product01", Product: controlplane.ProductAutoLive,
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
		ActivationCode: "AUTO-CODE-01",
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
