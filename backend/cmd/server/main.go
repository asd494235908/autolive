package main

import (
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"autoLive/backend/internal/config"
	"autoLive/backend/internal/httpapi"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
	"autoLive/backend/migrations"

	_ "github.com/lib/pq"
)

func main() {
	cfg, err := config.LoadFromEnv()
	if err != nil {
		slog.New(slog.NewJSONHandler(os.Stderr, nil)).Error("load config failed", "error", err)
		os.Exit(1)
	}

	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{
		Level: slog.LevelInfo,
	}))
	storage, err := openStorage(cfg)
	if err != nil {
		logger.Error("storage initialization failed", "storage_mode", cfg.StorageMode, "error", err)
		os.Exit(1)
	}
	closeStorage := func() {
		if storage.close != nil {
			if closeErr := storage.close(); closeErr != nil {
				logger.Error("storage close failed", "error", closeErr)
			}
		}
	}
	defer closeStorage()
	if cfg.StorageMode == config.StorageModePostgres || (cfg.AdminUsername != "" && cfg.AdminPassword != "") {
		bootstrap := service.NewControlPlaneWithRepositoryAndSecretStoreAndOptions(storage.repository, nil, storage.secretStore, service.ControlPlaneOptions{AllowInsecureHTTP: cfg.AllowInsecureHTTP})
		bootstrapCtx, cancel := context.WithTimeout(context.Background(), cfg.RequestTimeout)
		var bootstrapErr error
		if cfg.AdminUsername != "" || cfg.AdminPassword != "" {
			bootstrapErr = bootstrap.EnsureConfiguredAdmin(bootstrapCtx, cfg.AdminUsername, cfg.AdminPassword)
		} else {
			bootstrapErr = bootstrap.CheckReady(bootstrapCtx)
		}
		cancel()
		if bootstrapErr != nil {
			logger.Error("administrator bootstrap failed", "storage_mode", cfg.StorageMode, "error", bootstrapErr)
			closeStorage()
			os.Exit(1)
		}
	}

	healthTelemetry := httpapi.NewModelPoolHealthProbeMetrics()
	retentionTelemetry := httpapi.NewRetentionCleanupMetrics()
	loginThrottleKey, err := parseOptionalHMACKey(cfg.AuthThrottleHMACKey)
	if err != nil {
		logger.Error("login throttle key configuration failed", "error", err)
		closeStorage()
		os.Exit(1)
	}
	handler, authRetentionCleaner := httpapi.NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetryAndRetentionCleaner(cfg.ServiceVersion, logger, httpapi.AuthConfig{
		Username:            cfg.AdminUsername,
		Password:            cfg.AdminPassword,
		UsePersistedAdmin:   cfg.StorageMode == config.StorageModePostgres && cfg.AdminUsername == "" && cfg.AdminPassword == "",
		AuthThrottleHMACKey: loginThrottleKey,
		TrustedProxyCIDRs:   cfg.TrustedProxyCIDRs,
	}, storage.repository, storage.secretStore, storage.sessionStore, cfg.AllowInsecureHTTP, healthTelemetry, retentionTelemetry)
	healthControlPlane := service.NewControlPlaneWithRepositoryAndSecretStoreAndOptions(storage.repository, nil, storage.secretStore, service.ControlPlaneOptions{AllowInsecureHTTP: cfg.AllowInsecureHTTP})
	healthScheduler, err := service.NewModelPoolHealthProbeScheduler(healthControlPlane, service.ModelPoolHealthProbeSchedulerOptions{
		Interval:          cfg.ModelHealthProbeInterval,
		ProbeTimeout:      cfg.ModelHealthProbeTimeout,
		MaxConcurrent:     cfg.ModelHealthProbeMaxConcurrent,
		MaxAccountsPerRun: cfg.ModelHealthProbeMaxAccounts,
		BackoffBase:       cfg.ModelHealthProbeBackoffBase,
		BackoffMax:        cfg.ModelHealthProbeBackoffMax,
		Logger:            logger,
		Telemetry:         healthTelemetry,
	})
	if err != nil {
		logger.Error("model pool health scheduler initialization failed", "error", err)
		closeStorage()
		os.Exit(1)
	}
	retentionScheduler, err := service.NewRetentionCleanupScheduler(
		asControlPlaneRetentionCleaner(storage.repository),
		authRetentionCleaner,
		service.RetentionCleanupSchedulerOptions{
			Interval:              cfg.RetentionCleanupInterval,
			OperationTimeout:      cfg.RetentionCleanupTimeout,
			BatchSize:             cfg.RetentionCleanupBatch,
			StagedSecretCleaner:   healthControlPlane.CleanupStagedSecrets,
			AuditOutboxDispatcher: healthControlPlane.DispatchAuditOutbox,
			Policy: service.RetentionCleanupPolicy{
				AuthSessionTTL:       cfg.AuthSessionRetentionTTL,
				AuthThrottleTTL:      cfg.AuthThrottleRetentionTTL,
				IdempotencyRecordTTL: cfg.IdempotencyRetentionTTL,
				ModelTestResultTTL:   cfg.ModelTestRetentionTTL,
				AuditLogTTL:          cfg.AuditLogRetentionTTL,
			},
			Logger:    logger,
			Telemetry: retentionTelemetry,
		})
	if err != nil {
		logger.Error("retention cleanup scheduler initialization failed", "error", err)
		closeStorage()
		os.Exit(1)
	}
	server := &http.Server{
		Addr:    cfg.ListenAddr,
		Handler: httpapi.WithRequestTimeout(handler, cfg.RequestTimeout),
	}
	configureHTTPServer(server)

	rootCtx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if cfg.ModelHealthProbeInterval > 0 {
		if err := healthScheduler.Start(rootCtx); err != nil {
			logger.Error("model pool health scheduler start failed", "error", err)
			closeStorage()
			os.Exit(1)
		}
	}
	if cfg.RetentionCleanupInterval > 0 {
		if err := retentionScheduler.Start(rootCtx); err != nil {
			_ = healthScheduler.Stop(context.Background())
			logger.Error("retention cleanup scheduler start failed", "error", err)
			closeStorage()
			os.Exit(1)
		}
	}

	errCh := make(chan error, 1)
	go func() {
		logger.Info("http server listening", "addr", cfg.ListenAddr, "version", cfg.ServiceVersion)
		if serveErr := server.ListenAndServe(); serveErr != nil && !errors.Is(serveErr, http.ErrServerClosed) {
			errCh <- serveErr
			return
		}
		errCh <- nil
	}()

	select {
	case <-rootCtx.Done():
		logger.Info("shutdown signal received")
	case serveErr := <-errCh:
		if serveErr != nil {
			_ = healthScheduler.Stop(context.Background())
			_ = retentionScheduler.Stop(context.Background())
			closeStorage()
			logger.Error("http server stopped unexpectedly", "error", serveErr)
			os.Exit(1)
		}
		return
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), cfg.ShutdownTimeout)
	defer cancel()

	if err := server.Shutdown(shutdownCtx); err != nil {
		_ = healthScheduler.Stop(shutdownCtx)
		_ = retentionScheduler.Stop(shutdownCtx)
		closeStorage()
		logger.Error("graceful shutdown failed", "error", err, "timeout", cfg.ShutdownTimeout.String())
		os.Exit(1)
	}
	healthStopErr := healthScheduler.Stop(shutdownCtx)
	retentionStopErr := retentionScheduler.Stop(shutdownCtx)
	if healthStopErr != nil {
		logger.Error("model pool health scheduler shutdown failed", "error", healthStopErr, "timeout", cfg.ShutdownTimeout.String())
	}
	if retentionStopErr != nil {
		logger.Error("retention cleanup scheduler shutdown failed", "error", retentionStopErr, "timeout", cfg.ShutdownTimeout.String())
	}
	if healthStopErr != nil || retentionStopErr != nil {
		closeStorage()
		os.Exit(1)
	}

	if serveErr := <-errCh; serveErr != nil {
		closeStorage()
		logger.Error("http server stopped with error", "error", serveErr)
		os.Exit(1)
	}

	logger.Info("http server stopped cleanly")
}

