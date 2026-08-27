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
	t.Setenv("APP_MIGRATION_METRICS_FILE", "")
	t.Setenv("APP_SECRET_ENCRYPTION_KEY", "")
	t.Setenv("APP_AUTH_THROTTLE_HMAC_KEY", "")
	t.Setenv("APP_TRUSTED_PROXY_CIDRS", "")
	t.Setenv("APP_MODEL_READ_SOURCE", "")
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "")
	t.Setenv("APP_REQUEST_TIMEOUT", "")
	t.Setenv("APP_MIGRATION_TIMEOUT", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_INTERVAL", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_TIMEOUT", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_ACCOUNTS", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_BACKOFF_BASE", "")
	t.Setenv("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX", "")
	t.Setenv("APP_RETENTION_CLEANUP_INTERVAL", "")
	t.Setenv("APP_RETENTION_CLEANUP_TIMEOUT", "")
	t.Setenv("APP_RETENTION_CLEANUP_BATCH", "")
	t.Setenv("APP_AUTH_SESSION_RETENTION_TTL", "")
	t.Setenv("APP_AUTH_THROTTLE_RETENTION_TTL", "")
	t.Setenv("APP_IDEMPOTENCY_RETENTION_TTL", "")
	t.Setenv("APP_MODEL_TEST_RETENTION_TTL", "")
	t.Setenv("APP_AUDIT_LOG_RETENTION_TTL", "")
	t.Setenv("APP_DEPLOYMENT_ENV", "")
	t.Setenv("APP_PUBLIC_BASE_URL", "")
	t.Setenv("APP_ALLOW_INSECURE_HTTP", "")

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
	if cfg.RequestTimeout != defaultRequestTimeout {
		t.Fatalf("RequestTimeout = %v, want %v", cfg.RequestTimeout, defaultRequestTimeout)
	}
	if cfg.MigrationTimeout != defaultMigrationTimeout {
		t.Fatalf("MigrationTimeout = %v, want %v", cfg.MigrationTimeout, defaultMigrationTimeout)
	}
	if cfg.MigrationMetricsFile != "" {
		t.Fatalf("MigrationMetricsFile = %q, want empty", cfg.MigrationMetricsFile)
	}
	if cfg.ModelHealthProbeInterval != defaultModelHealthProbeInterval || cfg.ModelHealthProbeTimeout != defaultModelHealthProbeTimeout || cfg.ModelHealthProbeMaxConcurrent != defaultModelHealthProbeMaxConcurrent || cfg.ModelHealthProbeMaxAccounts != defaultModelHealthProbeMaxAccounts {
		t.Fatalf("health probe defaults = interval %v timeout %v concurrency %d accounts %d", cfg.ModelHealthProbeInterval, cfg.ModelHealthProbeTimeout, cfg.ModelHealthProbeMaxConcurrent, cfg.ModelHealthProbeMaxAccounts)
	}
	if cfg.RetentionCleanupInterval != defaultRetentionCleanupInterval || cfg.RetentionCleanupTimeout != defaultRetentionCleanupTimeout || cfg.RetentionCleanupBatch != defaultRetentionCleanupBatch || cfg.AuthSessionRetentionTTL != defaultAuthSessionRetentionTTL || cfg.AuthThrottleRetentionTTL != defaultAuthThrottleRetentionTTL || cfg.IdempotencyRetentionTTL != defaultIdempotencyRetentionTTL || cfg.ModelTestRetentionTTL != defaultModelTestRetentionTTL || cfg.AuditLogRetentionTTL != defaultAuditLogRetentionTTL {
		t.Fatalf("retention defaults = %+v", cfg)
	}
	if cfg.DeploymentEnvironment != DeploymentEnvironmentDevelopment {
		t.Fatalf("DeploymentEnvironment = %q, want %q", cfg.DeploymentEnvironment, DeploymentEnvironmentDevelopment)
	}
	if cfg.AllowInsecureHTTP {
		t.Fatal("AllowInsecureHTTP = true, want false by default")
	}
}

