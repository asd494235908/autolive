//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedUserAuthorizationSummaryReadsDomainFacts(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	suffix := time.Now().UTC().UnixNano()
	userID := fmt.Sprintf("auth_summary_user_%d", suffix)
	deviceActiveID := fmt.Sprintf("auth_summary_device_active_%d", suffix)
	deviceDisabledID := fmt.Sprintf("auth_summary_device_disabled_%d", suffix)
	accountID := fmt.Sprintf("auth_summary_account_%d", suffix)
	leaseActiveID := fmt.Sprintf("auth_summary_lease_active_%d", suffix)
	leaseExpiredID := fmt.Sprintf("auth_summary_lease_expired_%d", suffix)
	usageTodayID := fmt.Sprintf("auth_summary_usage_today_%d", suffix)
	usageYesterdayID := fmt.Sprintf("auth_summary_usage_yesterday_%d", suffix)
	usageForeignID := fmt.Sprintf("auth_summary_usage_foreign_%d", suffix)
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_usage_records WHERE id IN ($1, $2, $3)`, usageTodayID, usageYesterdayID, usageForeignID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_leases WHERE id IN ($1, $2)`, leaseActiveID, leaseExpiredID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM user_authorization_policies WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id IN ($1, $2)`, deviceActiveID, deviceDisabledID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, "auth-summary-"+fmt.Sprint(suffix), "integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, client_version, status, device_name, platform)
		VALUES ($1, $2, $3, $4, $5, $6, $7), ($8, $2, $9, $4, $10, $11, $12)
	`, deviceActiveID, userID, "key/"+deviceActiveID, "1.0.0", controlplane.DeviceStatusActive, "active", "windows",
		deviceDisabledID, "key/"+deviceDisabledID, controlplane.DeviceStatusDisabled, "disabled", "linux"); err != nil {
		t.Fatalf("seed devices: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, active_requests, daily_reserved_tokens, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $6, 0, 1, 0, 0, 0, $7, $7)
	`, accountID, "integration", "summary-model", "https://provider.example.test", "secret/"+accountID, "active", now); err != nil {
		t.Fatalf("seed account: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, provider, model, proxy_mode, concurrency_limit)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 1), ($12, $2, $3, $4, $5, $6, $13, $8, $9, $10, $11, 1)
	`, leaseActiveID, accountID, userID, deviceActiveID, "client_direct", "active", now.Add(time.Hour), now, "integration", "summary-model", "direct", leaseExpiredID, now.Add(-time.Hour)); err != nil {
		t.Fatalf("seed leases: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO user_authorization_policies (user_id, allowed_models, daily_token_limit, updated_at)
		VALUES ($1, $2::jsonb, $3, $4)
	`, userID, `["summary-model"]`, 100, now); err != nil {
		t.Fatalf("seed authorization policy: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_usage_records (id, account_id, lease_id, user_id, device_id, provider, model, total_tokens, status, created_at, request_id, client_call_id, usage_source)
		VALUES ($1, $2, $3, $4, $5, $6, $7, 17, $8, $9, $10, $11, $12),
		       ($13, $2, $3, $4, $5, $6, $7, 99, $8, $14, $10, $15, $12),
		       ($16, $2, $17, $4, $5, $6, $7, 1000, $8, $9, $10, $16, $12)
	`, usageTodayID, accountID, leaseActiveID, userID, deviceActiveID, "integration", "summary-model", "accepted", now, "request/"+usageTodayID, "call/"+usageTodayID, "client_reported",
		usageYesterdayID, now.Add(-24*time.Hour), "call/"+usageYesterdayID,
		usageForeignID, nil); err != nil {
		t.Fatalf("seed usage records: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	summary, err := repository.GetUserAuthorizationSummary(ctx, userID)
	if err != nil {
		t.Fatalf("GetUserAuthorizationSummary() error = %v", err)
	}
	if summary.UserID != userID || summary.DeviceCount != 2 || summary.ActiveDeviceCount != 1 || summary.ActiveLeaseCount != 1 || summary.ActiveAccountCount != 1 || summary.DailyUsedTokens != 17 || summary.DailyTokenLimit != 100 {
		t.Fatalf("authorization summary = %+v", summary)
	}
	if len(summary.AllowedModels) != 1 || summary.AllowedModels[0] != "summary-model" || summary.HardQuotaConfigured || summary.UsageSource != "client_reported_soft" {
		t.Fatalf("authorization policy/quota semantics = %+v", summary)
	}
}
