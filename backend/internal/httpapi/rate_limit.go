package httpapi

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/netip"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/time/rate"

	"autoLive/backend/internal/store"
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

type clientAddressResolver struct {
	trusted []netip.Prefix
}

type loginThrottleDigests struct {
	Username string
	Address  string
}

type loginThrottleOptions struct {
	store   store.LoginThrottleStore
	hmacKey []byte
	clients clientAddressResolver
	now     func() time.Time
	onError func(error)
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
	return rateLimitMiddlewareWithMetricsAndClients(limiter, metrics, clientAddressResolver{}, next)
}

func rateLimitMiddlewareWithMetricsAndClients(limiter *requestRateLimiter, metrics *httpMetrics, clients clientAddressResolver, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		policy, ok := rateLimitPolicyForRequest(r)
		if !ok || limiter == nil {
			next.ServeHTTP(w, r)
			return
		}
		key := policy.name + ":" + clients.clientAddress(r)
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

// loginThrottleMiddleware adds the cross-instance failure buckets. It should
// wrap the login handlers inside the existing short-burst process limiter.
func loginThrottleMiddleware(options loginThrottleOptions, next http.Handler) http.Handler {
	if options.now == nil {
		options.now = time.Now
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if options.store == nil || len(options.hmacKey) < 32 || r.Method != http.MethodPost || !isLoginPath(r.URL.Path) {
			next.ServeHTTP(w, r)
			return
		}
		username, hasUsername := loginUsername(r)
		clientAddress := options.clients.clientAddress(r)
		address := store.LoginThrottleBucket{Type: store.LoginThrottleBucketAddress, Hash: hmacDigest(options.hmacKey, "address\x00"+clientAddress)}
		buckets := []store.LoginThrottleBucket{address}
		var account store.LoginThrottleBucket
		if hasUsername {
			digests, err := loginThrottleDigest(options.hmacKey, username, clientAddress)
			if err != nil {
				options.reportError(err)
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_INITIALIZATION_UNAVAILABLE", "登录保护暂时不可用")
				return
			}
			account = store.LoginThrottleBucket{Type: store.LoginThrottleBucketAccount, Hash: digests.Username}
			address.Hash = digests.Address
			buckets[0] = address
			buckets = append(buckets, account)
		}
		now := options.now().UTC()
		retryAfter, err := options.store.CheckLoginThrottle(r.Context(), buckets, now)
		if err != nil {
			options.reportError(err)
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_INITIALIZATION_UNAVAILABLE", "登录保护暂时不可用")
			return
		}
		if retryAfter > 0 {
			w.Header().Set("Retry-After", formatRetryAfter(retryAfter))
			writeError(w, r, http.StatusTooManyRequests, "RATE_LIMITED", "请求过于频繁，请稍后重试")
			return
		}
		recorder := &loginThrottleStatusRecorder{ResponseWriter: w, statusCode: http.StatusOK}
		next.ServeHTTP(recorder, r)
		switch recorder.statusCode {
		case http.StatusOK:
			// A successful account login clears only that account's failures. The
			// source-address bucket remains intact so one valid account cannot
			// erase attacks against other accounts from the same source.
			if hasUsername {
				writeCtx, cancel := throttleWriteContext(r.Context())
				if err := options.store.ResetLoginFailures(writeCtx, []store.LoginThrottleBucket{account}); err != nil {
					options.reportError(err)
				}
				cancel()
			}
		case http.StatusUnauthorized, http.StatusForbidden:
			writeCtx, cancel := throttleWriteContext(r.Context())
			if err := options.store.RecordLoginFailure(writeCtx, buckets, now); err != nil {
				options.reportError(err)
			}
			cancel()
		}
	})
}

func (o loginThrottleOptions) reportError(err error) {
	if err != nil && o.onError != nil {
		o.onError(err)
	}
}

type loginThrottleStatusRecorder struct {
	http.ResponseWriter
	statusCode int
}

func (r *loginThrottleStatusRecorder) WriteHeader(statusCode int) {
	r.statusCode = statusCode
	r.ResponseWriter.WriteHeader(statusCode)
}

