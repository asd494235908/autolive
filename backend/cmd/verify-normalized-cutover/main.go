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
	if err := validateCutoverConfig(cfg); err != nil {
		logger.Error("normalized cutover verification rejected", "error", err)
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
	repository, err := store.NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, time.Now, nil, store.ModelReadSourceNormalized, cfg.RequestTimeout)
	if err != nil {
		logger.Error("create normalized repository failed", "error", err)
		os.Exit(1)
	}
	report, err := repository.VerifyNormalizedCutover(ctx)
	if err != nil {
		logger.Error("normalized cutover preflight failed", "error", err)
		os.Exit(1)
	}
	tableCounts := make(map[string]int64, len(report.Tables))
	for _, table := range report.Tables {
		tableCounts[table.Name] = table.Rows
	}
	logger.Info("normalized cutover preflight passed", "backfill_status", report.BackfillStatus, "backfill_completed_at", report.BackfillCompletedAt.UTC().Format(time.RFC3339), "table_counts", tableCounts)
}

func validateCutoverConfig(cfg config.Config) error {
	if cfg.StorageMode != config.StorageModePostgres {
		return fmt.Errorf("normalized cutover verification requires APP_STORAGE_MODE=%q", config.StorageModePostgres)
	}
	if cfg.ModelReadSource != config.ModelReadSourceNormalized {
		return fmt.Errorf("normalized cutover verification requires APP_MODEL_READ_SOURCE=%q", config.ModelReadSourceNormalized)
	}
	if cfg.DatabaseURL == "" {
		return errors.New("APP_DATABASE_URL must be set")
	}
	if os.Getenv("APP_NORMALIZED_CUTOVER_CONFIRM") != confirmation {
		return fmt.Errorf("set APP_NORMALIZED_CUTOVER_CONFIRM=%s to run the read-only preflight", confirmation)
	}
	return nil
}
