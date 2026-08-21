package httpapi

import (
	"context"
	"net/http"
	"time"
)

type HealthResponse struct {
	Status    string `json:"status"`
	Service   string `json:"service"`
	Version   string `json:"version"`
	Now       string `json:"now"`
	RequestID string `json:"request_id"`
}

func healthHandler(serviceVersion string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			writeError(w, r, http.StatusMethodNotAllowed, "METHOD_NOT_ALLOWED", "请求方法不被允许")
			return
		}

		writeJSON(w, http.StatusOK, HealthResponse{
			Status:    "ok",
			Service:   "autolive-control-plane",
			Version:   serviceVersion,
			Now:       time.Now().UTC().Format(time.RFC3339),
			RequestID: RequestIDFromContext(r.Context()),
		})
	})
}

func readinessHandler(serviceVersion string, check func(context.Context) error) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			writeError(w, r, http.StatusMethodNotAllowed, "METHOD_NOT_ALLOWED", "请求方法不被允许")
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		if err := check(ctx); err != nil {
			writeJSON(w, http.StatusServiceUnavailable, HealthResponse{
				Status:    "degraded",
				Service:   "autolive-control-plane",
				Version:   serviceVersion,
				Now:       time.Now().UTC().Format(time.RFC3339),
				RequestID: RequestIDFromContext(r.Context()),
			})
			return
		}
		writeJSON(w, http.StatusOK, HealthResponse{
			Status:    "ok",
			Service:   "autolive-control-plane",
			Version:   serviceVersion,
			Now:       time.Now().UTC().Format(time.RFC3339),
			RequestID: RequestIDFromContext(r.Context()),
		})
	})
}
