package config

import (
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	defaultListenAddr     = ":8080"
	defaultServiceVersion = "dev"
	defaultStorageMode    = StorageModeMemory
	// Shutdown must outlive one bounded background operation (retention or
	// health probe) and the HTTP request drain budget.
	defaultShutdownTimeout               = 45 * time.Second
	defaultRequestTimeout                = 30 * time.Second
	defaultMigrationTimeout              = 30 * time.Second
	defaultModelHealthProbeInterval      = 5 * time.Minute
	defaultModelHealthProbeTimeout       = 10 * time.Second
	defaultModelHealthProbeMaxConcurrent = 2
	defaultModelHealthProbeMaxAccounts   = 50
	defaultModelHealthProbeBackoffBase   = 30 * time.Second
	defaultModelHealthProbeBackoffMax    = 15 * time.Minute
	defaultRetentionCleanupInterval      = time.Hour
	defaultRetentionCleanupTimeout       = 30 * time.Second
	defaultRetentionCleanupBatch         = 1000
	defaultAuthSessionRetentionTTL       = 30 * 24 * time.Hour
	defaultIdempotencyRetentionTTL       = 30 * 24 * time.Hour
	defaultModelTestRetentionTTL         = 30 * 24 * time.Hour
	defaultAuditLogRetentionTTL          = 365 * 24 * time.Hour
	defaultModelReadSource               = ModelReadSourceSnapshot
	defaultDeploymentEnv                 = DeploymentEnvironmentDevelopment
)

const (
	StorageModeMemory   = "memory"
	StorageModePostgres = "postgres"

	ModelReadSourceSnapshot   = "snapshot"
	ModelReadSourceNormalized = "normalized"

	DeploymentEnvironmentDevelopment = "development"
	DeploymentEnvironmentStaging     = "staging"
	DeploymentEnvironmentProduction  = "production"
)

type Config struct {
	ListenAddr                    string
	ServiceVersion                string
	StorageMode                   string
	ModelReadSource               string
	DatabaseURL                   string
	MigrationMetricsFile          string
	SecretEncryptionKey           string
	AdminUsername                 string
	AdminPassword                 string
	DeploymentEnvironment         string
	PublicBaseURL                 string
	AllowInsecureHTTP             bool
	ShutdownTimeout               time.Duration
	RequestTimeout                time.Duration
	MigrationTimeout              time.Duration
	ModelHealthProbeInterval      time.Duration
	ModelHealthProbeTimeout       time.Duration
	ModelHealthProbeMaxConcurrent int
	ModelHealthProbeMaxAccounts   int
	ModelHealthProbeBackoffBase   time.Duration
	ModelHealthProbeBackoffMax    time.Duration
	RetentionCleanupInterval      time.Duration
	RetentionCleanupTimeout       time.Duration
	RetentionCleanupBatch         int
	AuthSessionRetentionTTL       time.Duration
	IdempotencyRetentionTTL       time.Duration
	ModelTestRetentionTTL         time.Duration
	AuditLogRetentionTTL          time.Duration
}