func TestLoadFromEnvParsesSemanticValues(t *testing.T) {
	t.Setenv("APP_ADDR", "127.0.0.1:9090")
	t.Setenv("APP_VERSION", "phase1")
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "3s")
	t.Setenv("APP_MODEL_HEALTH_PROBE_INTERVAL", "2m")
	t.Setenv("APP_MODEL_HEALTH_PROBE_TIMEOUT", "7s")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", "3")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_ACCOUNTS", "9")
	t.Setenv("APP_RETENTION_CLEANUP_INTERVAL", "2h")
	t.Setenv("APP_RETENTION_CLEANUP_TIMEOUT", "11s")
	t.Setenv("APP_RETENTION_CLEANUP_BATCH", "17")
	t.Setenv("APP_AUTH_SESSION_RETENTION_TTL", "10h")
	t.Setenv("APP_AUTH_THROTTLE_RETENTION_TTL", "9h")
	t.Setenv("APP_IDEMPOTENCY_RETENTION_TTL", "11h")
	t.Setenv("APP_MODEL_TEST_RETENTION_TTL", "12h")
	t.Setenv("APP_AUDIT_LOG_RETENTION_TTL", "13h")

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
	if cfg.ModelHealthProbeInterval != 2*time.Minute || cfg.ModelHealthProbeTimeout != 7*time.Second || cfg.ModelHealthProbeMaxConcurrent != 3 || cfg.ModelHealthProbeMaxAccounts != 9 {
		t.Fatalf("health probe config = interval %v timeout %v concurrency %d accounts %d", cfg.ModelHealthProbeInterval, cfg.ModelHealthProbeTimeout, cfg.ModelHealthProbeMaxConcurrent, cfg.ModelHealthProbeMaxAccounts)
	}
	if cfg.RetentionCleanupInterval != 2*time.Hour || cfg.RetentionCleanupTimeout != 11*time.Second || cfg.RetentionCleanupBatch != 17 || cfg.AuthSessionRetentionTTL != 10*time.Hour || cfg.AuthThrottleRetentionTTL != 9*time.Hour || cfg.IdempotencyRetentionTTL != 11*time.Hour || cfg.ModelTestRetentionTTL != 12*time.Hour || cfg.AuditLogRetentionTTL != 13*time.Hour {
		t.Fatalf("retention config = %+v", cfg)
	}
}

func TestLoadFromEnvValidatesRetentionBudget(t *testing.T) {
	t.Setenv("APP_RETENTION_CLEANUP_INTERVAL", "-1s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("negative retention interval accepted")
	}
	t.Setenv("APP_RETENTION_CLEANUP_INTERVAL", "0")
	t.Setenv("APP_RETENTION_CLEANUP_TIMEOUT", "0s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("zero retention timeout accepted")
	}
	t.Setenv("APP_RETENTION_CLEANUP_TIMEOUT", "1s")
	t.Setenv("APP_RETENTION_CLEANUP_BATCH", "1001")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("oversized retention batch accepted")
	}
	t.Setenv("APP_RETENTION_CLEANUP_BATCH", "1")
	t.Setenv("APP_AUDIT_LOG_RETENTION_TTL", "0s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("zero audit retention TTL accepted")
	}
}

func TestLoadFromEnvRejectsInvalidTimeout(t *testing.T) {
	t.Setenv("APP_SHUTDOWN_TIMEOUT", "not-a-duration")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want error")
	}
}

func TestLoadFromEnvParsesAndValidatesRequestTimeout(t *testing.T) {
	t.Setenv("APP_REQUEST_TIMEOUT", "4s")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}
	if cfg.RequestTimeout != 4*time.Second {
		t.Fatalf("RequestTimeout = %v, want 4s", cfg.RequestTimeout)
	}

	t.Setenv("APP_REQUEST_TIMEOUT", "0s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want non-positive request timeout error")
	}
}

