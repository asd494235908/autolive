//go:build postgres_integration

package migrations

import (
	"context"
	"database/sql"
	"fmt"
	"net/url"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/golang-migrate/migrate/v4"
	"github.com/golang-migrate/migrate/v4/database/postgres"
	"github.com/golang-migrate/migrate/v4/source/iofs"
	"github.com/lib/pq"
)

func TestPostgresMigration23CompatibilityAndOrphanRecovery(t *testing.T) {
	baseURL := strings.TrimSpace(os.Getenv("TEST_POSTGRES_URL"))
	if baseURL == "" {
		t.Skip("TEST_POSTGRES_URL is not set")
	}

	t.Run("0018 orphan auth_session survives 0022 to 0023 forward migration", func(t *testing.T) {
		_, schemaDB, migrationTable, cleanup := openIsolatedMigrationDatabase(t, baseURL)
		defer cleanup()

		applyMigrationsToVersion(t, schemaDB, migrationTable, 22)

		now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
		userID := fmt.Sprintf("migration_orphan_user_%d", time.Now().UTC().UnixNano())
		sessionID := fmt.Sprintf("migration_orphan_session_%d", time.Now().UTC().UnixNano())
		deviceID := fmt.Sprintf("migration_orphan_device_%d", time.Now().UTC().UnixNano())
		if _, err := schemaDB.ExecContext(context.Background(), `
			INSERT INTO users (id, username, password_hash, role, status, created_at)
			VALUES ($1, $2, $3, $4, $5, $6)
		`, userID, userID, "$2a$10$migration-hash", "user", "active", now); err != nil {
			t.Fatalf("seed migration orphan user: %v", err)
		}
		if _, err := schemaDB.ExecContext(context.Background(), `
			INSERT INTO auth_sessions (
				id, user_id, device_id, access_token_hash, refresh_token_hash,
				access_expires_at, refresh_expires_at, created_at, device_bound_at
			) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
		`, sessionID, userID, deviceID, "access-"+sessionID, "refresh-"+sessionID, now.Add(time.Hour), now.Add(24*time.Hour), now, now); err != nil {
			t.Fatalf("seed orphan auth session: %v", err)
		}

		applyMigrationsToVersion(t, schemaDB, migrationTable, LatestVersion)

		version, dirty := readMigrationVersion(t, schemaDB, migrationTable)
		if dirty || version != LatestVersion {
			t.Fatalf("migration version after upgrade = (%d, dirty=%v), want (%d, false)", version, dirty, LatestVersion)
		}
		assertMigration23ForeignKeys(t, schemaDB)

		var storedProduct, storedDeviceID string
		if err := schemaDB.QueryRowContext(context.Background(), `
			SELECT product, device_id
			FROM auth_sessions
			WHERE id = $1
		`, sessionID).Scan(&storedProduct, &storedDeviceID); err != nil {
			t.Fatalf("read migrated orphan auth session: %v", err)
		}
		if storedProduct != "autolive" || storedDeviceID != deviceID {
			t.Fatalf("migrated orphan session = (%q, %q), want (%q, %q)", storedProduct, storedDeviceID, "autolive", deviceID)
		}

		var membershipCount int
		if err := schemaDB.QueryRowContext(context.Background(), `
			SELECT COUNT(*)
			FROM user_products
			WHERE user_id = $1 AND product = 'autolive'
		`, userID).Scan(&membershipCount); err != nil {
			t.Fatalf("count migrated autolive membership: %v", err)
		}
		if membershipCount != 1 {
			t.Fatalf("autolive membership count = %d, want 1", membershipCount)
		}
	})

	t.Run("clean forward migration keeps legacy inserts compatible", func(t *testing.T) {
		_, schemaDB, migrationTable, cleanup := openIsolatedMigrationDatabase(t, baseURL)
		defer cleanup()

		applyMigrationsToVersion(t, schemaDB, migrationTable, LatestVersion)

		version, dirty := readMigrationVersion(t, schemaDB, migrationTable)
		if dirty || version != LatestVersion {
			t.Fatalf("migration version after clean forward = (%d, dirty=%v), want (%d, false)", version, dirty, LatestVersion)
		}
		assertMigration23ForeignKeys(t, schemaDB)

		now := time.Date(2026, 8, 21, 13, 0, 0, 0, time.UTC)
		userID := fmt.Sprintf("migration_clean_user_%d", time.Now().UTC().UnixNano())
		deviceID := fmt.Sprintf("migration_clean_device_%d", time.Now().UTC().UnixNano())
		sessionID := fmt.Sprintf("migration_clean_session_%d", time.Now().UTC().UnixNano())
		if _, err := schemaDB.ExecContext(context.Background(), `
			INSERT INTO users (id, username, password_hash, role, status, created_at)
			VALUES ($1, $2, $3, $4, $5, $6)
		`, userID, userID, "$2a$10$migration-hash", "user", "active", now); err != nil {
			t.Fatalf("insert clean forward user: %v", err)
		}
		if _, err := schemaDB.ExecContext(context.Background(), `
			INSERT INTO devices (id, user_id, device_key, device_name, platform, client_version, status, last_heartbeat_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
		`, deviceID, userID, "state-device/"+deviceID, "Migration Device", "windows", "integration", "active", now); err != nil {
			t.Fatalf("insert legacy-shaped device after 0023: %v", err)
		}
		if _, err := schemaDB.ExecContext(context.Background(), `
			INSERT INTO auth_sessions (
				id, user_id, audience, device_id, access_token_hash, refresh_token_hash,
				token_family_id, generation,
				access_expires_at, refresh_expires_at, created_at, device_bound_at
			) VALUES ($1, $2, 'desktop', $3, $4, $5, $1, 0, $6, $7, $8, $9)
		`, sessionID, userID, deviceID, "access-"+sessionID, "refresh-"+sessionID, now.Add(time.Hour), now.Add(24*time.Hour), now, now); err != nil {
			t.Fatalf("insert legacy-shaped auth session after 0023: %v", err)
		}

		var deviceProduct, sessionProduct string
		if err := schemaDB.QueryRowContext(context.Background(), `SELECT product FROM devices WHERE id = $1`, deviceID).Scan(&deviceProduct); err != nil {
			t.Fatalf("read legacy device product: %v", err)
		}
		if err := schemaDB.QueryRowContext(context.Background(), `SELECT product FROM auth_sessions WHERE id = $1`, sessionID).Scan(&sessionProduct); err != nil {
			t.Fatalf("read legacy auth session product: %v", err)
		}
		if deviceProduct != "autolive" || sessionProduct != "autolive" {
			t.Fatalf("legacy write products = (%q, %q), want (%q, %q)", deviceProduct, sessionProduct, "autolive", "autolive")
		}

		var membershipCount int
		if err := schemaDB.QueryRowContext(context.Background(), `
			SELECT COUNT(*)
			FROM user_products
			WHERE user_id = $1 AND product = 'autolive'
		`, userID).Scan(&membershipCount); err != nil {
			t.Fatalf("count clean forward autolive membership: %v", err)
		}
		if membershipCount != 1 {
			t.Fatalf("clean forward autolive membership count = %d, want 1", membershipCount)
		}
	})
}

