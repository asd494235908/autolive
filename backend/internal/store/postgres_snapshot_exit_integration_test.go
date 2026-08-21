//go:build postgres_integration

package store

import (
	"context"
	"testing"
	"time"
)

func TestPostgresNormalizedRuntimeDoesNotDependOnLegacySnapshot(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	var snapshot []byte
	if err := database.QueryRowContext(ctx, `SELECT state FROM control_plane_state WHERE id = TRUE`).Scan(&snapshot); err != nil {
		t.Fatalf("read legacy snapshot: %v", err)
	}
	t.Cleanup(func() {
		restoreCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(restoreCtx, `
			INSERT INTO control_plane_state (id, state, updated_at)
			VALUES (TRUE, $1, CURRENT_TIMESTAMP)
			ON CONFLICT (id) DO UPDATE SET state = EXCLUDED.state, updated_at = EXCLUDED.updated_at
		`, snapshot)
	})
	if _, err := database.ExecContext(ctx, `DELETE FROM control_plane_state WHERE id = TRUE`); err != nil {
		t.Fatalf("remove legacy snapshot: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	if _, err := repository.ListUsersPage(ctx, 0, 10); err != nil {
		t.Fatalf("normalized page read after snapshot removal: %v", err)
	}
	if _, err := repository.CleanupAuditLogs(ctx, RetentionCleanupRequest{Cutoff: time.Now().UTC(), BatchSize: 10}); err != nil {
		t.Fatalf("normalized retention cleanup after snapshot removal: %v", err)
	}
	if err := repository.Run(ctx, func(*State) error { return nil }); err != nil {
		t.Fatalf("normalized state operation after snapshot removal: %v", err)
	}
}
