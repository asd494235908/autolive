//go:build postgres_integration

package store

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedModelLeaseDetailReadsWithoutLegacySnapshot(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := "lease_detail_user_" + formatIntegrationSuffix(suffix)
	deviceID := "lease_detail_device_" + formatIntegrationSuffix(suffix)
	accountID := "lease_detail_account_" + formatIntegrationSuffix(suffix)
	leaseID := "lease_detail_lease_" + formatIntegrationSuffix(suffix)
	createdAt := now.Add(-time.Minute)
	expiresAt := now.Add(time.Hour)

	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_leases WHERE id = $1`, leaseID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, "lease-detail-user-"+formatIntegrationSuffix(suffix), "integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, createdAt); err != nil {
		t.Fatalf("insert user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, client_version, status)
		VALUES ($1, $2, $3, $4, $5)
	`, deviceID, userID, "lease-detail-key-"+formatIntegrationSuffix(suffix), "integration", controlplane.DeviceStatusActive); err != nil {
		t.Fatalf("insert device: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, concurrency_limit, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
	`, accountID, "openai", "rewrite", "https://provider.example/v1", "model-account/"+accountID, controlplane.ModelAccountStatusActive, 2, createdAt); err != nil {
		t.Fatalf("insert model account: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, provider, model, proxy_mode, concurrency_limit)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
	`, leaseID, accountID, userID, deviceID, "validation", controlplane.ModelLeaseStatusActive, expiresAt, createdAt, "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 2); err != nil {
		t.Fatalf("insert model lease: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	detail, err := repository.GetModelLeaseAdminDetail(ctx, leaseID)
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetail(): %v", err)
	}
	if detail.ID != leaseID || detail.UserID != userID || detail.DeviceID != deviceID || detail.Status != controlplane.ModelLeaseStatusActive {
		t.Fatalf("normalized lease detail = %+v", detail)
	}
}
