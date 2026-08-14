package migrationscheck

import (
	"context"
	"database/sql"
	"fmt"

	"autoLive/backend/migrations"
)

// ValidateApplied 确认数据库已执行到应用要求的迁移版本；迁移执行仍由 cmd/migrate 负责。
func ValidateApplied(ctx context.Context, db *sql.DB) error {
	var version int
	var dirty bool
	if err := db.QueryRowContext(ctx, `SELECT version, dirty FROM schema_migrations LIMIT 1`).Scan(&version, &dirty); err != nil {
		return fmt.Errorf("read schema migration version: %w", err)
	}
	if dirty {
		return fmt.Errorf("database migrations are dirty at version %d", version)
	}
	if version != migrations.LatestVersion {
		return fmt.Errorf("database migration version %d does not match application version %d", version, migrations.LatestVersion)
	}
	return nil
}