func configureHTTPServer(server *http.Server) *http.Server {
	server.ReadHeaderTimeout = 5 * time.Second
	server.ReadTimeout = 30 * time.Second
	server.WriteTimeout = 60 * time.Second
	server.IdleTimeout = 120 * time.Second
	return server
}

type storageRuntime struct {
	repository   store.Repository
	secretStore  store.SecretStore
	sessionStore store.SessionStore
	close        func() error
}

func asControlPlaneRetentionCleaner(repository store.Repository) store.ControlPlaneRetentionCleaner {
	cleaner, _ := repository.(store.ControlPlaneRetentionCleaner)
	return cleaner
}

func openStorage(cfg config.Config) (storageRuntime, error) {
	switch cfg.StorageMode {
	case config.StorageModeMemory:
		return storageRuntime{
			repository:  store.NewMemoryStore(time.Now),
			secretStore: store.NewMemorySecretStore(),
		}, nil
	case config.StorageModePostgres:
		key, err := parseSecretEncryptionKey(cfg.SecretEncryptionKey)
		if err != nil {
			return storageRuntime{}, err
		}
		database, err := sql.Open("postgres", cfg.DatabaseURL)
		if err != nil {
			return storageRuntime{}, fmt.Errorf("open postgres: %w", err)
		}
		database.SetMaxOpenConns(10)
		database.SetMaxIdleConns(5)
		database.SetConnMaxLifetime(30 * time.Minute)
		closeDatabase := func() error { return database.Close() }
		ctx, cancel := context.WithTimeout(context.Background(), cfg.RequestTimeout)
		defer cancel()
		if err := database.PingContext(ctx); err != nil {
			_ = closeDatabase()
			return storageRuntime{}, fmt.Errorf("ping postgres: %w", err)
		}
		if err := validateDatabaseVersion(ctx, database); err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		secretStore, err := store.NewEncryptedSQLSecretStoreWithTimeout(database, key, cfg.RequestTimeout)
		if err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		repository, err := store.NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, time.Now, secretStore, cfg.ModelReadSource, cfg.RequestTimeout)
		if err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		sessionStore, err := store.NewSQLSessionStoreWithTimeout(database, time.Now, cfg.RequestTimeout)
		if err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		return storageRuntime{repository: repository, secretStore: secretStore, sessionStore: sessionStore, close: closeDatabase}, nil
	default:
		return storageRuntime{}, fmt.Errorf("storage mode %q is not supported; refusing to fall back to memory", cfg.StorageMode)
	}
}

