//go:build postgres_integration

package store

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"

	"github.com/lib/pq"
)

func TestPostgresSecretReferenceCleanupHonorsCutoffAndProtectedReferences(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	suffix := time.Now().UTC().UnixNano()
	oldReference := "model-account/retention-old-" + formatIntegrationSuffix(suffix)
	protectedReference := "model-account/retention-protected-" + formatIntegrationSuffix(suffix)
	freshReference := "model-account/retention-fresh-" + formatIntegrationSuffix(suffix)
	for _, reference := range []string{oldReference, protectedReference, freshReference} {
		if err := secretStore.Put(ctx, reference, "integration-secret"); err != nil {
			t.Fatalf("seed secret %q: %v", reference, err)
		}
	}
	cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(func() {
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = ANY($1::text[])`, pq.Array([]string{oldReference, protectedReference, freshReference}))
	})
	cutoff := time.Now().UTC().Add(-time.Hour)
	if _, err := database.ExecContext(ctx, `UPDATE model_account_secrets SET updated_at = $2 WHERE secret_ref = $1`, oldReference, cutoff.Add(-time.Minute)); err != nil {
		t.Fatalf("age old secret: %v", err)
	}
	if _, err := database.ExecContext(ctx, `UPDATE model_account_secrets SET updated_at = $2 WHERE secret_ref = $1`, protectedReference, cutoff.Add(-time.Minute)); err != nil {
		t.Fatalf("age protected secret: %v", err)
	}

	deleted, err := secretStore.CleanupSecretReferences(ctx, RetentionCleanupRequest{
		Cutoff: cutoff, BatchSize: 10,
	}, []string{oldReference, protectedReference, freshReference}, []string{protectedReference})
	if err != nil {
		t.Fatalf("CleanupSecretReferences() error = %v", err)
	}
	if len(deleted) != 1 || deleted[0] != oldReference {
		t.Fatalf("deleted references = %v, want only %q", deleted, oldReference)
	}
	if _, err := secretStore.Get(ctx, oldReference); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("old secret after cleanup error = %v, want ErrSecretNotFound", err)
	}
	for _, reference := range []string{protectedReference, freshReference} {
		if _, err := secretStore.Get(ctx, reference); err != nil {
			t.Fatalf("retained secret %q lookup error = %v", reference, err)
		}
	}
}

func TestPostgresNormalizedStagedSecretCleanupPreservesActiveReferenceAcrossRestart(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	accountID := "normalized_secret_cleanup_account_" + formatIntegrationSuffix(suffix)
	baseReference := "model-account/" + accountID
	activeReference := baseReference + "/rotation_active"
	orphanReference := baseReference + "/rotation_orphan"
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "cleanup-model", "https://provider.example.test", activeReference,
		controlplane.ModelAccountStatusActive, 1, 1, 100, now); err != nil {
		t.Fatalf("seed model account: %v", err)
	}
	for _, reference := range []string{activeReference, orphanReference} {
		if err := secretStore.Put(ctx, reference, "integration-secret-"+reference); err != nil {
			t.Fatalf("seed secret %q: %v", reference, err)
		}
	}
	cutoff := now.Add(-time.Hour)
	if _, err := database.ExecContext(ctx, `
		UPDATE model_account_secrets
		SET updated_at = $2
		WHERE secret_ref = ANY($1::text[])
	`, pq.Array([]string{activeReference, orphanReference}), cutoff.Add(-time.Minute)); err != nil {
		t.Fatalf("age staged secrets: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = ANY($1::text[])`, pq.Array([]string{activeReference, orphanReference}))
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
	})

	// Reconstruct both stores to model a process restart before recovery runs.
	restartedSecretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("restarted secret store constructor: %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, restartedSecretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted repository constructor: %v", err)
	}
	request := RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 10}
	deleted, err := repository.CleanupUnreferencedStagedSecrets(ctx, request)
	if err != nil {
		t.Fatalf("normalized staged cleanup: %v", err)
	}
	if deleted != 1 {
		t.Fatalf("deleted = %d, want only orphan staged secret", deleted)
	}
	if stored, err := restartedSecretStore.Get(ctx, activeReference); err != nil || stored == "" {
		t.Fatalf("active secret after cleanup = (%q, %v)", stored, err)
	}
	if _, err := restartedSecretStore.Get(ctx, orphanReference); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("orphan secret after cleanup error = %v, want ErrSecretNotFound", err)
	}
	deleted, err = repository.CleanupUnreferencedStagedSecrets(ctx, request)
	if err != nil {
		t.Fatalf("repeated normalized staged cleanup: %v", err)
	}
	if deleted != 0 {
		t.Fatalf("repeated cleanup deleted = %d, want 0", deleted)
	}
}

func formatIntegrationSuffix(value int64) string {
	return time.Unix(0, value).UTC().Format("20060102150405.000000000")
}
