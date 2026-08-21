package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestEnsureConfiguredAdminPersistsCredentialAndDoesNotOverwriteIt(t *testing.T) {
	ctx := context.Background()
	svc := NewControlPlane(store.NewMemoryStore(time.Now))
	if err := svc.EnsureConfiguredAdmin(ctx, "admin", "first-password"); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() first error = %v", err)
	}
	if _, _, err := svc.AuthenticateUser(ctx, "admin", "first-password"); err != nil {
		t.Fatalf("AuthenticateUser() first password error = %v", err)
	}
	if err := svc.EnsureConfiguredAdmin(ctx, "admin", "second-password"); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() restart error = %v", err)
	}
	if _, _, err := svc.AuthenticateUser(ctx, "admin", "first-password"); err != nil {
		t.Fatalf("persisted password error = %v", err)
	}
	if _, _, err := svc.AuthenticateUser(ctx, "admin", "second-password"); !controlplane.IsErrorCode(err, controlplane.ErrUnauthenticated.Code) {
		t.Fatalf("replacement password error = %v, want unauthenticated", err)
	}
}

func TestEnsureConfiguredAdminRejectsInvalidBootstrap(t *testing.T) {
	svc := NewControlPlane(store.NewMemoryStore(time.Now))
	if err := svc.EnsureConfiguredAdmin(context.Background(), "admin", "short"); err == nil {
		t.Fatal("EnsureConfiguredAdmin() error = nil, want invalid credential error")
	}
	if err := svc.CheckReady(context.Background()); err == nil {
		t.Fatal("CheckReady() error = nil before administrator initialization")
	}
	if err := svc.EnsureConfiguredAdmin(context.Background(), "", "valid-password"); err == nil {
		t.Fatal("EnsureConfiguredAdmin() accepted empty username")
	}
}

func TestChangeLocalAdminPasswordRotatesPersistedCredentialIdempotently(t *testing.T) {
	ctx := context.Background()
	svc := NewControlPlane(store.NewMemoryStore(time.Now))
	if err := svc.EnsureConfiguredAdmin(ctx, "admin", "first-password"); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() error = %v", err)
	}

	input := controlplane.ChangeLocalAdminPasswordInput{Password: "rotated-password"}
	user, err := svc.ChangeLocalAdminPassword(ctx, "rotate-local-admin", input)
	if err != nil {
		t.Fatalf("ChangeLocalAdminPassword() error = %v", err)
	}
	if user.ID != "usr_local_admin" {
		t.Fatalf("rotated user = %+v", user)
	}
	if _, _, err := svc.AuthenticateUser(ctx, "admin", "first-password"); !controlplane.IsErrorCode(err, controlplane.ErrUnauthenticated.Code) {
		t.Fatalf("old password error = %v, want unauthenticated", err)
	}
	if _, _, err := svc.AuthenticateUser(ctx, "admin", "rotated-password"); err != nil {
		t.Fatalf("rotated password error = %v", err)
	}

	if _, err := svc.ChangeLocalAdminPassword(ctx, "rotate-local-admin", input); err != nil {
		t.Fatalf("idempotent rotation error = %v", err)
	}
	if _, err := svc.ChangeLocalAdminPassword(ctx, "rotate-local-admin", controlplane.ChangeLocalAdminPasswordInput{Password: "different-password"}); !controlplane.IsErrorCode(err, controlplane.ErrIdempotencyConflict.Code) {
		t.Fatalf("idempotency conflict error = %v, want conflict", err)
	}
	if _, err := svc.ChangeLocalAdminPassword(ctx, "short-key", controlplane.ChangeLocalAdminPasswordInput{Password: "short"}); !controlplane.IsErrorCode(err, controlplane.ErrInvalidRequest.Code) {
		t.Fatalf("short password error = %v, want invalid request", err)
	}
}

func TestUpdateAndResetUserCredentials(t *testing.T) {
	now := time.Date(2026, 8, 20, 11, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	user, err := svc.CreateUser(ctx, "create-user", controlplane.CreateUserInput{
		Username: "operator",
		Password: "old-password",
		Role:     controlplane.RoleUser,
	})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}

	updatedName := "operator-renamed"
	updated, err := svc.UpdateUser(ctx, "update-user", user.ID, controlplane.UpdateUserInput{Username: &updatedName})
	if err != nil {
		t.Fatalf("UpdateUser() error = %v", err)
	}
	if updated.Username != updatedName {
		t.Fatalf("updated user = %+v", updated)
	}
	if _, _, err := svc.AuthenticateUser(ctx, updatedName, "old-password"); err != nil {
		t.Fatalf("AuthenticateUser() after rename error = %v", err)
	}

	reset, err := svc.ResetUserPassword(ctx, "reset-user", user.ID, controlplane.ResetUserPasswordInput{Password: "new-password"})
	if err != nil {
		t.Fatalf("ResetUserPassword() error = %v", err)
	}
	if reset.ID != user.ID {
		t.Fatalf("reset user = %+v", reset)
	}
	if _, _, err := svc.AuthenticateUser(ctx, updatedName, "old-password"); !controlplane.IsErrorCode(err, controlplane.ErrUnauthenticated.Code) {
		t.Fatalf("old password error = %v, want UNAUTHENTICATED", err)
	}
	if _, _, err := svc.AuthenticateUser(ctx, updatedName, "new-password"); err != nil {
		t.Fatalf("new password error = %v", err)
	}
}