func TestLoadFromEnvParsesAndValidatesMigrationTimeout(t *testing.T) {
	t.Setenv("APP_MIGRATION_TIMEOUT", "45s")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}
	if cfg.MigrationTimeout != 45*time.Second {
		t.Fatalf("MigrationTimeout = %v, want 45s", cfg.MigrationTimeout)
	}

	t.Setenv("APP_MIGRATION_TIMEOUT", "0s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want non-positive migration timeout error")
	}
	t.Setenv("APP_MIGRATION_TIMEOUT", "not-a-duration")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want invalid migration timeout error")
	}
}

func TestLoadFromEnvParsesMigrationMetricsFile(t *testing.T) {
	t.Setenv("APP_MIGRATION_METRICS_FILE", "/var/lib/autolive/metrics/migration.prom")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}
	if cfg.MigrationMetricsFile != "/var/lib/autolive/metrics/migration.prom" {
		t.Fatalf("MigrationMetricsFile = %q", cfg.MigrationMetricsFile)
	}

	t.Setenv("APP_MIGRATION_METRICS_FILE", "  ")
	cfg, err = LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() with blank metrics file error = %v", err)
	}
	if cfg.MigrationMetricsFile != "" {
		t.Fatalf("blank MigrationMetricsFile = %q, want empty", cfg.MigrationMetricsFile)
	}
}

func TestLoadFromEnvValidatesHealthProbeBudget(t *testing.T) {
	t.Setenv("APP_MODEL_HEALTH_PROBE_INTERVAL", "0")
	t.Setenv("APP_MODEL_HEALTH_PROBE_TIMEOUT", "5s")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", "4")
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_ACCOUNTS", "20")
	t.Setenv("APP_MODEL_HEALTH_PROBE_BACKOFF_BASE", "20s")
	t.Setenv("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX", "2m")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v", err)
	}
	if cfg.ModelHealthProbeInterval != 0 || cfg.ModelHealthProbeBackoffBase != 20*time.Second || cfg.ModelHealthProbeBackoffMax != 2*time.Minute {
		t.Fatalf("health probe budget = %+v", cfg)
	}

	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", "0")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want invalid health probe concurrency")
	}
	t.Setenv("APP_MODEL_HEALTH_PROBE_MAX_CONCURRENT", "4")
	t.Setenv("APP_MODEL_HEALTH_PROBE_BACKOFF_MAX", "1s")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want backoff max/base ordering error")
	}
}

func TestLoadFromEnvRequiresDatabaseURLForPostgres(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_DATABASE_URL", "")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want missing database URL error")
	}
}

func TestLoadFromEnvAllowsPersistedProductionAdmin(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	t.Setenv("APP_ADMIN_USERNAME", "")
	t.Setenv("APP_ADMIN_PASSWORD", "")
	if _, err := LoadFromEnv(); err != nil {
		t.Fatalf("LoadFromEnv() error = %v, want persisted-admin configuration to be accepted", err)
	}

	t.Setenv("APP_ADMIN_USERNAME", "admin")
	t.Setenv("APP_ADMIN_PASSWORD", "")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want paired production admin credentials")
	}

	t.Setenv("APP_ADMIN_USERNAME", "")
	t.Setenv("APP_ADMIN_PASSWORD", "long-enough-password")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want paired production admin credentials")
	}

	t.Setenv("APP_ADMIN_USERNAME", "admin")
	t.Setenv("APP_ADMIN_PASSWORD", "short")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want short production admin password error")
	}
}

func TestLoadFromEnvRejectsProductionPlaceholders(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	t.Setenv("APP_ADMIN_USERNAME", "REPLACE_WITH_ADMIN_USERNAME")
	t.Setenv("APP_ADMIN_PASSWORD", "REPLACE_WITH_ADMIN_PASSWORD")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want placeholder rejection")
	}
}

func TestLoadFromEnvRejectsUnknownStorageMode(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", "sqlite")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want unknown storage mode error")
	}
}