func LoadFromEnv() (Config, error) {
	cfg := Config{
		ListenAddr:                    valueOrDefault("APP_ADDR", defaultListenAddr),
		ServiceVersion:                valueOrDefault("APP_VERSION", defaultServiceVersion),
		StorageMode:                   valueOrDefault("APP_STORAGE_MODE", defaultStorageMode),
		ModelReadSource:               valueOrDefault("APP_MODEL_READ_SOURCE", defaultModelReadSource),
		DatabaseURL:                   strings.TrimSpace(os.Getenv("APP_DATABASE_URL")),
		MigrationMetricsFile:          strings.TrimSpace(os.Getenv("APP_MIGRATION_METRICS_FILE")),
		SecretEncryptionKey:           strings.TrimSpace(os.Getenv("APP_SECRET_ENCRYPTION_KEY")),
		AdminUsername:                 strings.TrimSpace(os.Getenv("APP_ADMIN_USERNAME")),
		AdminPassword:                 os.Getenv("APP_ADMIN_PASSWORD"),
		DeploymentEnvironment:         valueOrDefault("APP_DEPLOYMENT_ENV", defaultDeploymentEnv),
		PublicBaseURL:                 strings.TrimSpace(os.Getenv("APP_PUBLIC_BASE_URL")),
		ShutdownTimeout:               defaultShutdownTimeout,
		RequestTimeout:                defaultRequestTimeout,
		MigrationTimeout:              defaultMigrationTimeout,
		ModelHealthProbeInterval:      defaultModelHealthProbeInterval,
		ModelHealthProbeTimeout:       defaultModelHealthProbeTimeout,
		ModelHealthProbeMaxConcurrent: defaultModelHealthProbeMaxConcurrent,
		ModelHealthProbeMaxAccounts:   defaultModelHealthProbeMaxAccounts,
		ModelHealthProbeBackoffBase:   defaultModelHealthProbeBackoffBase,
		ModelHealthProbeBackoffMax:    defaultModelHealthProbeBackoffMax,
		RetentionCleanupInterval:      defaultRetentionCleanupInterval,
		RetentionCleanupTimeout:       defaultRetentionCleanupTimeout,
		RetentionCleanupBatch:         defaultRetentionCleanupBatch,
		AuthSessionRetentionTTL:       defaultAuthSessionRetentionTTL,
		IdempotencyRetentionTTL:       defaultIdempotencyRetentionTTL,
		ModelTestRetentionTTL:         defaultModelTestRetentionTTL,
		AuditLogRetentionTTL:          defaultAuditLogRetentionTTL,
	}
	allowInsecureHTTP, err := parseBoolEnv("APP_ALLOW_INSECURE_HTTP")
	if err != nil {
		return Config{}, err
	}
	cfg.AllowInsecureHTTP = allowInsecureHTTP

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
		return Config{}, fmt.Errorf("APP_MODEL_READ_SOURCE=%q is not supported; use %q or %q", cfg.ModelReadSource, ModelReadSourceSnapshot, ModelReadSourceNormalized)
	}
	if cfg.StorageMode != StorageModePostgres && cfg.ModelReadSource == ModelReadSourceNormalized {
		return Config{}, fmt.Errorf("APP_MODEL_READ_SOURCE=%q requires APP_STORAGE_MODE=%q", ModelReadSourceNormalized, StorageModePostgres)
	}
	if cfg.DeploymentEnvironment == DeploymentEnvironmentProduction && cfg.ModelReadSource != ModelReadSourceNormalized {
		return Config{}, fmt.Errorf("APP_MODEL_READ_SOURCE=%q is not permitted in production; run the normalized backfill and use %q", cfg.ModelReadSource, ModelReadSourceNormalized)
	}
	if cfg.StorageMode == StorageModePostgres && cfg.DatabaseURL == "" {
		return Config{}, fmt.Errorf("APP_DATABASE_URL must be set when APP_STORAGE_MODE=%q", StorageModePostgres)
	}
	if cfg.StorageMode == StorageModePostgres {
		if err := validateProductionAdminCredentials(cfg.AdminUsername, cfg.AdminPassword); err != nil {
			return Config{}, err
		}
	}
	if err := validatePublishConfig(cfg.DeploymentEnvironment, cfg.PublicBaseURL, cfg.AllowInsecureHTTP); err != nil {
		return Config{}, err
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
	if raw := strings.TrimSpace(os.Getenv("APP_REQUEST_TIMEOUT")); raw != "" {
		timeout, err := time.ParseDuration(raw)
		if err != nil {
			return Config{}, fmt.Errorf("parse APP_REQUEST_TIMEOUT: %w", err)
		}
		if timeout <= 0 {
			return Config{}, fmt.Errorf("APP_REQUEST_TIMEOUT must be greater than 0")
		}
		cfg.RequestTimeout = timeout
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MIGRATION_TIMEOUT")); raw != "" {
		timeout, err := time.ParseDuration(raw)
		if err != nil {
			return Config{}, fmt.Errorf("parse APP_MIGRATION_TIMEOUT: %w", err)
		}
		if timeout <= 0 {
			return Config{}, fmt.Errorf("APP_MIGRATION_TIMEOUT must be greater than 0")
		}
		cfg.MigrationTimeout = timeout
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_INTERVAL")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration < 0 {
			return Config{}, fmt.Errorf("APP_MODEL_HEALTH_PROBE_INTERVAL must be zero or a positive duration")
		}
		cfg.ModelHealthProbeInterval = duration
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_TIMEOUT")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration <= 0 {
			return Config{}, fmt.Errorf("APP_MODEL_HEALTH_PROBE_TIMEOUT must be a positive duration")
		}
		cfg.ModelHealthProbeTimeout = duration
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT")); raw != "" {
		value, err := parsePositiveIntEnv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", raw)
		if err != nil {
			return Config{}, err
		}
		cfg.ModelHealthProbeMaxConcurrent = value
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_MAX_ACCOUNTS")); raw != "" {
		value, err := parsePositiveIntEnv("APP_MODEL_HEALTH_PROBE_MAX_ACCOUNTS", raw)
		if err != nil {
			return Config{}, err
		}
		cfg.ModelHealthProbeMaxAccounts = value
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_BACKOFF_BASE")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration <= 0 {
			return Config{}, fmt.Errorf("APP_MODEL_HEALTH_PROBE_BACKOFF_BASE must be a positive duration")
		}
		cfg.ModelHealthProbeBackoffBase = duration
	}
	if raw := strings.TrimSpace(os.Getenv("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration <= 0 {
			return Config{}, fmt.Errorf("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX must be a positive duration")
		}
		cfg.ModelHealthProbeBackoffMax = duration
	}
	if cfg.ModelHealthProbeBackoffMax < cfg.ModelHealthProbeBackoffBase {
		return Config{}, fmt.Errorf("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX must not be shorter than APP_MODEL_HEALTH_PROBE_BACKOFF_BASE")
	}
	if raw := strings.TrimSpace(os.Getenv("APP_RETENTION_CLEANUP_INTERVAL")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration < 0 {
			return Config{}, fmt.Errorf("APP_RETENTION_CLEANUP_INTERVAL must be zero or a positive duration")
		}
		cfg.RetentionCleanupInterval = duration
	}
	if raw := strings.TrimSpace(os.Getenv("APP_RETENTION_CLEANUP_TIMEOUT")); raw != "" {
		duration, err := time.ParseDuration(raw)
		if err != nil || duration <= 0 {
			return Config{}, fmt.Errorf("APP_RETENTION_CLEANUP_TIMEOUT must be a positive duration")
		}
		cfg.RetentionCleanupTimeout = duration
	}
	if raw := strings.TrimSpace(os.Getenv("APP_RETENTION_CLEANUP_BATCH")); raw != "" {
		value, err := parsePositiveIntEnv("APP_RETENTION_CLEANUP_BATCH", raw)
		if err != nil {
			return Config{}, err
		}
		if value > 1000 {
			return Config{}, fmt.Errorf("APP_RETENTION_CLEANUP_BATCH must be at most 1000")
		}
		cfg.RetentionCleanupBatch = value
	}
	for _, setting := range []struct {
		name  string
		value *time.Duration
	}{
		{name: "APP_AUTH_SESSION_RETENTION_TTL", value: &cfg.AuthSessionRetentionTTL},
		{name: "APP_IDEMPOTENCY_RETENTION_TTL", value: &cfg.IdempotencyRetentionTTL},
		{name: "APP_MODEL_TEST_RETENTION_TTL", value: &cfg.ModelTestRetentionTTL},
		{name: "APP_AUDIT_LOG_RETENTION_TTL", value: &cfg.AuditLogRetentionTTL},
	} {
		if raw := strings.TrimSpace(os.Getenv(setting.name)); raw != "" {
			duration, err := time.ParseDuration(raw)
			if err != nil || duration <= 0 {
				return Config{}, fmt.Errorf("%s must be a positive duration", setting.name)
			}
			*setting.value = duration
		}
	}

	return cfg, nil
}

