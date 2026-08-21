package httpapi

import (
	"net"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/time/rate"
)

type rateLimitPolicy struct {
	name  string
	limit rate.Limit
	burst int
	retry time.Duration
}

var protectedRateLimitPolicies = []rateLimitPolicy{
	{name: "auth-login", limit: rate.Every(2 * time.Second), burst: 5, retry: 2 * time.Second},
	{name: "auth-refresh", limit: rate.Every(time.Second), burst: 10, retry: time.Second},
	{name: "model-test", limit: rate.Every(2 * time.Second), burst: 5, retry: 2 * time.Second},
	{name: "model-rotate", limit: rate.Every(2 * time.Second), burst: 5, retry: 2 * time.Second},
	{name: "admin-password-change", limit: rate.Every(30 * time.Second), burst: 2, retry: 30 * time.Second},
}

type requestRateLimiter struct {
	mu        sync.Mutex
	now       func() time.Time
	entries   map[string]rateLimitEntry
	maxKeys   int
	idleAfter time.Duration
}

type rateLimitEntry struct {
	limiter  *rate.Limiter
	lastSeen time.Time
}

func newRequestRateLimiter(now func() time.Time) *requestRateLimiter {
	if now == nil {
		now = time.Now
	}
	return &requestRateLimiter{
		now:       now,
		entries:   make(map[string]rateLimitEntry),
		maxKeys:   4096,
		idleAfter: 10 * time.Minute,
	}
}

func (l *requestRateLimiter) allow(key string, policy rateLimitPolicy) bool {
	now := l.now()
	l.mu.Lock()
	defer l.mu.Unlock()
	for existingKey, entry := range l.entries {
		if now.Sub(entry.lastSeen) > l.idleAfter {
			delete(l.entries, existingKey)
		}
	}
	entry, ok := l.entries[key]
	if !ok {
		if len(l.entries) >= l.maxKeys {
			l.evictOldestLocked()
		}
		entry = rateLimitEntry{limiter: rate.NewLimiter(policy.limit, policy.burst)}
	}
	entry.lastSeen = now
	l.entries[key] = entry
	return entry.limiter.AllowN(now, 1)
}

func (l *requestRateLimiter) evictOldestLocked() {
	var oldestKey string
	var oldest time.Time
	for key, entry := range l.entries {
		if oldestKey == "" || entry.lastSeen.Before(oldest) {
			oldestKey = key
			oldest = entry.lastSeen
		}
	}
	if oldestKey != "" {
		delete(l.entries, oldestKey)
	}
}

func rateLimitMiddleware(limiter *requestRateLimiter, next http.Handler) http.Handler {
	return rateLimitMiddlewareWithMetrics(limiter, nil, next)
}

func rateLimitMiddlewareWithMetrics(limiter *requestRateLimiter, metrics *httpMetrics, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		policy, ok := rateLimitPolicyForRequest(r)
		if !ok || limiter == nil {
			next.ServeHTTP(w, r)
			return
		}
		key := policy.name + ":" + requestClientAddress(r)
		if !limiter.allow(key, policy) {
			if metrics != nil {
				metrics.recordRateLimited(policy.name)
			}
			w.Header().Set("Retry-After", formatRetryAfter(policy.retry))
			writeError(w, r, http.StatusTooManyRequests, "RATE_LIMITED", "请求过于频繁，请稍后重试")
			return
		}
		next.ServeHTTP(w, r)
	})
}

func rateLimitPolicyForRequest(r *http.Request) (rateLimitPolicy, bool) {
	if r.Method != http.MethodPost {
		return rateLimitPolicy{}, false
	}
	switch {
	case r.URL.Path == "/api/v1/auth/login":
		return protectedRateLimitPolicies[0], true
	case r.URL.Path == "/api/v1/auth/refresh":
		return protectedRateLimitPolicies[1], true
	case strings.HasPrefix(r.URL.Path, "/api/v1/admin/model-pool/") && strings.HasSuffix(r.URL.Path, "/test"):
		return protectedRateLimitPolicies[2], true
	case strings.HasPrefix(r.URL.Path, "/api/v1/admin/model-pool/") && strings.HasSuffix(r.URL.Path, "/rotate-secret"):
		return protectedRateLimitPolicies[3], true
	case r.URL.Path == "/api/v1/admin/auth/change-password":
		return protectedRateLimitPolicies[4], true
	default:
		return rateLimitPolicy{}, false
	}
}

func requestClientAddress(r *http.Request) string {
	host, _, err := net.SplitHostPort(strings.TrimSpace(r.RemoteAddr))
	if err == nil && host != "" {
		return host
	}
	if value := strings.TrimSpace(r.RemoteAddr); value != "" {
		return value
	}
	return "unknown"
}

func formatRetryAfter(delay time.Duration) string {
	seconds := int(delay.Round(time.Second) / time.Second)
	if seconds < 1 {
		seconds = 1
	}
	return strconv.Itoa(seconds)
}
