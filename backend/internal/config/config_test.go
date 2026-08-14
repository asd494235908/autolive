package config

import (
	"testing"
	"time"
)

func TestLoadFromEnvUsesDefaults(t *testing.T) {
	t.Setenv("APP_ADDR", "")
	t.Setenv("APP_VERSION", "")
	t.Setenv("APP_STORAGE_MODE", "")
	t.Setenv("APP_DATABASE_URL", "")
	t.Setenv("APP_SECRET_ENCRYPTION_KEY", "")
	t.Setenv("APP_MODEL_READ_SOURCE", "")
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "")

	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}

	if cfg.ListenAddr != defaultListenAddr {
		t.Fatalf("ListenAddr = %q, want %q", cfg.ListenAddr, defaultListenAddr)
	}

	if cfg.ServiceVersion != defaultServiceVersion {
		t.Fatalf("ServiceVersion = %q, want %q", cfg.ServiceVersion, defaultServiceVersion)
	}
	if cfg.StorageMode != StorageModeMemory {
		t.Fatalf("StorageMode = %q, want %q", cfg.StorageMode, StorageModeMemory)
	}

	if cfg.ShutdownTimeout != defaultShutdownTimeout {
		t.Fatalf("ShutdownTimeout = %v, want %v", cfg.ShutdownTimeout, defaultShutdownTimeout)
	}
}

func TestLoadFromEnvParsesSemanticValues(t *testing.T) {
	t.Setenv("APP_ADDR", "127.0.0.1:9090")
	t.Setenv("APP_VERSION", "phase1")
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "3s")

	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}

	if cfg.ListenAddr != "127.0.0.1:9090" {
		t.Fatalf("ListenAddr = %q", cfg.ListenAddr)
	}

	if cfg.ServiceVersion != "phase1" {
		t.Fatalf("ServiceVersion = %q", cfg.ServiceVersion)
	}

	if cfg.ShutdownTimeout != 3*time.Second {
		t.Fatalf("ShutdownTimeout = %v", cfg.ShutdownTimeout)
	}
}

func TestLoadFromEnvRejectsInvalidTimeout(t *testing.T) {
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "not-a-duration")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want error")
	}
}

func TestLoadFromEnvRequiresDatabaseURLForPostgres(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_DATABASE_URL", "")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want missing database URL error")
	}
}

func TestLoadFromEnvRejectsUnknownStorageMode(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", "sqlite")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want unknown storage mode error")
	}
}

func TestLoadFromEnvRejectsNormalizedModelSourceInMemoryMode(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModeMemory)
	t.Setenv("APP_MODEL_READ_SOURCE", ModelReadSourceNormalized)
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want normalized source storage error")
	}
}

func TestLoadFromEnvAcceptsNormalizedModelSourceForPostgres(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_MODEL_READ_SOURCE", ModelReadSourceNormalized)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}
	if cfg.ModelReadSource != ModelReadSourceNormalized {
		t.Fatalf("ModelReadSource = %q, want %q", cfg.ModelReadSource, ModelReadSourceNormalized)
	}
}