func assertMigration23ForeignKeys(t *testing.T, database *sql.DB) {
	t.Helper()

	names := make([]string, 0, len(migration23ForeignKeyContracts))
	for _, contract := range migration23ForeignKeyContracts {
		names = append(names, contract.name)
	}

	rows, err := database.QueryContext(context.Background(), `
		SELECT c.conname, child.relname, parent.relname, current_schema(), pg_get_constraintdef(c.oid)
		FROM pg_constraint AS c
		JOIN pg_class AS child ON child.oid = c.conrelid
		JOIN pg_namespace AS child_namespace ON child_namespace.oid = child.relnamespace
		JOIN pg_class AS parent ON parent.oid = c.confrelid
		JOIN pg_namespace AS parent_namespace ON parent_namespace.oid = parent.relnamespace
		WHERE c.contype = 'f'
		  AND child_namespace.nspname = current_schema()
		  AND parent_namespace.nspname = current_schema()
		  AND c.conname = ANY($1::text[])
	`, pq.Array(names))
	if err != nil {
		t.Fatalf("query migration 0023 foreign keys: %v", err)
	}
	defer rows.Close()

	type actualForeignKey struct {
		table       string
		parentTable string
		definition  string
	}
	actual := make(map[string]actualForeignKey, len(names))
	for rows.Next() {
		var name, table, parentTable, schemaName, definition string
		if err := rows.Scan(&name, &table, &parentTable, &schemaName, &definition); err != nil {
			t.Fatalf("scan migration 0023 foreign key: %v", err)
		}
		definition = strings.ReplaceAll(definition, fmt.Sprintf(`"%s".`, schemaName), "")
		definition = strings.ReplaceAll(definition, schemaName+".", "")
		actual[name] = actualForeignKey{
			table:       table,
			parentTable: parentTable,
			definition:  strings.Join(strings.Fields(definition), " "),
		}
	}
	if err := rows.Err(); err != nil {
		t.Fatalf("iterate migration 0023 foreign keys: %v", err)
	}

	for _, contract := range migration23ForeignKeyContracts {
		got, ok := actual[contract.name]
		if !ok {
			t.Errorf("migration 0023 foreign key %q is missing", contract.name)
			continue
		}
		wantDefinition := fmt.Sprintf(
			"FOREIGN KEY (%s) REFERENCES %s(%s)",
			contract.localColumns,
			contract.parentTable,
			contract.referencedColumns,
		)
		if got.table != contract.table || got.parentTable != contract.parentTable || got.definition != wantDefinition {
			t.Errorf("foreign key %q = table %s, parent %s, definition %q; want table %s, parent %s, definition %q", contract.name, got.table, got.parentTable, got.definition, contract.table, contract.parentTable, wantDefinition)
		}
	}
}

