package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type transactionalMemoryRepository struct {
	*store.MemoryStore
	bindCalls       int
	accessTokenHash string
	userID          string
	deviceID        string
}

type normalizedDeviceActivatorStub struct {
	*store.MemoryStore
	record store.DeviceActivationRecord
}

type normalizedHeartbeatRecorderStub struct {
	*store.MemoryStore
	record store.DeviceHeartbeatRecord
}

type normalizedDeviceLifecycleStub struct {
	*store.MemoryStore
	disableRecord store.DeviceMutationRecord
	unbindRecord  store.DeviceMutationRecord
}

type normalizedModelLeaseStub struct {
	*store.MemoryStore
	record        store.ModelLeaseCreateRecord
	renewRecord   store.ModelLeaseRenewRecord
	releaseRecord store.ModelLeaseReleaseRecord
	reclaimRecord store.ModelLeaseReclaimRecord
}

type normalizedModelLeaseCreatorStub struct {
	*store.MemoryStore
	record store.ModelLeaseCreateRecord
}

type normalizedModelUsageStub struct {
	*store.MemoryStore
	record store.ModelUsageWriteRecord
}

type normalizedModelPoolStub struct {
	*store.MemoryStore
	disableRecord store.ModelPoolAccountMutationRecord
	updateRecord  store.ModelPoolAccountUpdateRecord
}

type normalizedModelPoolCreatorStub struct {
	*store.MemoryStore
	record store.ModelPoolAccountCreateRecord
}

type normalizedModelPoolTestStub struct {
	*store.MemoryStore
	prepareRecord store.ModelPoolTestPrepareRecord
	record        store.ModelPoolTestRecord
	preparation   store.ModelPoolTestPreparation
}

type normalizedAuditStub struct {
	*store.MemoryStore
	input controlplane.AuditLogInput
}

func (r *normalizedDeviceLifecycleStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedDeviceLifecycleStub) DisableDevice(_ context.Context, record store.DeviceMutationRecord) (controlplane.DeviceSummary, error) {
	r.disableRecord = record
	return controlplane.DeviceSummary{ID: record.DeviceID, Status: controlplane.DeviceStatusDisabled}, nil
}

func (r *normalizedDeviceLifecycleStub) UnbindDevice(_ context.Context, record store.DeviceMutationRecord) (controlplane.DeviceSummary, error) {
	r.unbindRecord = record
	return controlplane.DeviceSummary{ID: record.DeviceID, Status: controlplane.DeviceStatusPendingActivation}, nil
}

func (r *normalizedModelLeaseStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelLeaseStub) CreateModelLease(_ context.Context, record store.ModelLeaseCreateRecord) (controlplane.ModelLease, error) {
	r.record = record
	return controlplane.ModelLease{ID: "lease_created", UserID: record.UserID, DeviceID: record.DeviceID, Provider: record.Provider, Model: record.Model, Status: controlplane.ModelLeaseStatusActive}, nil
}

func (r *normalizedModelLeaseStub) RenewModelLease(_ context.Context, record store.ModelLeaseRenewRecord) (controlplane.ModelLease, error) {
	r.renewRecord = record
	return controlplane.ModelLease{ID: record.LeaseID, Status: controlplane.ModelLeaseStatusActive}, nil
}

func (r *normalizedModelLeaseStub) ReleaseModelLease(_ context.Context, record store.ModelLeaseReleaseRecord) (controlplane.ReleaseModelLeaseResult, error) {
	r.releaseRecord = record
	return controlplane.ReleaseModelLeaseResult{LeaseID: record.LeaseID, Released: true}, nil
}

func (r *normalizedModelLeaseStub) ReclaimModelLease(_ context.Context, record store.ModelLeaseReclaimRecord) (controlplane.ReleaseModelLeaseResult, error) {
	r.reclaimRecord = record
	return controlplane.ReleaseModelLeaseResult{LeaseID: record.LeaseID, Released: true}, nil
}

func (r *normalizedModelLeaseCreatorStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelLeaseCreatorStub) CreateModelLease(_ context.Context, record store.ModelLeaseCreateRecord) (controlplane.ModelLease, error) {
	r.record = record
	return controlplane.ModelLease{ID: "lease_created", UserID: record.UserID, DeviceID: record.DeviceID, Provider: record.Provider, Model: record.Model, Status: controlplane.ModelLeaseStatusActive}, nil
}

func (r *normalizedModelUsageStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelUsageStub) RecordDirectLLMCall(_ context.Context, record store.ModelUsageWriteRecord) (controlplane.ModelUsageRecord, error) {
	r.record = record
	return controlplane.ModelUsageRecord{ID: "usage_1", LeaseID: record.Input.LeaseID, ClientCallID: record.Input.ClientCallID}, nil
}

func (r *normalizedModelPoolStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelPoolStub) DisableModelPoolAccount(_ context.Context, record store.ModelPoolAccountMutationRecord) (controlplane.ModelPoolAccountSummary, error) {
	r.disableRecord = record
	return controlplane.ModelPoolAccountSummary{ID: record.AccountID, Status: controlplane.ModelAccountStatusDisabled}, nil
}

func (r *normalizedModelPoolStub) UpdateModelPoolAccount(_ context.Context, record store.ModelPoolAccountUpdateRecord) (controlplane.ModelPoolAccountSummary, error) {
	r.updateRecord = record
	return controlplane.ModelPoolAccountSummary{ID: record.AccountID, Status: controlplane.ModelAccountStatusActive, BaseURL: valueOrEmpty(record.Input.BaseURL)}, nil
}

func (r *normalizedModelPoolCreatorStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelPoolCreatorStub) CreateModelPoolAccount(_ context.Context, record store.ModelPoolAccountCreateRecord) (controlplane.ModelPoolAccountSummary, error) {
	r.record = record
	return controlplane.ModelPoolAccountSummary{ID: "mpa_created", Provider: record.Provider, Model: record.Model, SecretConfigured: true}, nil
}

func (r *normalizedModelPoolTestStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedModelPoolTestStub) PrepareModelPoolAccountTest(_ context.Context, record store.ModelPoolTestPrepareRecord) (store.ModelPoolTestPreparation, error) {
	r.prepareRecord = record
	return r.preparation, nil
}

func (r *normalizedModelPoolTestStub) RecordModelPoolAccountTest(_ context.Context, record store.ModelPoolTestRecord) (controlplane.ModelPoolConnectivityTestResult, error) {
	r.record = record
	return record.Result, nil
}

func (r *normalizedAuditStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedAuditStub) RecordAudit(_ context.Context, input controlplane.AuditLogInput) error {
	r.input = input
	return nil
}

