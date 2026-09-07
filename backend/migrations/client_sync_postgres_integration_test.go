//go:build postgres_integration

package migrations

import (
	"context"
	"os"
	"strings"
	"testing"
)

func TestPostgresMigration28CreatesClientSyncConstraints(t *testing.T) {
	baseURL := strings.TrimSpace(os.Getenv("TEST_POSTGRES_URL"))
	if baseURL == "" {
		t.Skip("TEST_POSTGRES_URL is not set")
	}
	_, database, migrationTable, cleanup := openIsolatedMigrationDatabase(t, baseURL)
	defer cleanup()
	applyMigrationsToVersion(t, database, migrationTable, LatestVersion)

	for _, table := range []string{"client_sync_workspaces", "client_sync_items", "client_sync_mutations"} {
		var exists bool
		if err := database.QueryRowContext(context.Background(), `SELECT to_regclass(current_schema() || '.' || $1) IS NOT NULL`, table).Scan(&exists); err != nil {
			t.Fatalf("query %s existence: %v", table, err)
		}
		if !exists {
			t.Fatalf("migration 0028 table %s is missing", table)
		}
	}

	var workspaceForeignKey, payloadCheck bool
	if err := database.QueryRowContext(context.Background(), `
		SELECT
			EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = 'client_sync_workspaces'::regclass AND conname = 'client_sync_workspaces_user_product_fkey'),
			EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = 'client_sync_items'::regclass AND conname = 'client_sync_items_payload_size_check')
	`).Scan(&workspaceForeignKey, &payloadCheck); err != nil {
		t.Fatalf("query migration 0028 constraints: %v", err)
	}
	if !workspaceForeignKey || !payloadCheck {
		t.Fatalf("migration 0028 constraints = workspace FK %v payload check %v", workspaceForeignKey, payloadCheck)
	}
}
