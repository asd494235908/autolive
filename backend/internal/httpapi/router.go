package httpapi

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log/slog"
	"net/http"
	"time"

	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

type contextKey string

const requestIDKey contextKey = "request_id"

type AuthConfig struct {
	Username string
	Password string
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
	if logger == nil {
		logger = slog.Default()
	}

	controlPlane := service.NewControlPlaneWithRepositoryAndSecretStore(repository, nil, secretStore)
	controlPlane.EnsureLocalAdmin(authConfig.Username)
	authenticator := newAuthenticator(controlPlane, authConfig, sessionStore)

	mux := http.NewServeMux()
	registerAuthRoutes(mux, authenticator)
	registerControlPlaneRoutes(mux, controlPlane, authenticator)
	mux.Handle("/api/v1/health", healthHandler(serviceVersion))
	mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeError(w, r, http.StatusNotFound, "NOT_FOUND", "请求的资源不存在")
	}))

	return requestIDMiddleware(loggingMiddleware(logger, auditMiddleware(controlPlane, mux)))
}

func RequestIDFromContext(ctx context.Context) string {
	value, _ := ctx.Value(requestIDKey).(string)
	return value
}

func requestIDMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestID := newRequestID()
		ctx := context.WithValue(r.Context(), requestIDKey, requestID)
		w.Header().Set("X-Request-Id", requestID)
		next.ServeHTTP(w, r.WithContext(ctx))
	})
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