func throttleWriteContext(parent context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(context.WithoutCancel(parent), 2*time.Second)
}

func isLoginPath(path string) bool {
	return path == "/api/v1/auth/login" || path == "/api/v1/client/auth/login"
}

func loginUsername(r *http.Request) (string, bool) {
	if r.Body == nil {
		return "", false
	}
	payload, err := io.ReadAll(io.LimitReader(r.Body, 4097))
	if err != nil {
		return "", false
	}
	r.Body = io.NopCloser(bytes.NewReader(payload))
	if len(payload) > 4096 {
		return "", false
	}
	var request struct {
		Username string `json:"username"`
	}
	if err := json.Unmarshal(payload, &request); err != nil || strings.TrimSpace(request.Username) == "" {
		return "", false
	}
	return strings.TrimSpace(request.Username), true
}

func loginThrottleDigest(key []byte, username, address string) (loginThrottleDigests, error) {
	if len(key) < 32 || strings.TrimSpace(username) == "" || strings.TrimSpace(address) == "" {
		return loginThrottleDigests{}, errors.New("login throttle digest arguments are invalid")
	}
	return loginThrottleDigests{
		Username: hmacDigest(key, "account\x00"+strings.TrimSpace(username)),
		Address:  hmacDigest(key, "address\x00"+strings.TrimSpace(address)),
	}, nil
}

func hmacDigest(key []byte, value string) string {
	digest := hmac.New(sha256.New, key)
	_, _ = digest.Write([]byte(value))
	return hex.EncodeToString(digest.Sum(nil))
}

func newClientAddressResolver(cidrs []string) (clientAddressResolver, error) {
	resolver := clientAddressResolver{trusted: make([]netip.Prefix, 0, len(cidrs))}
	for _, raw := range cidrs {
		raw = strings.TrimSpace(raw)
		if raw == "" {
			continue
		}
		prefix, err := netip.ParsePrefix(raw)
		if err != nil {
			return clientAddressResolver{}, errors.New("trusted proxy CIDR is invalid")
		}
		resolver.trusted = append(resolver.trusted, prefix.Masked())
	}
	return resolver, nil
}

func (r clientAddressResolver) clientAddress(request *http.Request) string {
	peer, ok := parseRemoteAddress(request.RemoteAddr)
	if !ok {
		return "unknown"
	}
	if !r.isTrusted(peer) {
		return peer.String()
	}
	values := request.Header.Values("X-Forwarded-For")
	chain := make([]netip.Addr, 0, len(values)+1)
	for _, value := range values {
		for _, raw := range strings.Split(value, ",") {
			address, err := netip.ParseAddr(strings.TrimSpace(raw))
			if err != nil {
				return peer.String()
			}
			chain = append(chain, address.Unmap())
		}
	}
	chain = append(chain, peer)
	for index := len(chain) - 1; index >= 0; index-- {
		if !r.isTrusted(chain[index]) {
			return chain[index].String()
		}
	}
	return chain[0].String()
}

func (r clientAddressResolver) isTrusted(address netip.Addr) bool {
	for _, prefix := range r.trusted {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}

func parseRemoteAddress(value string) (netip.Addr, bool) {
	if addressPort, err := netip.ParseAddrPort(strings.TrimSpace(value)); err == nil {
		return addressPort.Addr().Unmap(), true
	}
	address, err := netip.ParseAddr(strings.TrimSpace(value))
	return address.Unmap(), err == nil
}

func rateLimitPolicyForRequest(r *http.Request) (rateLimitPolicy, bool) {
	if r.Method != http.MethodPost {
		return rateLimitPolicy{}, false
	}
	switch {
	case r.URL.Path == "/api/v1/auth/login" || r.URL.Path == "/api/v1/client/auth/login":
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
	return (clientAddressResolver{}).clientAddress(r)
}

func formatRetryAfter(delay time.Duration) string {
	seconds := int(delay.Round(time.Second) / time.Second)
	if seconds < 1 {
		seconds = 1
	}
	return strconv.Itoa(seconds)
}