func TestGetUserAuthorizationSummarySeparatesActiveAccessAndSoftUsage(t *testing.T) {
	now := time.Date(2026, 8, 20, 11, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users["usr_summary"] = controlplane.UserSummary{ID: "usr_summary", Username: "summary-user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.Devices["dev_summary1"] = controlplane.DeviceSummary{ID: "dev_summary1", UserID: "usr_summary", Status: controlplane.DeviceStatusActive}
		state.Devices["dev_summary2"] = controlplane.DeviceSummary{ID: "dev_summary2", UserID: "usr_summary", Status: controlplane.DeviceStatusDisabled}
		state.ModelLeases["lease_summary1"] = controlplane.ModelLease{ID: "lease_summary1", AccountID: "account_summary1", UserID: "usr_summary", DeviceID: "dev_summary1", Status: controlplane.ModelLeaseStatusActive}
		state.ModelLeases["lease_summary2"] = controlplane.ModelLease{ID: "lease_summary2", AccountID: "account_summary1", UserID: "usr_summary", DeviceID: "dev_summary2", Status: controlplane.ModelLeaseStatusReleased}
		state.ModelUsageRecords["usage_summary1"] = controlplane.ModelUsageRecord{ID: "usage_summary1", LeaseID: "lease_summary1", TotalTokens: 12, CreatedAt: now.Add(-time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_summary2"] = controlplane.ModelUsageRecord{ID: "usage_summary2", LeaseID: "lease_summary2", TotalTokens: 5, CreatedAt: now.Add(-2 * time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_summary3"] = controlplane.ModelUsageRecord{ID: "usage_summary3", LeaseID: "lease_summary1", TotalTokens: 8, CreatedAt: now.Add(-25 * time.Hour).Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("seed authorization summary state: %v", err)
	}

	summary, err := NewControlPlane(repository).GetUserAuthorizationSummary(context.Background(), "usr_summary")
	if err != nil {
		t.Fatalf("GetUserAuthorizationSummary() error = %v", err)
	}
	if summary.UserID != "usr_summary" || summary.DeviceCount != 2 || summary.ActiveDeviceCount != 1 || summary.ActiveLeaseCount != 1 || summary.ActiveAccountCount != 1 || summary.DailyUsedTokens != 17 {
		t.Fatalf("authorization summary = %+v", summary)
	}
	if summary.UsageSource != "client_reported_soft" || summary.HardQuotaConfigured {
		t.Fatalf("authorization quota semantics = %+v", summary)
	}
}

func TestDisableUserPreservesLastActiveAdmin(t *testing.T) {
	now := time.Date(2026, 8, 20, 11, 0, 0, 0, time.UTC)
	svc := NewControlPlane(store.NewMemoryStore(func() time.Time { return now }))
	ctx := context.Background()
	if err := svc.EnsureLocalAdmin(ctx, "admin"); err != nil {
		t.Fatalf("EnsureLocalAdmin() error = %v", err)
	}
	if _, err := svc.DisableUser(ctx, "disable-local", "usr_local_admin"); !controlplane.IsErrorCode(err, controlplane.ErrCannotDisableLocalAdmin.Code) {
		t.Fatalf("DisableUser(local admin) error = %v", err)
	}
	user, err := svc.CreateUser(ctx, "create-admin", controlplane.CreateUserInput{
		Username: "second-admin",
		Password: "admin-password",
		Role:     controlplane.RoleAdmin,
	})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if _, err := svc.DisableUser(ctx, "disable-second", user.ID); err != nil {
		t.Fatalf("DisableUser(second admin) error = %v", err)
	}

	name := "admin-renamed"
	if _, err := svc.UpdateUser(ctx, "update-local", "usr_local_admin", controlplane.UpdateUserInput{Username: &name}); !controlplane.IsErrorCode(err, controlplane.ErrCannotModifyLocalAdmin.Code) {
		t.Fatalf("UpdateUser(local admin) error = %v", err)
	}
}
