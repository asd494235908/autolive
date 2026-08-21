//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"testing"
	"time"

	"golang.org/x/crypto/bcrypt"
)

func TestPostgresNormalizedLocalAdminLifecyclePersistsCredential(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	var existing int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM users WHERE id = 'usr_local_admin'`).Scan(&existing); err != nil {
		t.Fatalf("check existing local admin: %v", err)
	}
	if existing != 0 {
		t.Skip("local administrator already exists in integration database")
	}
	now := time.Now().UTC().Truncate(time.Second)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = 'control-plane-state' AND idempotency_key LIKE 'change-local-admin-password:%'`)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = 'usr_local_admin'`)
	})
	if err := repository.EnsureConfiguredAdmin(ctx, "integration-admin", []byte("$2a$04$integration-hash")); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() error = %v", err)
	}
	if err := repository.CheckAdminReady(ctx); err != nil {
		t.Fatalf("CheckAdminReady() error = %v", err)
	}
	newHash, err := bcrypt.GenerateFromPassword([]byte("rotated-integration-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatalf("bcrypt.GenerateFromPassword() error = %v", err)
	}
	idempotencyKey := fmt.Sprintf("change-local-admin-password:%d", now.UnixNano())
	user, err := repository.ChangeLocalAdminPassword(ctx, "control-plane-state", idempotencyKey, "integration-fingerprint", newHash)
	if err != nil {
		t.Fatalf("ChangeLocalAdminPassword() error = %v", err)
	}
	if user.ID != "usr_local_admin" {
		t.Fatalf("changed admin = %+v", user)
	}
	_, storedHash, err := repository.GetUserCredential(ctx, "integration-admin")
	if err != nil {
		t.Fatalf("read rotated credential: %v", err)
	}
	if err := bcrypt.CompareHashAndPassword(storedHash, []byte("rotated-integration-password")); err != nil {
		t.Fatalf("rotated credential comparison: %v", err)
	}
	if _, err := repository.ChangeLocalAdminPassword(ctx, "control-plane-state", idempotencyKey, "integration-fingerprint", []byte("different-hash")); err != nil {
		t.Fatalf("idempotent local admin rotation: %v", err)
	}
	if err := bcrypt.CompareHashAndPassword(storedHash, []byte("rotated-integration-password")); err != nil {
		t.Fatalf("stored credential after replay: %v", err)
	}
}