func openIsolatedMigrationDatabase(t *testing.T, baseURL string) (*sql.DB, *sql.DB, string, func()) {
	t.Helper()

	adminDB, err := sql.Open("postgres", baseURL)
	if err != nil {
		t.Fatalf("sql.Open admin DB: %v", err)
	}
	schemaName := fmt.Sprintf("migration_%d", time.Now().UTC().UnixNano())
	migrationTable := fmt.Sprintf("schema_migrations_%d", time.Now().UTC().UnixNano())
	if _, err := adminDB.ExecContext(context.Background(), `CREATE SCHEMA `+schemaName); err != nil {
		adminDB.Close()
		t.Fatalf("create schema %s: %v", schemaName, err)
	}

	schemaURL := withSearchPath(t, baseURL, schemaName)
	schemaDB, err := sql.Open("postgres", schemaURL)
	if err != nil {
		_, _ = adminDB.ExecContext(context.Background(), `DROP SCHEMA `+schemaName+` CASCADE`)
		adminDB.Close()
		t.Fatalf("sql.Open schema DB: %v", err)
	}

	cleanup := func() {
		_ = schemaDB.Close()
		_, _ = adminDB.ExecContext(context.Background(), `DROP SCHEMA `+schemaName+` CASCADE`)
		_ = adminDB.Close()
	}
	return adminDB, schemaDB, migrationTable, cleanup
}

func withSearchPath(t *testing.T, rawURL, schemaName string) string {
	t.Helper()

	parsed, err := url.Parse(rawURL)
	if err != nil {
		t.Fatalf("parse TEST_POSTGRES_URL: %v", err)
	}
	query := parsed.Query()
	query.Set("search_path", schemaName)
	parsed.RawQuery = query.Encode()
	return parsed.String()
}

func applyMigrationsToVersion(t *testing.T, database *sql.DB, migrationTable string, target uint) {
	t.Helper()

	source, err := iofs.New(FS, ".")
	if err != nil {
		t.Fatalf("open embedded migrations: %v", err)
	}
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatalf("open postgres migration connection: %v", err)
	}
	driver, err := postgres.WithConnection(context.Background(), connection, &postgres.Config{
		MigrationsTable: migrationTable,
	})
	if err != nil {
		_ = connection.Close()
		t.Fatalf("create postgres migration driver: %v", err)
	}
	migrator, err := migrate.NewWithInstance("iofs", source, "postgres", driver)
	if err != nil {
		_ = driver.Close()
		t.Fatalf("create migration runner: %v", err)
	}
	defer func() {
		_, _ = migrator.Close()
	}()
	if err := migrator.Migrate(target); err != nil && err != migrate.ErrNoChange {
		t.Fatalf("migrate to %d: %v", target, err)
	}
}

func readMigrationVersion(t *testing.T, database *sql.DB, migrationTable string) (int, bool) {
	t.Helper()

	var version int
	var dirty bool
	if err := database.QueryRowContext(context.Background(), `SELECT version, dirty FROM `+migrationTable+` LIMIT 1`).Scan(&version, &dirty); err != nil {
		t.Fatalf("read schema_migrations: %v", err)
	}
	return version, dirty
}
