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
	defer func() {
		if storage.close != nil {
			_ = storage.close()
		}
	}()

	server := &http.Server{
		Addr: cfg.ListenAddr,
		Handler: httpapi.NewRouterWithRepositoryAndSecretStoreAndSessionStore(cfg.ServiceVersion, logger, httpapi.AuthConfig{
			Username: cfg.AdminUsername,
			Password: cfg.AdminPassword,
		}, storage.repository, storage.secretStore, storage.sessionStore),
	}
	configureHTTPServer(server)

	rootCtx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

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
			logger.Error("http server stopped unexpectedly", "error", serveErr)
			os.Exit(1)
		}
		return
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), cfg.ShutdownTimeout)
	defer cancel()

	if err := server.Shutdown(shutdownCtx); err != nil {
		logger.Error("graceful shutdown failed", "error", err, "timeout", cfg.ShutdownTimeout.String())
		os.Exit(1)
	}

	if serveErr := <-errCh; serveErr != nil {
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
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		if err := database.PingContext(ctx); err != nil {
			_ = closeDatabase()
			return storageRuntime{}, fmt.Errorf("ping postgres: %w", err)
		}
		if err := validateDatabaseVersion(ctx, database); err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		secretStore, err := store.NewEncryptedSQLSecretStore(database, key)
		if err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		repository, err := store.NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, secretStore, cfg.ModelReadSource)
		if err != nil {
			_ = closeDatabase()
			return storageRuntime{}, err
		}
		sessionStore, err := store.NewSQLSessionStore(database, time.Now)
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
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil, errors.New("APP_SECRET_ENCRYPTION_KEY must be set for PostgreSQL mode")
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
		return nil, errors.New("APP_SECRET_ENCRYPTION_KEY must be 32 bytes encoded as 64 hex characters or base64")
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
