package main

import (
	"testing"
	"time"

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

func TestMigrationOperationTimeoutMustBePositive(t *testing.T) {
	if err := validateMigrationOperationTimeout(0); err == nil {
		t.Fatal("validateMigrationOperationTimeout(0) error = nil, want error")
	}
	if err := validateMigrationOperationTimeout(-time.Second); err == nil {
		t.Fatal("validateMigrationOperationTimeout(-1s) error = nil, want error")
	}
	if err := validateMigrationOperationTimeout(time.Second); err != nil {
		t.Fatalf("validateMigrationOperationTimeout(1s) error = %v", err)
	}
}
