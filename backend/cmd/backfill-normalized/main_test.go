package main

import (
	"testing"

	"autoLive/backend/internal/config"
)

func TestValidateBackfillConfigRequiresExplicitConfirmation(t *testing.T) {
	t.Setenv("APP_NORMALIZED_BACKFILL_CONFIRM", "")
	cfg := config.Config{StorageMode: config.StorageModePostgres, DatabaseURL: "postgres://example"}
	if err := validateBackfillConfig(cfg); err == nil {
		t.Fatal("validateBackfillConfig() error = nil, want explicit confirmation")
	}

	t.Setenv("APP_NORMALIZED_BACKFILL_CONFIRM", confirmation)
	if err := validateBackfillConfig(cfg); err != nil {
		t.Fatalf("validateBackfillConfig() error = %v, want confirmation to pass", err)
	}
}

func TestValidateBackfillConfigRequiresPostgresAndDatabase(t *testing.T) {
	t.Setenv("APP_NORMALIZED_BACKFILL_CONFIRM", confirmation)
	if err := validateBackfillConfig(config.Config{StorageMode: config.StorageModeMemory, DatabaseURL: "postgres://example"}); err == nil {
		t.Fatal("validateBackfillConfig() error = nil, want postgres mode requirement")
	}
	if err := validateBackfillConfig(config.Config{StorageMode: config.StorageModePostgres}); err == nil {
		t.Fatal("validateBackfillConfig() error = nil, want database URL requirement")
	}
}