func parseSecretEncryptionKey(raw string) ([]byte, error) {
	return parseRequired32ByteKey(raw, "APP_SECRET_ENCRYPTION_KEY", " for PostgreSQL mode")
}

func parseOptionalHMACKey(raw string) ([]byte, error) {
	if strings.TrimSpace(raw) == "" {
		return nil, nil
	}
	raw = strings.TrimSpace(raw)
	var key []byte
	var err error
	if len(raw)%2 == 0 {
		key, err = hex.DecodeString(raw)
	}
	if err != nil || len(key) == 0 {
		key, err = base64.RawStdEncoding.DecodeString(raw)
		if err != nil {
			key, err = base64.StdEncoding.DecodeString(raw)
		}
	}
	if err != nil || len(key) < 32 || len(key) > 128 {
		return nil, errors.New("APP_AUTH_THROTTLE_HMAC_KEY must be 32 to 128 bytes encoded as hex or base64")
	}
	return key, nil
}

func parseRequired32ByteKey(raw, name, requiredContext string) ([]byte, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil, fmt.Errorf("%s must be set%s", name, requiredContext)
	}
	if len(raw) == 64 {
		key, err := hex.DecodeString(raw)
		if err == nil && len(key) == 32 {
			return key, nil
		}
	}
	key, err := base64.RawStdEncoding.DecodeString(raw)
	if err != nil {
		key, err = base64.StdEncoding.DecodeString(raw)
	}
	if err != nil || len(key) != 32 {
		return nil, fmt.Errorf("%s must be 32 bytes encoded as 64 hex characters or base64", name)
	}
	return key, nil
}

func validateDatabaseVersion(ctx context.Context, database *sql.DB) error {
	var version int64
	var dirty bool
	err := database.QueryRowContext(ctx, `
		SELECT version, dirty
		FROM schema_migrations
		ORDER BY version DESC
		LIMIT 1
	`).Scan(&version, &dirty)
	if errors.Is(err, sql.ErrNoRows) {
		return errors.New("database has no applied migrations; run cmd/migrate before starting the API")
	}
	if err != nil {
		return fmt.Errorf("check database migration version: %w", err)
	}
	return validateAppliedMigration(version, dirty)
}

func validateAppliedMigration(version int64, dirty bool) error {
	if dirty {
		return fmt.Errorf("database migration version %d is dirty", version)
	}
	if version != migrations.LatestVersion {
		return fmt.Errorf("database migration version %d does not match application version %d", version, migrations.LatestVersion)
	}
	return nil
}