func validateProductionAdminCredentials(username, password string) error {
	username = strings.TrimSpace(username)
	if username == "" && password == "" {
		// An already-bootstrapped PostgreSQL instance may start without the
		// bootstrap secret; main preflights the persisted administrator instead.
		return nil
	}
	if username == "" {
		return fmt.Errorf("APP_ADMIN_USERNAME and APP_ADMIN_PASSWORD must be provided together")
	}
	if password == "" {
		return fmt.Errorf("APP_ADMIN_USERNAME and APP_ADMIN_PASSWORD must be provided together")
	}
	if strings.Contains(username, "REPLACE_WITH_") {
		return fmt.Errorf("APP_ADMIN_USERNAME must be replaced before PostgreSQL startup")
	}
	if len(username) < 3 || len(username) > 64 {
		return fmt.Errorf("APP_ADMIN_USERNAME must contain 3 to 64 characters")
	}
	if len(password) < 12 || len(password) > 256 {
		return fmt.Errorf("APP_ADMIN_PASSWORD must contain 12 to 256 characters for PostgreSQL mode")
	}
	if strings.Contains(password, "REPLACE_WITH_") {
		return fmt.Errorf("APP_ADMIN_PASSWORD must be replaced before PostgreSQL startup")
	}
	return nil
}

