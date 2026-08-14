package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"time"

	"autoLive/backend/internal/config"
	"autoLive/backend/migrations"

	"github.com/golang-migrate/migrate/v4"
	"github.com/golang-migrate/migrate/v4/database/postgres"
	"github.com/golang-migrate/migrate/v4/source/iofs"
	_ "github.com/lib/pq"
)

const databaseOperationTimeout = 30 * time.Second

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg, err := config.LoadFromEnv()
	if err != nil {
		logger.Error("load config failed", "error", err)
		os.Exit(1)
	}
	if err := validateMigrationConfig(cfg); err != nil {
		logger.Error("migration configuration rejected", "error", err)
		os.Exit(1)
	}

	if err := migrateDatabase(cfg.DatabaseURL); err != nil {
		logger.Error("database migration failed", "error", err)
		os.Exit(1)
	}
	logger.Info("database migrations applied")
}

func validateMigrationConfig(cfg config.Config) error {
	if cfg.StorageMode != config.StorageModePostgres {
		return fmt.Errorf("migration command requires APP_STORAGE_MODE=%q", config.StorageModePostgres)
	}
	if cfg.DatabaseURL == "" {
		return errors.New("APP_DATABASE_URL must be set")
	}
	return nil
}

func migrateDatabase(databaseURL string) error {
	database, err := sql.Open("postgres", databaseURL)
	if err != nil {
		return fmt.Errorf("open postgres: %w", err)
	}
	defer database.Close()

	ctx, cancel := context.WithTimeout(context.Background(), databaseOperationTimeout)
	defer cancel()
	if err := database.PingContext(ctx); err != nil {
		return fmt.Errorf("ping postgres: %w", err)
	}

	source, err := iofs.New(migrations.FS, ".")
	if err != nil {
		return fmt.Errorf("open embedded migrations: %w", err)
	}
	driver, err := postgres.WithInstance(database, &postgres.Config{
		MigrationsTable: "schema_migrations",
	})
	if err != nil {
		return fmt.Errorf("create postgres migration driver: %w", err)
	}
	migrator, err := migrate.NewWithInstance("iofs", source, "postgres", driver)
	if err != nil {
		return fmt.Errorf("create migration runner: %w", err)
	}
	defer func() {
		_, _ = migrator.Close()
	}()
	if err := migrator.Up(); err != nil && !errors.Is(err, migrate.ErrNoChange) {
		return fmt.Errorf("apply migrations: %w", err)
	}
	return nil
}
