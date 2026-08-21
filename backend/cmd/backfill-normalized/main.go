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
	"autoLive/backend/internal/migrationscheck"
	"autoLive/backend/internal/store"
	"autoLive/backend/migrations"

	_ "github.com/lib/pq"
)

const confirmation = "YES"

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg, err := config.LoadFromEnv()
	if err != nil {
		logger.Error("load config failed", "error", err)
		os.Exit(1)
	}
	if err := validateBackfillConfig(cfg); err != nil {
		logger.Error("backfill configuration rejected", "error", err)
		os.Exit(1)
	}

	database, err := sql.Open("postgres", cfg.DatabaseURL)
	if err != nil {
		logger.Error("open postgres failed", "error", err)
		os.Exit(1)
	}
	defer func() { _ = database.Close() }()
	database.SetMaxOpenConns(2)
	database.SetMaxIdleConns(1)

	ctx, cancel := context.WithTimeout(context.Background(), cfg.RequestTimeout)
	defer cancel()
	if err := database.PingContext(ctx); err != nil {
		logger.Error("ping postgres failed", "error", err)
		os.Exit(1)
	}
	if err := migrationscheck.ValidateApplied(ctx, database); err != nil {
		logger.Error("migration version check failed", "error", err)
		os.Exit(1)
	}
	repository, err := store.NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, time.Now, nil, store.ModelReadSourceSnapshot, cfg.RequestTimeout)
	if err != nil {
		logger.Error("create repository failed", "error", err)
		os.Exit(1)
	}
	if err := repository.BackfillNormalized(ctx); err != nil {
		logger.Error("normalized backfill failed", "error", err)
		os.Exit(1)
	}
	logger.Info("normalized backfill completed", "migration_version", migrations.LatestVersion)
}

func validateBackfillConfig(cfg config.Config) error {
	if cfg.StorageMode != config.StorageModePostgres {
		return fmt.Errorf("normalized backfill requires APP_STORAGE_MODE=%q", config.StorageModePostgres)
	}
	if cfg.DatabaseURL == "" {
		return errors.New("APP_DATABASE_URL must be set")
	}
	if os.Getenv("APP_NORMALIZED_BACKFILL_CONFIRM") != confirmation {
		return fmt.Errorf("set APP_NORMALIZED_BACKFILL_CONFIRM=%s to run the one-time backfill", confirmation)
	}
	return nil
}