func TestLoadFromEnvRequiresHTTPSPublicURLInProduction(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_MODEL_READ_SOURCE", ModelReadSourceNormalized)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	t.Setenv("APP_DEPLOYMENT_ENV", DeploymentEnvironmentProduction)
	t.Setenv("APP_PUBLIC_BASE_URL", "")
	t.Setenv("APP_AUTH_THROTTLE_HMAC_KEY", "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want missing production public URL error")
	}

	t.Setenv("APP_PUBLIC_BASE_URL", "http://admin.example.com")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want production HTTP URL error")
	}

	t.Setenv("APP_PUBLIC_BASE_URL", "https://admin.example.com")
	cfg, err := LoadFromEnv()
	if err != nil {
		t.Fatalf("LoadFromEnv() error = %v, want HTTPS production configuration to pass", err)
	}
	if cfg.PublicBaseURL != "https://admin.example.com" {
		t.Fatalf("PublicBaseURL = %q", cfg.PublicBaseURL)
	}
}

func TestLoadFromEnvValidatesTrustedProxyCIDRs(t *testing.T) {
	t.Setenv("APP_TRUSTED_PROXY_CIDRS", "10.0.0.0/8, 2001:db8::/32")
	cfg, err := LoadFromEnv()
	if err != nil || len(cfg.TrustedProxyCIDRs) != 2 {
		t.Fatalf("trusted proxy config = %+v, error %v", cfg.TrustedProxyCIDRs, err)
	}
	t.Setenv("APP_TRUSTED_PROXY_CIDRS", "not-a-cidr")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("invalid trusted proxy CIDR accepted")
	}
}

func TestLoadFromEnvRejectsSnapshotModelSourceInProduction(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_MODEL_READ_SOURCE", ModelReadSourceSnapshot)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	t.Setenv("APP_DEPLOYMENT_ENV", DeploymentEnvironmentProduction)
	t.Setenv("APP_PUBLIC_BASE_URL", "https://admin.example.com")

	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want production snapshot rejection")
	}
}

func TestLoadFromEnvRequiresExplicitInsecureFlagOutsideDevelopment(t *testing.T) {
	t.Setenv("APP_DEPLOYMENT_ENV", DeploymentEnvironmentStaging)
	t.Setenv("APP_PUBLIC_BASE_URL", "http://admin.example.com")
	t.Setenv("APP_ALLOW_INSECURE_HTTP", "")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want staging HTTP URL error")
	}

	t.Setenv("APP_ALLOW_INSECURE_HTTP", "true")
	if _, err := LoadFromEnv(); err != nil {
		t.Fatalf("LoadFromEnv() error = %v, want explicit staging HTTP opt-in", err)
	}

	t.Setenv("APP_DEPLOYMENT_ENV", DeploymentEnvironmentProduction)
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want production insecure HTTP rejection")
	}
}

func TestLoadFromEnvRejectsInvalidPublishURLAndBoolean(t *testing.T) {
	t.Setenv("APP_PUBLIC_BASE_URL", "https://user:password@example.com")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want URL credentials rejection")
	}

	t.Setenv("APP_PUBLIC_BASE_URL", "https://example.com/path?secret=1")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want URL query rejection")
	}

	t.Setenv("APP_PUBLIC_BASE_URL", "")
	t.Setenv("APP_ALLOW_INSECURE_HTTP", "sometimes")
	if _, err := LoadFromEnv(); err == nil {
		t.Fatal("LoadFromEnv() error = nil, want invalid boolean rejection")
	}
}

func TestLoadFromEnvAllowsNormalizedModelSourceForPostgres(t *testing.T) {
	t.Setenv("APP_STORAGE_MODE", StorageModePostgres)
	t.Setenv("APP_MODEL_READ_SOURCE", ModelReadSourceNormalized)
	t.Setenv("APP_DATABASE_URL", "postgres://example")
	if _, err := LoadFromEnv(); err != nil {
		t.Fatalf("LoadFromEnv() error = %v, want normalized source to be accepted", err)
	}
}