// validatePublishConfig makes the external transport contract explicit. The Go
// process may continue to listen on plain HTTP behind a private reverse proxy;
// production clients must still enter through an HTTPS public URL.
func validatePublishConfig(environment, publicBaseURL string, allowInsecureHTTP bool) error {
	switch environment {
	case DeploymentEnvironmentDevelopment, DeploymentEnvironmentStaging, DeploymentEnvironmentProduction:
	default:
		return fmt.Errorf("APP_DEPLOYMENT_ENV must be %q, %q or %q", DeploymentEnvironmentDevelopment, DeploymentEnvironmentStaging, DeploymentEnvironmentProduction)
	}

	if allowInsecureHTTP && environment == DeploymentEnvironmentProduction {
		return fmt.Errorf("APP_ALLOW_INSECURE_HTTP must be false in production")
	}
	if publicBaseURL == "" {
		if environment == DeploymentEnvironmentProduction {
			return fmt.Errorf("APP_PUBLIC_BASE_URL must be set to an https URL in production")
		}
		return nil
	}

	parsed, err := url.Parse(publicBaseURL)
	if err != nil || parsed.Scheme == "" || parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return fmt.Errorf("APP_PUBLIC_BASE_URL must be an absolute http(s) URL without credentials, query or fragment")
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return fmt.Errorf("APP_PUBLIC_BASE_URL must use http or https")
	}
	if environment == DeploymentEnvironmentProduction && parsed.Scheme != "https" {
		return fmt.Errorf("APP_PUBLIC_BASE_URL must use https in production")
	}
	if parsed.Scheme == "http" && !allowInsecureHTTP && environment != DeploymentEnvironmentDevelopment {
		return fmt.Errorf("APP_ALLOW_INSECURE_HTTP=true is required for a non-production http public URL")
	}
	return nil
}

func parseBoolEnv(key string) (bool, error) {
	raw := strings.TrimSpace(os.Getenv(key))
	if raw == "" {
		return false, nil
	}
	switch strings.ToLower(raw) {
	case "1", "true", "yes", "on":
		return true, nil
	case "0", "false", "no", "off":
		return false, nil
	default:
		return false, fmt.Errorf("%s must be true or false", key)
	}
}

func parsePositiveIntEnv(key, raw string) (int, error) {
	value, err := strconv.Atoi(raw)
	if err != nil || value < 1 {
		return 0, fmt.Errorf("%s must be a positive integer", key)
	}
	return value, nil
}

func valueOrDefault(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}
