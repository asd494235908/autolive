package service_test

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

func TestModelPoolConnectivityTestUsesOpenAICompatibleModelsEndpoint(t *testing.T) {
	var authorization string
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/models" {
			t.Fatalf("path = %q, want /v1/models", r.URL.Path)
		}
		authorization = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[]}`))
	}))
	defer provider.Close()

	svc := service.NewControlPlaneWithHTTPClient(store.NewMemoryStore(time.Now), provider.Client())
	created, err := svc.CreateModelPoolAccount(context.Background(), "connectivity-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", BaseURL: provider.URL + "/v1", APIKey: "sk-test-secret", ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	result, err := svc.TestModelPoolAccount(context.Background(), "connectivity-test", created.ID, controlplane.TestModelPoolAccountInput{TimeoutSeconds: 5})
	if err != nil {
		t.Fatalf("TestModelPoolAccount() error = %v", err)
	}
	if result.Status != "succeeded" || result.HTTPStatus != http.StatusOK || result.Model != "rewrite-model" {
		t.Fatalf("connectivity result = %+v", result)
	}
	if authorization != "Bearer sk-test-secret" {
		t.Fatalf("authorization = %q, want provider bearer header", authorization)
	}
	if result.TestedAt == "" {
		t.Fatal("connectivity result TestedAt is empty")
	}
	items, err := svc.ListModelPoolAccounts(context.Background())
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after connectivity test error = %v", err)
	}
	if len(items) != 1 || items[0].LastTestStatus != "succeeded" || items[0].LastTestedAt != result.TestedAt {
		t.Fatalf("account health summary = %+v", items)
	}
}

func TestModelPoolSummaryTracksLeasesAndDailyUsageAndStopsAtQuota(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	svc.EnsureLocalAdmin("admin")
	ctx := context.Background()
	code, err := svc.CreateActivationCode(ctx, "quota-code-1", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(ctx, "quota-device-1", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefString(t, code.PlainCode),
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_quota01", DeviceName: "MacBook", Platform: "macOS", AppVersion: "0.1.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	account, err := svc.CreateModelPoolAccount(ctx, "quota-account-1", controlplane.CreateModelPoolAccountInput{
		Provider: "openai-compatible", Model: "rewrite-model", APIKey: "sk-test-secret", DailyLimit: 10, ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	lease, err := svc.CreateModelLease(ctx, "quota-lease-1", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: account.Provider, Model: account.Model, Purpose: "realtime_script"})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	_, err = svc.RecordDirectLLMCall(ctx, "quota-call-1", "usr_local_admin", device.ID, "req_quota01", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_quota01", LeaseID: lease.ID, Provider: account.Provider, Model: account.Model, InputTokens: 4, OutputTokens: 3, Status: "succeeded", UsageSource: "client_reported",
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() first error = %v", err)
	}
	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if items[0].ActiveLeases != 1 || items[0].DailyUsedTokens != 7 || items[0].Status != controlplane.ModelAccountStatusActive {
		t.Fatalf("account usage summary after first call = %+v", items[0])
	}
	_, err = svc.RecordDirectLLMCall(ctx, "quota-call-2", "usr_local_admin", device.ID, "req_quota02", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_quota02", LeaseID: lease.ID, Provider: account.Provider, Model: account.Model, InputTokens: 2, OutputTokens: 1, Status: "succeeded", UsageSource: "client_reported",
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() second error = %v", err)
	}
	items, err = svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() after quota error = %v", err)
	}
	if items[0].DailyUsedTokens != 10 || items[0].Status != controlplane.ModelAccountStatusExhausted {
		t.Fatalf("account usage summary at quota = %+v", items[0])
	}
	if _, err := svc.CreateModelLease(ctx, "quota-lease-2", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: account.Provider, Model: account.Model, Purpose: "realtime_script"}); !controlplane.IsErrorCode(err, "MODEL_POOL_UNAVAILABLE") {
		t.Fatalf("CreateModelLease() after quota error = %v, want MODEL_POOL_UNAVAILABLE", err)
	}
}

func TestRecordDirectLLMCallValidatesLeaseAndIsIdempotent(t *testing.T) {
	now := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	svc.EnsureLocalAdmin("admin")
	code, err := svc.CreateActivationCode(context.Background(), "direct-call-code", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(context.Background(), "direct-call-device", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefString(t, code.PlainCode),
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_direct01", DeviceName: "MacBook", Platform: "macOS", AppVersion: "0.1.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}
	if _, err := svc.CreateModelPoolAccount(context.Background(), "direct-call-account", controlplane.CreateModelPoolAccountInput{Provider: "openai-compatible", Model: "rewrite-model", APIKey: "sk-test-secret", ConcurrencyLimit: 1}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	lease, err := svc.CreateModelLease(context.Background(), "direct-call-lease", "usr_local_admin", device.ID, controlplane.CreateModelLeaseInput{Provider: "openai-compatible", Model: "rewrite-model", Purpose: "realtime_script"})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	input := controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_direct01", LeaseID: lease.ID, Provider: lease.Provider, Model: lease.Model, InputTokens: 3, OutputTokens: 5, LatencyMS: 40, Status: "succeeded", UsageSource: "client_reported"}
	record, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-record", "usr_local_admin", device.ID, "req_test", input)
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() error = %v", err)
	}
	if record.TotalTokens != 8 || record.RequestID != "req_test" || record.UsageSource != "client_reported" {
		t.Fatalf("record = %+v", record)
	}
	repeated, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-record", "usr_local_admin", device.ID, "req_test-2", input)
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() repeated error = %v", err)
	}
	if repeated.ID != record.ID {
		t.Fatalf("repeated id = %q, want %q", repeated.ID, record.ID)
	}
	if _, err := svc.RecordDirectLLMCall(context.Background(), "direct-call-invalid", "usr_local_admin", device.ID, "req_invalid", controlplane.CreateDirectLLMCallRecordInput{ClientCallID: "call_direct02", LeaseID: lease.ID, Provider: "other", Model: lease.Model, Status: "succeeded", UsageSource: "client_reported"}); err == nil {
		t.Fatal("RecordDirectLLMCall() with provider mismatch unexpectedly succeeded")
	}
}

func TestCreateAndListModelPoolRedactsSecretAndSupportsIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	svc.EnsureLocalAdmin("admin")
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	if !created.SecretConfigured {
		t.Fatalf("SecretConfigured = %v, want true", created.SecretConfigured)
	}

	repeated, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() duplicate error = %v", err)
	}
	if repeated.ID != created.ID {
		t.Fatalf("duplicate id = %q, want %q", repeated.ID, created.ID)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "model-account-1", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model-v2",
		APIKey:           "sk-live-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	}); err == nil {
		t.Fatal("CreateModelPoolAccount() with conflicting idempotency unexpectedly succeeded")
	}

	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("len(items) = %d, want 1", len(items))
	}
	if items[0].ID != created.ID || !items[0].SecretConfigured {
		t.Fatalf("unexpected item = %+v", items[0])
	}
}

func TestDisableModelPoolAccountIsIdempotentAndBlocksMissingAccounts(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "disable-account-create", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	disabled, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", created.ID)
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() error = %v", err)
	}
	if disabled.Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("disabled status = %q, want %q", disabled.Status, controlplane.ModelAccountStatusDisabled)
	}

	repeated, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", created.ID)
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() repeated error = %v", err)
	}
	if repeated.ID != created.ID || repeated.Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("repeated result = %+v, want disabled account %q", repeated, created.ID)
	}

	if _, err := svc.DisableModelPoolAccount(ctx, "disable-account-1", "another-account"); !controlplane.IsErrorCode(err, "IDEMPOTENCY_CONFLICT") {
		t.Fatalf("conflicting idempotency error = %v, want IDEMPOTENCY_CONFLICT", err)
	}
	if _, err := svc.DisableModelPoolAccount(ctx, "disable-account-missing", "missing-account"); !controlplane.IsErrorCode(err, "MODEL_POOL_ACCOUNT_NOT_FOUND") {
		t.Fatalf("missing account error = %v, want MODEL_POOL_ACCOUNT_NOT_FOUND", err)
	}
}

func TestUpdateModelPoolAccountSupportsCooldownAndIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()

	created, err := svc.CreateModelPoolAccount(ctx, "update-account-create", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		BaseURL:          "https://api.example.com/v1",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	priority := 20
	dailyLimit := 2000
	status := controlplane.ModelAccountStatusCooldown
	updated, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{
		Priority:   &priority,
		DailyLimit: &dailyLimit,
		Status:     &status,
	})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() error = %v", err)
	}
	if updated.Priority != priority || updated.DailyLimit != dailyLimit || updated.Status != status {
		t.Fatalf("updated account = %+v", updated)
	}

	repeated, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{
		Priority:   &priority,
		DailyLimit: &dailyLimit,
		Status:     &status,
	})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() repeated error = %v", err)
	}
	if repeated.ID != created.ID || repeated.Status != status {
		t.Fatalf("repeated account = %+v", repeated)
	}

	otherPriority := 30
	if _, err := svc.UpdateModelPoolAccount(ctx, "update-account-1", created.ID, controlplane.UpdateModelPoolAccountInput{Priority: &otherPriority}); !controlplane.IsErrorCode(err, "IDEMPOTENCY_CONFLICT") {
		t.Fatalf("conflicting update error = %v, want IDEMPOTENCY_CONFLICT", err)
	}
	if _, err := svc.UpdateModelPoolAccount(ctx, "update-account-missing", "missing-account", controlplane.UpdateModelPoolAccountInput{Priority: &priority}); !controlplane.IsErrorCode(err, "MODEL_POOL_ACCOUNT_NOT_FOUND") {
		t.Fatalf("missing account error = %v, want MODEL_POOL_ACCOUNT_NOT_FOUND", err)
	}
}

func TestModelLeaseLifecycleChecksOwnershipAndReleaseIdempotency(t *testing.T) {
	now := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	svc.EnsureLocalAdmin("admin")
	ctx := context.Background()

	if _, err := svc.CreateUser(ctx, "lease-user-1", controlplane.CreateUserInput{
		Username: "lease-user",
		Password: "correct-password",
		Role:     controlplane.RoleUser,
	}); err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if _, err := svc.CreateUser(ctx, "lease-user-2", controlplane.CreateUserInput{
		Username: "other-user",
		Password: "correct-password",
		Role:     controlplane.RoleUser,
	}); err != nil {
		t.Fatalf("CreateUser() second user error = %v", err)
	}

	code, err := svc.CreateActivationCode(ctx, "lease-code", controlplane.CreateActivationCodeInput{
		ExpiresAt:  now.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDevice(ctx, "lease-device", "usr_00000001", controlplane.ActivateDeviceInput{
		ActivationCode: derefString(t, code.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_lease001",
			DeviceName: "MacBook",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() error = %v", err)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "lease-account", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-model-secret",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}

	lease, err := svc.CreateModelLease(ctx, "lease-create-1", "usr_00000001", device.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	if lease.ProxyMode != controlplane.ModelLeaseProxyModeDirectLease {
		t.Fatalf("ProxyMode = %q, want %q", lease.ProxyMode, controlplane.ModelLeaseProxyModeDirectLease)
	}

	leaseAgain, err := svc.CreateModelLease(ctx, "lease-create-1", "usr_00000001", device.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() duplicate error = %v", err)
	}
	if leaseAgain.ID != lease.ID {
		t.Fatalf("duplicate lease id = %q, want %q", leaseAgain.ID, lease.ID)
	}

	renewed, err := svc.RenewModelLease(ctx, "lease-renew-1", "usr_00000001", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 120,
	})
	if err != nil {
		t.Fatalf("RenewModelLease() error = %v", err)
	}
	if renewed.ID != lease.ID || renewed.ExpiresAt <= lease.ExpiresAt {
		t.Fatalf("unexpected renewed lease = %+v", renewed)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-foreign", "usr_00000002", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 60,
	}); err == nil {
		t.Fatal("RenewModelLease() with foreign user unexpectedly succeeded")
	}

	release, err := svc.ReleaseModelLease(ctx, "lease-release-1", "usr_00000001", device.ID, lease.ID, controlplane.ReleaseModelLeaseInput{
		Reason: "done",
	})
	if err != nil {
		t.Fatalf("ReleaseModelLease() error = %v", err)
	}
	if !release.Released {
		t.Fatalf("Released = %v, want true", release.Released)
	}

	releaseAgain, err := svc.ReleaseModelLease(ctx, "lease-release-2", "usr_00000001", device.ID, lease.ID, controlplane.ReleaseModelLeaseInput{
		Reason: "retry",
	})
	if err != nil {
		t.Fatalf("ReleaseModelLease() repeat error = %v", err)
	}
	if !releaseAgain.Released {
		t.Fatalf("repeat Released = %v, want true", releaseAgain.Released)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-after-release", "usr_00000001", device.ID, lease.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 60,
	}); err == nil {
		t.Fatal("RenewModelLease() after release unexpectedly succeeded")
	}
}

func TestModelLeaseConcurrencyDisabledAccountAndLazyExpiryRecycle(t *testing.T) {
	startNow := time.Date(2026, 8, 12, 10, 0, 0, 0, time.UTC)
	currentNow := startNow
	svc := service.NewControlPlane(store.NewMemoryStore(func() time.Time { return currentNow }))
	svc.EnsureLocalAdmin("admin")
	ctx := context.Background()

	code1, err := svc.CreateActivationCode(ctx, "lease-code-1", controlplane.CreateActivationCodeInput{
		ExpiresAt:  startNow.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device1, err := svc.ActivateDevice(ctx, "lease-device-1", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefString(t, code1.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_expiry001",
			DeviceName: "MacBook",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() first error = %v", err)
	}

	code2, err := svc.CreateActivationCode(ctx, "lease-code-2", controlplane.CreateActivationCodeInput{
		ExpiresAt:  startNow.Add(time.Hour),
		MaxDevices: 1,
	})
	if err != nil {
		t.Fatalf("CreateActivationCode() second error = %v", err)
	}
	device2, err := svc.ActivateDevice(ctx, "lease-device-2", "usr_local_admin", controlplane.ActivateDeviceInput{
		ActivationCode: derefString(t, code2.PlainCode),
		Device: controlplane.DeviceRegistration{
			DeviceID:   "dev_expiry002",
			DeviceName: "MacBook 2",
			Platform:   "macOS",
			AppVersion: "0.1.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDevice() second error = %v", err)
	}

	if _, err := svc.CreateModelPoolAccount(ctx, "lease-account-disabled", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-disabled",
		Priority:         100,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
		Status:           controlplane.ModelAccountStatusDisabled,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() disabled error = %v", err)
	}
	if _, err := svc.CreateModelPoolAccount(ctx, "lease-account-active", controlplane.CreateModelPoolAccountInput{
		Provider:         "openai-compatible",
		Model:            "rewrite-model",
		APIKey:           "sk-active",
		Priority:         10,
		DailyLimit:       1000,
		ConcurrencyLimit: 1,
	}); err != nil {
		t.Fatalf("CreateModelPoolAccount() active error = %v", err)
	}
	if _, err := svc.CreateModelLease(ctx, "lease-create-unbound", "usr_local_admin", "", controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	}); !controlplane.IsErrorCode(err, "DEVICE_BINDING_REQUIRED") {
		t.Fatalf("CreateModelLease() without bound device error = %v, want DEVICE_BINDING_REQUIRED", err)
	}

	first, err := svc.CreateModelLease(ctx, "lease-create-first", "usr_local_admin", device1.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() first error = %v", err)
	}

	if _, err := svc.CreateModelLease(ctx, "lease-create-second", "usr_local_admin", device2.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	}); err == nil {
		t.Fatal("CreateModelLease() under concurrency limit unexpectedly succeeded")
	}

	currentNow = startNow.Add(31 * time.Second)
	second, err := svc.CreateModelLease(ctx, "lease-create-third", "usr_local_admin", device2.ID, controlplane.CreateModelLeaseInput{
		Provider:           "openai-compatible",
		Model:              "rewrite-model",
		Purpose:            "realtime_script",
		MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("CreateModelLease() after expiry error = %v", err)
	}
	if second.ID == first.ID {
		t.Fatalf("CreateModelLease() recycled same lease id = %q", second.ID)
	}

	if _, err := svc.RenewModelLease(ctx, "lease-renew-expired", "usr_local_admin", device1.ID, first.ID, controlplane.RenewModelLeaseInput{
		ExtendSeconds: 30,
	}); err == nil {
		t.Fatal("RenewModelLease() expired lease unexpectedly succeeded")
	}

	items, err := svc.ListModelPoolAccounts(ctx)
	if err != nil {
		t.Fatalf("ListModelPoolAccounts() error = %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("len(items) = %d, want 2", len(items))
	}
	if items[0].Status != controlplane.ModelAccountStatusDisabled && items[1].Status != controlplane.ModelAccountStatusDisabled {
		t.Fatalf("disabled account summary missing: %+v", items)
	}
}
