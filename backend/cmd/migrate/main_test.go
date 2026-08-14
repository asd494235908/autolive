package main

import (
	"testing"

	"autoLive/backend/internal/config"
)

func TestValidateMigrationConfigRequiresPostgresMode(t *testing.T) {
	cfg := config.Config{StorageMode: config.StorageModeMemory, DatabaseURL: "postgres://example"}
	if err := validateMigrationConfig(cfg); err == nil {
		t.Fatal("validateMigrationConfig() error = nil, want postgres mode error")
	}
}

func TestValidateMigrationConfigRequiresDatabaseURL(t *testing.T) {
	cfg := config.Config{StorageMode: config.StorageModePostgres}
	if err := validateMigrationConfig(cfg); err == nil {
		t.Fatal("validateMigrationConfig() error = nil, want database URL error")
	}
}

func TestValidateMigrationConfigAcceptsConfiguredPostgres(t *testing.T) {
	cfg := config.Config{
		StorageMode: config.StorageModePostgres,
		DatabaseURL: "postgres://example",
	}
	if err := validateMigrationConfig(cfg); err != nil {
		t.Fatalf("validateMigrationConfig() error = %v, want nil", err)
	}
}
