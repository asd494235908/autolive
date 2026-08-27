package httpapi

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"time"

	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

type contextKey string

const requestIDKey contextKey = "request_id"

type AuthConfig struct {
	Username            string
	Password            string
	UsePersistedAdmin   bool
	AuthThrottleHMACKey []byte
	TrustedProxyCIDRs   []string
}

func NewRouter(serviceVersion string, logger *slog.Logger) http.Handler {
	return NewRouterWithAuth(serviceVersion, logger, AuthConfig{})
}

func NewRouterWithAuth(serviceVersion string, logger *slog.Logger, authConfig AuthConfig) http.Handler {
	return NewRouterWithRepository(serviceVersion, logger, authConfig, store.NewMemoryStore(time.Now))
}

func NewRouterWithRepository(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository) http.Handler {
	return NewRouterWithRepositoryAndSecretStore(serviceVersion, logger, authConfig, repository, store.NewMemorySecretStore())
}

func NewRouterWithRepositoryAndSecretStore(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository, secretStore store.SecretStore) http.Handler {
	return NewRouterWithRepositoryAndSecretStoreAndSessionStore(serviceVersion, logger, authConfig, repository, secretStore, nil)
}

func NewRouterWithRepositoryAndSecretStoreAndSessionStore(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository, secretStore store.SecretStore, sessionStore store.SessionStore) http.Handler {
	return NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions(serviceVersion, logger, authConfig, repository, secretStore, sessionStore, false)
}

func NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository, secretStore store.SecretStore, sessionStore store.SessionStore, allowInsecureHTTP bool) http.Handler {
	return NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetry(serviceVersion, logger, authConfig, repository, secretStore, sessionStore, allowInsecureHTTP, nil, nil)
}

// NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetry
// keeps the existing router constructor stable while allowing the server to
// share one bounded health telemetry collector with its background scheduler.
func NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetry(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository, secretStore store.SecretStore, sessionStore store.SessionStore, allowInsecureHTTP bool, healthTelemetry service.ModelPoolHealthProbeTelemetry, retentionTelemetry service.RetentionCleanupTelemetry) http.Handler {
	handler, _ := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetryAndRetentionCleaner(serviceVersion, logger, authConfig, repository, secretStore, sessionStore, allowInsecureHTTP, healthTelemetry, retentionTelemetry)
	return handler
}

// NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetryAndRetentionCleaner
// also returns the authenticator's session cleaner so the server can clean
// memory-mode sessions with the same owned retention scheduler.
func NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptionsAndHealthTelemetryAndRetentionCleaner(serviceVersion string, logger *slog.Logger, authConfig AuthConfig, repository store.Repository, secretStore store.SecretStore, sessionStore store.SessionStore, allowInsecureHTTP bool, healthTelemetry service.ModelPoolHealthProbeTelemetry, retentionTelemetry service.RetentionCleanupTelemetry) (http.Handler, store.AuthSessionRetentionCleaner) {
	if logger == nil {
		logger = slog.Default()
	}

	controlPlane := service.NewControlPlaneWithRepositoryAndSecretStoreAndOptions(repository, nil, secretStore, service.ControlPlaneOptions{
		AllowInsecureHTTP: allowInsecureHTTP,
		PasswordUpgradeError: func(err error) {
			logger.Warn("password credential upgrade failed", "error", err)
		},
	})
	var productRepository store.ProductRepository
	if candidate, ok := repository.(store.ProductRepository); ok {
		// Product membership is a normalized-only capability. Snapshot-backed
		// PostgreSQL implements the compatibility methods too, but must keep the
		// legacy login path instead of returning normalized-read errors.
		if source, ok := repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
			productRepository = candidate
		}
	}
	authenticator := newAuthenticatorWithProductRepository(controlPlane, authConfig, productRepository, sessionStore)
	metrics := newHTTPMetricsWithTelemetry(repository, healthTelemetry, retentionTelemetry)

	mux := http.NewServeMux()
	registerAuthRoutes(mux, authenticator)
	registerAdminRBACRoutes(mux, controlPlane, authenticator)
	registerControlPlaneRoutes(mux, controlPlane, authenticator)
	mux.Handle("/api/v1/health", healthHandler(serviceVersion))
	mux.Handle("/api/v1/livez", healthHandler(serviceVersion))
	mux.Handle("/api/v1/readyz", readinessHandler(serviceVersion, func(ctx context.Context) error {
		if authenticator.initErr != nil {
			return authenticator.initErr
		}
		return controlPlane.CheckReady(ctx)
	}))
	mux.Handle("/metrics", metrics.handler())
	mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeError(w, r, http.StatusNotFound, "NOT_FOUND", "请求的资源不存在")
	}))

	clientAddresses, err := newClientAddressResolver(authConfig.TrustedProxyCIDRs)
	if err != nil {
		// Server configuration validates CIDRs before constructing the router. A
		// direct library caller with invalid input must fail closed and ignore all
		// forwarded headers instead of trusting an ambiguous proxy chain.
		logger.Error("trusted proxy configuration rejected", "error", err)
		clientAddresses = clientAddressResolver{}
	}
	var protected http.Handler = mux
	if throttleStore, ok := repository.(store.LoginThrottleStore); ok {
		protected = loginThrottleMiddleware(loginThrottleOptions{
			store:   throttleStore,
			hmacKey: authConfig.AuthThrottleHMACKey,
			clients: clientAddresses,
			now:     time.Now,
			onError: func(err error) {
				logger.Error("persistent login throttle operation failed", "error", err)
			},
		}, protected)
	}
	protected = rateLimitMiddlewareWithMetricsAndClients(newRequestRateLimiter(time.Now), metrics, clientAddresses, protected)

	return requestIDMiddleware(loggingMiddleware(logger, auditMiddlewareWithOptions(controlPlane, logger, metrics, metrics.middleware(protected)))), authenticator
}

func RequestIDFromContext(ctx context.Context) string {
	value, _ := ctx.Value(requestIDKey).(string)
	return value
}

func requestIDMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestID := validatedRequestID(r.Header.Get("X-Request-Id"))
		if requestID == "" {
			requestID = newRequestID()
		}
		ctx := context.WithValue(r.Context(), requestIDKey, requestID)
		w.Header().Set("X-Request-Id", requestID)
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

func validatedRequestID(value string) string {
	value = strings.TrimSpace(value)
	if len(value) < 8 || len(value) > 128 {
		return ""
	}
	for _, char := range value {
		if (char >= 'a' && char <= 'z') || (char >= 'A' && char <= 'Z') || (char >= '0' && char <= '9') || char == '-' || char == '_' {
			continue
		}
		return ""
	}
	return value
}

func loggingMiddleware(logger *slog.Logger, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		startedAt := time.Now()
		recorder := &statusRecorder{ResponseWriter: w, statusCode: http.StatusOK}

		next.ServeHTTP(recorder, r)

		logger.Info("http request completed",
			"method", r.Method,
			"path", r.URL.Path,
			"status", recorder.statusCode,
			"duration_ms", time.Since(startedAt).Milliseconds(),
			"request_id", RequestIDFromContext(r.Context()),
		)
	})
}

type statusRecorder struct {
	http.ResponseWriter
	statusCode int
}

func (r *statusRecorder) WriteHeader(statusCode int) {
	r.statusCode = statusCode
	r.ResponseWriter.WriteHeader(statusCode)
}

func newRequestID() string {
	var buf [12]byte
	if _, err := rand.Read(buf[:]); err == nil {
		return "req_" + hex.EncodeToString(buf[:])
	}

	return fmt.Sprintf("req_%d", time.Now().UnixNano())
}
