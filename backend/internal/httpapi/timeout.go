package httpapi

import (
	"context"
	"net/http"
	"time"
)

// WithRequestTimeout bounds the request context without replacing the
// structured JSON response path owned by the wrapped handler.
func WithRequestTimeout(next http.Handler, timeout time.Duration) http.Handler {
	if next == nil || timeout <= 0 {
		return next
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithTimeout(r.Context(), timeout)
		defer cancel()
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}
