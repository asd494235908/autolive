package main

import (
	"testing"

	"autoLive/backend/internal/config"
)

func TestValidateCutoverConfigRequiresNormalizedPostgresAndConfirmation(t *testing.T) {
	cfg := config.Config{StorageMode: config.StorageModePostgres, ModelReadSource: config.ModelReadSourceNormalized, DatabaseURL: "postgres://example"}
	t.Setenv("APP_NORMALIZED_CUTOVER_CONFIRM", "")
	if err := validateCutoverConfig(cfg); err == nil {
		t.Fatal("validateCutoverConfig() error = nil, want explicit confirmation")
	}
	t.Setenv("APP_NORMALIZED_CUTOVER_CONFIRM", confirmation)
	if err := validateCutoverConfig(cfg); err != nil {
		t.Fatalf("validateCutoverConfig() error = %v, want nil", err)
	}
}

func TestValidateCutoverConfigRejectsOtherModes(t *testing.T) {
	t.Setenv("APP_NORMALIZED_CUTOVER_CONFIRM", confirmation)
	if err := validateCutoverConfig(config.Config{StorageMode: config.StorageModeMemory, ModelReadSource: config.ModelReadSourceNormalized, DatabaseURL: "postgres://example"}); err == nil {
		t.Fatal("memory mode should be rejected")
	}
	if err := validateCutoverConfig(config.Config{StorageMode: config.StorageModePostgres, ModelReadSource: config.ModelReadSourceSnapshot, DatabaseURL: "postgres://example"}); err == nil {
		t.Fatal("snapshot source should be rejected")
	}
	if err := validateCutoverConfig(config.Config{StorageMode: config.StorageModePostgres, ModelReadSource: config.ModelReadSourceNormalized}); err == nil {
		t.Fatal("missing database URL should be rejected")
	}
}