func valueOrEmpty(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

func (r *normalizedHeartbeatRecorderStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedHeartbeatRecorderStub) RecordHeartbeatWithSessionBinding(_ context.Context, record store.DeviceHeartbeatRecord) (controlplane.HeartbeatResult, error) {
	r.record = record
	return controlplane.HeartbeatResult{AcceptedAt: r.Now().Format(time.RFC3339), DeviceStatus: controlplane.DeviceStatusActive}, nil
}

func (r *normalizedDeviceActivatorStub) UsesNormalizedReadSource() bool { return true }

func (r *normalizedDeviceActivatorStub) ActivateDeviceWithSessionBinding(_ context.Context, record store.DeviceActivationRecord) (controlplane.DeviceSummary, error) {
	r.record = record
	return controlplane.DeviceSummary{ID: record.Device.DeviceID, UserID: record.UserID, Status: controlplane.DeviceStatusActive, LastSeenAt: r.Now().Format(time.RFC3339)}, nil
}

func (r *transactionalMemoryRepository) RunWithSessionBinding(ctx context.Context, accessTokenHash, userID, deviceID string, fn store.StateOperation) error {
	r.bindCalls++
	r.accessTokenHash = accessTokenHash
	r.userID = userID
	r.deviceID = deviceID
	return r.MemoryStore.Run(ctx, fn)
}

func TestDeviceFlowsUseTransactionalSessionBindingRunner(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &transactionalMemoryRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	code, err := svc.CreateActivationCode(ctx, "transactional-activation", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	device, err := svc.ActivateDeviceWithSessionBinding(ctx, "transactional-activate", "usr_local_admin", "access_hash", controlplane.ActivateDeviceInput{
		ActivationCode: *code.PlainCode,
		Device: controlplane.DeviceRegistration{
			DeviceID: "dev_transactional01", DeviceName: "Transactional Device", Platform: "windows", AppVersion: "1.0.0",
		},
	})
	if err != nil {
		t.Fatalf("ActivateDeviceWithSessionBinding() error = %v", err)
	}
	if device.ID != "dev_transactional01" || repository.bindCalls != 1 || repository.accessTokenHash != "access_hash" || repository.deviceID != device.ID {
		t.Fatalf("transactional activation = device=%+v bind_calls=%d hash=%q user=%q device_id=%q", device, repository.bindCalls, repository.accessTokenHash, repository.userID, repository.deviceID)
	}

	if _, err := svc.RecordHeartbeatWithSessionBinding(ctx, "transactional-heartbeat", "usr_local_admin", "access_hash", controlplane.HeartbeatInput{
		DeviceID: device.ID,
		SentAt:   now,
		Status:   controlplane.HeartbeatStatus{CurrentMediaName: "demo.mp4", PlaybackState: "playing"},
	}); err != nil {
		t.Fatalf("RecordHeartbeatWithSessionBinding() error = %v", err)
	}
	if repository.bindCalls != 2 || repository.deviceID != device.ID {
		t.Fatalf("transactional heartbeat bind calls = %d, device_id=%q", repository.bindCalls, repository.deviceID)
	}
}

func TestActivateDeviceWithSessionBindingUsesNormalizedActivator(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &normalizedDeviceActivatorStub{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	device, err := svc.ActivateDeviceWithSessionBinding(context.Background(), "normalized-activate", "usr_1", "access-hash", controlplane.ActivateDeviceInput{
		ActivationCode: "code_0123456789",
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_normalized01", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	})
	if err != nil {
		t.Fatalf("ActivateDeviceWithSessionBinding() error = %v", err)
	}
	if device.ID != "dev_normalized01" || repository.record.AccessTokenHash != "access-hash" || repository.record.ActivationCodeHash == "" || repository.record.Scope != "control-plane-state" || repository.record.IdempotencyKey != "activate-device:usr_1:normalized-activate" {
		t.Fatalf("normalized activation route = device=%+v record=%+v", device, repository.record)
	}
}

func TestRecordHeartbeatWithSessionBindingUsesNormalizedRecorder(t *testing.T) {
	now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
	repository := &normalizedHeartbeatRecorderStub{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	result, err := svc.RecordHeartbeatWithSessionBinding(context.Background(), "normalized-heartbeat", "usr_1", "access-hash", controlplane.HeartbeatInput{
		DeviceID: "dev_normalized01",
		SentAt:   now,
		Status:   controlplane.HeartbeatStatus{DiskFreeBytes: 1024, PlaybackState: "playing"},
	})
	if err != nil {
		t.Fatalf("RecordHeartbeatWithSessionBinding() error = %v", err)
	}
	if result.DeviceStatus != controlplane.DeviceStatusActive || repository.record.AccessTokenHash != "access-hash" || repository.record.Scope != "control-plane-state" || repository.record.IdempotencyKey != "heartbeat:usr_1:dev_normalized01:normalized-heartbeat" {
		t.Fatalf("normalized heartbeat route = result=%+v record=%+v", result, repository.record)
	}
}

func TestNormalizedSessionBindingPassesSuccessAuditIntoTransaction(t *testing.T) {
	now := time.Date(2026, 8, 21, 14, 0, 0, 0, time.UTC)
	activationRepository := &normalizedDeviceActivatorStub{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	activationService := NewControlPlaneWithRepository(activationRepository)
	activationAudit := controlplane.AuditLogInput{Action: "POST /api/v1/client/activate", TargetType: "device", TargetID: "dev_audit01", Outcome: "success", StatusCode: 200, RequestID: "req-audit-activate"}
	if _, err := activationService.ActivateDeviceWithSessionBindingAndAudit(context.Background(), "activate-audit", "usr_1", "access-hash", controlplane.ActivateDeviceInput{
		ActivationCode: "code_0123456789",
		Device:         controlplane.DeviceRegistration{DeviceID: "dev_audit01", DeviceName: "Demo", Platform: "windows", AppVersion: "1.0.0"},
	}, activationAudit); err != nil {
		t.Fatalf("ActivateDeviceWithSessionBindingAndAudit() error = %v", err)
	}
	if activationRepository.record.Audit != activationAudit {
		t.Fatalf("activation audit = %+v, want %+v", activationRepository.record.Audit, activationAudit)
	}

	heartbeatRepository := &normalizedHeartbeatRecorderStub{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	heartbeatService := NewControlPlaneWithRepository(heartbeatRepository)
	heartbeatAudit := controlplane.AuditLogInput{Action: "POST /api/v1/client/heartbeat", TargetType: "device", TargetID: "dev_audit01", Outcome: "success", StatusCode: 200, RequestID: "req-audit-heartbeat"}
	if _, err := heartbeatService.RecordHeartbeatWithSessionBindingAndAudit(context.Background(), "heartbeat-audit", "usr_1", "access-hash", controlplane.HeartbeatInput{DeviceID: "dev_audit01", SentAt: now}, heartbeatAudit); err != nil {
		t.Fatalf("RecordHeartbeatWithSessionBindingAndAudit() error = %v", err)
	}
	if heartbeatRepository.record.Audit != heartbeatAudit {
		t.Fatalf("heartbeat audit = %+v, want %+v", heartbeatRepository.record.Audit, heartbeatAudit)
	}
}

func TestNormalizedDeviceLifecyclePassesSuccessAuditIntoTransaction(t *testing.T) {
	repository := &normalizedDeviceLifecycleStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	audit := controlplane.AuditLogInput{Action: "POST /api/v1/admin/devices/dev_1/disable", TargetType: "device", TargetID: "dev_1", Outcome: "success", StatusCode: 200, RequestID: "req-lifecycle"}
	if _, err := svc.DisableDeviceWithAudit(context.Background(), "disable-audit", "dev_1", audit); err != nil {
		t.Fatalf("DisableDeviceWithAudit() error = %v", err)
	}
	if repository.disableRecord.Audit != audit {
		t.Fatalf("disable audit = %+v, want %+v", repository.disableRecord.Audit, audit)
	}
	if _, err := svc.UnbindDeviceWithAudit(context.Background(), "unbind-audit", "dev_1", audit); err != nil {
		t.Fatalf("UnbindDeviceWithAudit() error = %v", err)
	}
	if repository.unbindRecord.Audit != audit {
		t.Fatalf("unbind audit = %+v, want %+v", repository.unbindRecord.Audit, audit)
	}
}

func TestDeviceLifecycleUsesNormalizedRepository(t *testing.T) {
	repository := &normalizedDeviceLifecycleStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	if !svc.SupportsTransactionalDeviceLifecycle() {
		t.Fatal("normalized device lifecycle should report transactional support")
	}
	disabled, err := svc.DisableDevice(context.Background(), "disable-normalized", "dev_normalized01")
	if err != nil {
		t.Fatalf("DisableDevice() error = %v", err)
	}
	if disabled.Status != controlplane.DeviceStatusDisabled || repository.disableRecord.IdempotencyKey != "disable-device:dev_normalized01:disable-normalized" {
		t.Fatalf("normalized disable route = device=%+v record=%+v", disabled, repository.disableRecord)
	}
	unbound, err := svc.UnbindDevice(context.Background(), "unbind-normalized", "dev_normalized01")
	if err != nil {
		t.Fatalf("UnbindDevice() error = %v", err)
	}
	if unbound.Status != controlplane.DeviceStatusPendingActivation || repository.unbindRecord.IdempotencyKey != "unbind-device:dev_normalized01:unbind-normalized" {
		t.Fatalf("normalized unbind route = device=%+v record=%+v", unbound, repository.unbindRecord)
	}
}

func TestModelLeaseLifecycleUsesNormalizedRepository(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &normalizedModelLeaseStub{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	lease, err := svc.RenewModelLease(context.Background(), "renew-normalized", "usr_1", "dev_1", "lease_1", controlplane.RenewModelLeaseInput{ExtendSeconds: 300})
	if err != nil {
		t.Fatalf("RenewModelLease() error = %v", err)
	}
	if lease.ID != "lease_1" || repository.renewRecord.IdempotencyKey != "renew-model-lease:lease_1:renew-normalized" || repository.renewRecord.Scope != "control-plane-state" {
		t.Fatalf("normalized renew route = lease=%+v record=%+v", lease, repository.renewRecord)
	}
	released, err := svc.ReleaseModelLease(context.Background(), "release-normalized", "usr_1", "dev_1", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "done"})
	if err != nil {
		t.Fatalf("ReleaseModelLease() error = %v", err)
	}
	if !released.Released || repository.releaseRecord.IdempotencyKey != "release-model-lease:lease_1:release-normalized" {
		t.Fatalf("normalized release route = result=%+v record=%+v", released, repository.releaseRecord)
	}
	reclaimed, err := svc.ReclaimModelLease(context.Background(), "reclaim-normalized", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "stale"})
	if err != nil {
		t.Fatalf("ReclaimModelLease() error = %v", err)
	}
	if !reclaimed.Released || repository.reclaimRecord.IdempotencyKey != "admin-reclaim-model-lease:lease_1:reclaim-normalized" {
		t.Fatalf("normalized reclaim route = result=%+v record=%+v", reclaimed, repository.reclaimRecord)
	}
}

func TestCreateModelLeaseUsesTransactionalNormalizedRepository(t *testing.T) {
	repository := &normalizedModelLeaseCreatorStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	lease, err := svc.CreateModelLease(context.Background(), "create-lease", "usr_1", "dev_1", controlplane.CreateModelLeaseInput{Provider: "openai", Model: "gpt", Purpose: "chat"})
	if err != nil {
		t.Fatalf("CreateModelLease() error = %v", err)
	}
	if lease.ID != "lease_created" || repository.record.IdempotencyKey != "create-model-lease:usr_1:dev_1:create-lease" || repository.record.Scope != "control-plane-state" || repository.record.MaxDurationSeconds != 300 {
		t.Fatalf("normalized lease create route = lease=%+v record=%+v", lease, repository.record)
	}
}

func TestNormalizedModelLeasePassesSuccessAuditIntoTransaction(t *testing.T) {
	repository := &normalizedModelLeaseStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	audit := controlplane.AuditLogInput{Action: "POST /api/v1/client/model-leases", TargetType: "model_lease", Outcome: "success", StatusCode: 200, RequestID: "req-lease-audit"}
	if _, err := svc.CreateModelLeaseWithAudit(context.Background(), "create-lease-audit", "usr_1", "dev_1", controlplane.CreateModelLeaseInput{Provider: "openai", Model: "gpt", Purpose: "chat"}, audit); err != nil {
		t.Fatalf("CreateModelLeaseWithAudit() error = %v", err)
	}
	if repository.record.Audit != audit {
		t.Fatalf("create lease audit = %+v, want %+v", repository.record.Audit, audit)
	}
	if _, err := svc.RenewModelLeaseWithAudit(context.Background(), "renew-lease-audit", "usr_1", "dev_1", "lease_1", controlplane.RenewModelLeaseInput{ExtendSeconds: 300}, audit); err != nil {
		t.Fatalf("RenewModelLeaseWithAudit() error = %v", err)
	}
	if repository.renewRecord.Audit != audit {
		t.Fatalf("renew lease audit = %+v, want %+v", repository.renewRecord.Audit, audit)
	}
	if _, err := svc.ReleaseModelLeaseWithAudit(context.Background(), "release-lease-audit", "usr_1", "dev_1", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "done"}, audit); err != nil {
		t.Fatalf("ReleaseModelLeaseWithAudit() error = %v", err)
	}
	if repository.releaseRecord.Audit != audit {
		t.Fatalf("release lease audit = %+v, want %+v", repository.releaseRecord.Audit, audit)
	}
	if _, err := svc.ReclaimModelLeaseWithAudit(context.Background(), "reclaim-lease-audit", "lease_1", controlplane.ReleaseModelLeaseInput{Reason: "stale"}, audit); err != nil {
		t.Fatalf("ReclaimModelLeaseWithAudit() error = %v", err)
	}
	if repository.reclaimRecord.Audit != audit {
		t.Fatalf("reclaim lease audit = %+v, want %+v", repository.reclaimRecord.Audit, audit)
	}
}

func TestModelUsageUsesNormalizedRepository(t *testing.T) {
	repository := &normalizedModelUsageStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	usage, err := svc.RecordDirectLLMCall(context.Background(), "usage-normalized", "usr_1", "dev_1", "req-1", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_0001", LeaseID: "lease_0001", Provider: "openai", Model: "gpt", InputTokens: 2, OutputTokens: 3, Status: "succeeded", UsageSource: "client_reported",
	})
	if err != nil {
		t.Fatalf("RecordDirectLLMCall() error = %v", err)
	}
	if usage.ID != "usage_1" || repository.record.IdempotencyKey != "record-direct-llm-call:usr_1:dev_1:usage-normalized" || repository.record.Input.TotalTokens != 5 || repository.record.Input.LeaseID != "lease_0001" || repository.record.Scope != "control-plane-state" {
		t.Fatalf("normalized usage route = usage=%+v record=%+v", usage, repository.record)
	}
}

func TestModelUsagePassesSuccessAuditIntoTransaction(t *testing.T) {
	repository := &normalizedModelUsageStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	audit := controlplane.AuditLogInput{Action: "POST /api/v1/client/llm/call-records", TargetType: "model_usage", Outcome: "success", StatusCode: 200, RequestID: "req-usage-audit"}
	if _, err := svc.RecordDirectLLMCallWithAudit(context.Background(), "usage-audit", "usr_1", "dev_1", "req-usage-audit", controlplane.CreateDirectLLMCallRecordInput{
		ClientCallID: "call_audit_01", LeaseID: "lease_0001", Provider: "openai", Model: "gpt", InputTokens: 2, OutputTokens: 3, Status: "succeeded", UsageSource: "client_reported",
	}, audit); err != nil {
		t.Fatalf("RecordDirectLLMCallWithAudit() error = %v", err)
	}
	if repository.record.Audit != audit {
		t.Fatalf("usage audit = %+v, want %+v", repository.record.Audit, audit)
	}
}

func TestModelPoolAccountLifecycleUsesNormalizedRepository(t *testing.T) {
	repository := &normalizedModelPoolStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	disabled, err := svc.DisableModelPoolAccount(context.Background(), "disable-account", "mpa_00000001")
	if err != nil {
		t.Fatalf("DisableModelPoolAccount() error = %v", err)
	}
	if disabled.Status != controlplane.ModelAccountStatusDisabled || repository.disableRecord.IdempotencyKey != "disable-model-account:disable-account" {
		t.Fatalf("normalized account disable route = summary=%+v record=%+v", disabled, repository.disableRecord)
	}
	baseURL := "https://api.example.test"
	updated, err := svc.UpdateModelPoolAccount(context.Background(), "update-account", "mpa_00000001", controlplane.UpdateModelPoolAccountInput{BaseURL: &baseURL})
	if err != nil {
		t.Fatalf("UpdateModelPoolAccount() error = %v", err)
	}
	if updated.BaseURL != baseURL || repository.updateRecord.IdempotencyKey != "update-model-account:update-account" || repository.updateRecord.Scope != "control-plane-state" {
		t.Fatalf("normalized account update route = summary=%+v record=%+v", updated, repository.updateRecord)
	}
}

func TestCreateModelPoolAccountUsesTransactionalNormalizedRepository(t *testing.T) {
	repository := &normalizedModelPoolCreatorStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	created, err := svc.CreateModelPoolAccount(context.Background(), "create-account", controlplane.CreateModelPoolAccountInput{
		Provider: "openai", Model: "gpt", APIKey: "sk-test-secret", ConcurrencyLimit: 2,
	})
	if err != nil {
		t.Fatalf("CreateModelPoolAccount() error = %v", err)
	}
	if created.ID != "mpa_created" || repository.record.IdempotencyKey != "create-model-account:create-account" || repository.record.Scope != "control-plane-state" || repository.record.APIKey != "sk-test-secret" {
		t.Fatalf("normalized account create route = summary=%+v record=%+v", created, repository.record)
	}
}

func TestNormalizedModelPoolPassesSuccessAuditIntoTransaction(t *testing.T) {
	repository := &normalizedModelPoolStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	audit := controlplane.AuditLogInput{Action: "model-account.mutation", TargetType: "model_account", Outcome: "success", StatusCode: 200, RequestID: "req-account-audit"}
	if _, err := svc.DisableModelPoolAccountWithAudit(context.Background(), "disable-account-audit", "mpa_1", audit); err != nil {
		t.Fatalf("DisableModelPoolAccountWithAudit() error = %v", err)
	}
	if repository.disableRecord.Audit != audit {
		t.Fatalf("disable account audit = %+v, want %+v", repository.disableRecord.Audit, audit)
	}
	baseURL := "https://api.example.test"
	if _, err := svc.UpdateModelPoolAccountWithAudit(context.Background(), "update-account-audit", "mpa_1", controlplane.UpdateModelPoolAccountInput{BaseURL: &baseURL}, audit); err != nil {
		t.Fatalf("UpdateModelPoolAccountWithAudit() error = %v", err)
	}
	if repository.updateRecord.Audit != audit {
		t.Fatalf("update account audit = %+v, want %+v", repository.updateRecord.Audit, audit)
	}

	creator := &normalizedModelPoolCreatorStub{MemoryStore: store.NewMemoryStore(time.Now)}
	creatorSvc := NewControlPlaneWithRepository(creator)
	if _, err := creatorSvc.CreateModelPoolAccountWithAudit(context.Background(), "create-account-audit", controlplane.CreateModelPoolAccountInput{Provider: "openai", Model: "gpt", APIKey: "sk-test-secret", ConcurrencyLimit: 1}, audit); err != nil {
		t.Fatalf("CreateModelPoolAccountWithAudit() error = %v", err)
	}
	if creator.record.Audit != audit {
		t.Fatalf("create account audit = %+v, want %+v", creator.record.Audit, audit)
	}
}

func TestAuditUsesNormalizedRepository(t *testing.T) {
	repository := &normalizedAuditStub{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	if err := svc.RecordAudit(context.Background(), controlplane.AuditLogInput{Action: "user.update", TargetType: "user", Outcome: "success", StatusCode: 200}); err != nil {
		t.Fatalf("RecordAudit() error = %v", err)
	}
	if repository.input.Action != "user.update" || repository.input.Outcome != "success" || repository.input.StatusCode != 200 {
		t.Fatalf("normalized audit route = %+v", repository.input)
	}
}
