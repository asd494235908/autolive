package config

import (
	"fmt"
	"os"
	"strings"
	"time"
)

const (
	defaultListenAddr      = ":8080"
	defaultServiceVersion  = "dev"
	defaultStorageMode     = StorageModeMemory
	defaultShutdownTimeout = 10 * time.Second
	defaultModelReadSource = ModelReadSourceSnapshot
)

const (
	StorageModeMemory   = "memory"
	StorageModePostgres = "postgres"

	ModelReadSourceSnapshot   = "snapshot"
	ModelReadSourceNormalized = "normalized"
)

type Config struct {
	ListenAddr          string
	ServiceVersion      string
	StorageMode         string
	ModelReadSource     string
	DatabaseURL         string
	SecretEncryptionKey string
	AdminUsername       string
	AdminPassword       string
	ShutdownTimeout     time.Duration
}

func LoadFromEnv() (Config, error) {
	cfg := Config{
		ListenAddr:          valueOrDefault("APP_ADDR", defaultListenAddr),
		ServiceVersion:      valueOrDefault("APP_VERSION", defaultServiceVersion),
		StorageMode:         valueOrDefault("APP_STORAGE_MODE", defaultStorageMode),
		ModelReadSource:     valueOrDefault("APP_MODEL_READ_SOURCE", defaultModelReadSource),
		DatabaseURL:         strings.TrimSpace(os.Getenv("APP_DATABASE_URL")),
		SecretEncryptionKey: strings.TrimSpace(os.Getenv("APP_SECRET_ENCRYPTION_KEY")),
		AdminUsername:       strings.TrimSpace(os.Getenv("APP_ADMIN_USERNAME")),
		AdminPassword:       os.Getenv("APP_ADMIN_PASSWORD"),
		ShutdownTimeout:     defaultShutdownTimeout,
	}

	if strings.TrimSpace(cfg.ListenAddr) == "" {
		return Config{}, fmt.Errorf("APP_ADDR must not be empty")
	}

	if strings.TrimSpace(cfg.ServiceVersion) == "" {
		return Config{}, fmt.Errorf("APP_VERSION must not be empty")
	}

	if cfg.StorageMode != StorageModeMemory && cfg.StorageMode != StorageModePostgres {
		return Config{}, fmt.Errorf("APP_STORAGE_MODE must be %q or %q", StorageModeMemory, StorageModePostgres)
	}
	if cfg.ModelReadSource != ModelReadSourceSnapshot && cfg.ModelReadSource != ModelReadSourceNormalized {
		return Config{}, fmt.Errorf("APP_MODEL_READ_SOURCE must be %q or %q", ModelReadSourceSnapshot, ModelReadSourceNormalized)
	}
	if cfg.StorageMode != StorageModePostgres && cfg.ModelReadSource != ModelReadSourceSnapshot {
		return Config{}, fmt.Errorf("APP_MODEL_READ_SOURCE=%q requires APP_STORAGE_MODE=%q", ModelReadSourceNormalized, StorageModePostgres)
	}
	if cfg.StorageMode == StorageModePostgres && cfg.DatabaseURL == "" {
		return Config{}, fmt.Errorf("APP_DATABASE_URL must be set when APP_STORAGE_MODE=%q", StorageModePostgres)
	}

	if raw := strings.TrimSpace(os.Getenv("APP_SHUTDOWN_TIMEOUT")); raw != "" {
		timeout, err := time.ParseDuration(raw)
		if err != nil {
			return Config{}, fmt.Errorf("parse APP_SHUTDOWN_TIMEOUT: %w", err)
		}
		if timeout <= 0 {
			return Config{}, fmt.Errorf("APP_SHUTDOWN_TIMEOUT must be greater than 0")
		}
		cfg.ShutdownTimeout = timeout
	}

	return cfg, nil
}

func valueOrDefault(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}
