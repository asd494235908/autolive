package httpapi

import (
	"context"
	"database/sql"
	"net/http"
	"strconv"
	"strings"
	"time"

	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promhttp"
)

// httpMetrics owns a registry per router so tests and multiple embedded
// servers do not share mutable global Prometheus state.
type httpMetrics struct {
	registry        *prometheus.Registry
	requestsTotal   *prometheus.CounterVec
	durationSeconds *prometheus.HistogramVec
	inFlight        *prometheus.GaugeVec
	auditFailures   prometheus.Counter
	rateLimited     *prometheus.CounterVec
}

// databasePoolSaturationThreshold is intentionally fixed and low-cardinality:
// operators can alert on the exported boolean without a second alerting
// framework, while the ratio remains useful for dashboards.
const databasePoolSaturationThreshold = 0.8

func newHTTPMetrics(repository any, healthTelemetry ...service.ModelPoolHealthProbeTelemetry) *httpMetrics {
	var health service.ModelPoolHealthProbeTelemetry
	if len(healthTelemetry) > 0 {
		health = healthTelemetry[0]
	}
	return newHTTPMetricsWithTelemetry(repository, health, nil)
}

func newHTTPMetricsWithTelemetry(repository any, healthTelemetry service.ModelPoolHealthProbeTelemetry, retentionTelemetry service.RetentionCleanupTelemetry) *httpMetrics {
	metrics := &httpMetrics{
		registry: prometheus.NewRegistry(),
		requestsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "http",
			Name:      "requests_total",
			Help:      "Total number of completed HTTP requests.",
		}, []string{"method", "route", "code"}),
		durationSeconds: prometheus.NewHistogramVec(prometheus.HistogramOpts{
			Namespace: "autolive",
			Subsystem: "http",
			Name:      "request_duration_seconds",
			Help:      "HTTP request duration in seconds.",
			Buckets:   prometheus.DefBuckets,
		}, []string{"method", "route"}),
		inFlight: prometheus.NewGaugeVec(prometheus.GaugeOpts{
			Namespace: "autolive",
			Subsystem: "http",
			Name:      "requests_in_flight",
			Help:      "Current number of in-flight HTTP requests.",
		}, []string{"method"}),
		auditFailures: prometheus.NewCounter(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "audit",
			Name:      "write_failures_total",
			Help:      "Total number of audit log writes that failed after an HTTP request completed.",
		}),
		rateLimited: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "http",
			Name:      "rate_limited_total",
			Help:      "Total number of requests rejected by the in-process rate limiter.",
		}, []string{"policy"}),
	}
	metrics.registry.MustRegister(prometheus.NewGoCollector())
	metrics.registry.MustRegister(metrics.requestsTotal, metrics.durationSeconds, metrics.inFlight, metrics.auditFailures, metrics.rateLimited)
	if provider, ok := repository.(interface{ DBStats() sql.DBStats }); ok {
		metrics.registerDatabaseStats(provider)
	}
	if provider, ok := repository.(store.OperationalMetricsReader); ok {
		metrics.registerOperationalMetrics(provider)
	}
	if healthTelemetry != nil {
		if collector, ok := healthTelemetry.(prometheus.Collector); ok {
			metrics.registry.MustRegister(collector)
		}
	}
	if retentionTelemetry != nil {
		if collector, ok := retentionTelemetry.(prometheus.Collector); ok {
			metrics.registry.MustRegister(collector)
		}
	}
	return metrics
}

// ModelPoolHealthProbeMetrics adapts the scheduler's bounded telemetry
// interface to Prometheus. All labels are fixed lifecycle/status values; no
// account, provider, URL, error text or secret is accepted at this boundary.
type ModelPoolHealthProbeMetrics struct {
	runsTotal   *prometheus.CounterVec
	runDuration *prometheus.HistogramVec
	probesTotal *prometheus.CounterVec
	skipsTotal  *prometheus.CounterVec
}

var _ service.ModelPoolHealthProbeTelemetry = (*ModelPoolHealthProbeMetrics)(nil)
var _ prometheus.Collector = (*ModelPoolHealthProbeMetrics)(nil)

func NewModelPoolHealthProbeMetrics() *ModelPoolHealthProbeMetrics {
	return &ModelPoolHealthProbeMetrics{
		runsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "model_pool_health",
			Name:      "runs_total",
			Help:      "Total model pool health probe scheduler runs by bounded outcome.",
		}, []string{"status"}),
		runDuration: prometheus.NewHistogramVec(prometheus.HistogramOpts{
			Namespace: "autolive",
			Subsystem: "model_pool_health",
			Name:      "run_duration_seconds",
			Help:      "Model pool health probe scheduler run duration in seconds.",
			Buckets:   prometheus.DefBuckets,
		}, []string{"status"}),
		probesTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "model_pool_health",
			Name:      "probes_total",
			Help:      "Total model pool health probes by bounded outcome.",
		}, []string{"status"}),
		skipsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "model_pool_health",
			Name:      "skips_total",
			Help:      "Total model pool health probe skips by bounded reason.",
		}, []string{"reason"}),
	}
}

func (m *ModelPoolHealthProbeMetrics) ObserveRun(status service.ModelPoolHealthProbeRunStatus, duration time.Duration) {
	if m == nil {
		return
	}
	value := boundedHealthRunStatus(status)
	if duration < 0 {
		duration = 0
	}
	m.runsTotal.WithLabelValues(value).Inc()
	m.runDuration.WithLabelValues(value).Observe(duration.Seconds())
}

func (m *ModelPoolHealthProbeMetrics) ObserveProbe(status service.ModelPoolHealthProbeStatus) {
	if m == nil {
		return
	}
	m.probesTotal.WithLabelValues(boundedHealthProbeStatus(status)).Inc()
}

func (m *ModelPoolHealthProbeMetrics) ObserveSkip(reason service.ModelPoolHealthProbeSkipReason) {
	if m == nil {
		return
	}
	m.skipsTotal.WithLabelValues(boundedHealthSkipReason(reason)).Inc()
}

func boundedHealthRunStatus(status service.ModelPoolHealthProbeRunStatus) string {
	switch status {
	case service.ModelPoolHealthProbeRunCompleted, service.ModelPoolHealthProbeRunFailed, service.ModelPoolHealthProbeRunCancelled, service.ModelPoolHealthProbeRunTimedOut, service.ModelPoolHealthProbeRunUnknown:
		return string(status)
	default:
		return string(service.ModelPoolHealthProbeRunUnknown)
	}
}

func boundedHealthProbeStatus(status service.ModelPoolHealthProbeStatus) string {
	switch status {
	case service.ModelPoolHealthProbeSucceeded, service.ModelPoolHealthProbeFailed, service.ModelPoolHealthProbeUnhealthy, service.ModelPoolHealthProbeCancelled, service.ModelPoolHealthProbeTimedOut, service.ModelPoolHealthProbeUnknown:
		return string(status)
	default:
		return string(service.ModelPoolHealthProbeUnknown)
	}
}

func boundedHealthSkipReason(reason service.ModelPoolHealthProbeSkipReason) string {
	switch reason {
	case service.ModelPoolHealthProbeSkipLimit, service.ModelPoolHealthProbeSkipInactive, service.ModelPoolHealthProbeSkipMissingSecret, service.ModelPoolHealthProbeSkipBackoff, service.ModelPoolHealthProbeSkipDistributedLock, service.ModelPoolHealthProbeSkipUnknown:
		return string(reason)
	default:
		return string(service.ModelPoolHealthProbeSkipUnknown)
	}
}

func (m *ModelPoolHealthProbeMetrics) Describe(ch chan<- *prometheus.Desc) {
	if m == nil {
		return
	}
	m.runsTotal.Describe(ch)
	m.runDuration.Describe(ch)
	m.probesTotal.Describe(ch)
	m.skipsTotal.Describe(ch)
}

func (m *ModelPoolHealthProbeMetrics) Collect(ch chan<- prometheus.Metric) {
	if m == nil {
		return
	}
	m.runsTotal.Collect(ch)
	m.runDuration.Collect(ch)
	m.probesTotal.Collect(ch)
	m.skipsTotal.Collect(ch)
}

func (m *httpMetrics) recordAuditWriteFailure() {
	if m == nil {
		return
	}
	m.auditFailures.Inc()
}

func (m *httpMetrics) recordRateLimited(policy string) {
	if m == nil {
		return
	}
	m.rateLimited.WithLabelValues(policy).Inc()
}

// registerDatabaseStats is intentionally limited to SQL-backed repositories:
// sql.DBStats describes a real database pool, so MemoryStore does not emit
// fabricated pool values. Both snapshot and normalized Postgres repositories
// expose the same aggregate metrics through PostgresRepository.DBStats.
func (m *httpMetrics) registerDatabaseStats(provider interface{ DBStats() sql.DBStats }) {
	newGauge := func(name, help string, value func(sql.DBStats) float64) prometheus.Collector {
		return prometheus.NewGaugeFunc(prometheus.GaugeOpts{
			Namespace: "autolive",
			Subsystem: "database",
			Name:      name,
			Help:      help,
		}, func() float64 { return value(provider.DBStats()) })
	}
	newCounter := func(name, help string, value func(sql.DBStats) float64) prometheus.Collector {
		return prometheus.NewCounterFunc(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "database",
			Name:      name,
			Help:      help,
		}, func() float64 { return value(provider.DBStats()) })
	}
	m.registry.MustRegister(
		newGauge("open_connections", "Current number of open PostgreSQL connections.", func(stats sql.DBStats) float64 { return float64(stats.OpenConnections) }),
		newGauge("in_use_connections", "Current number of PostgreSQL connections in use.", func(stats sql.DBStats) float64 { return float64(stats.InUse) }),
		newGauge("idle_connections", "Current number of idle PostgreSQL connections.", func(stats sql.DBStats) float64 { return float64(stats.Idle) }),
		newCounter("wait_count_total", "Total number of waits for a PostgreSQL connection.", func(stats sql.DBStats) float64 { return float64(stats.WaitCount) }),
		newCounter("wait_duration_seconds_total", "Total time spent waiting for a PostgreSQL connection.", func(stats sql.DBStats) float64 { return stats.WaitDuration.Seconds() }),
		newGauge("max_open_connections", "Configured maximum number of open PostgreSQL connections.", func(stats sql.DBStats) float64 { return float64(stats.MaxOpenConnections) }),
		newGauge("pool_saturation_ratio", "Current PostgreSQL connection pool saturation ratio (in-use divided by max open connections). Zero means the pool has no configured maximum.", databasePoolSaturationRatio),
		newGauge("pool_saturated", "Whether the PostgreSQL connection pool is at or above the fixed 0.8 saturation threshold.", func(stats sql.DBStats) float64 {
			if databasePoolSaturationRatio(stats) >= databasePoolSaturationThreshold {
				return 1
			}
			return 0
		}),
	)
}

func databasePoolSaturationRatio(stats sql.DBStats) float64 {
	if stats.MaxOpenConnections <= 0 || stats.InUse <= 0 {
		return 0
	}
	ratio := float64(stats.InUse) / float64(stats.MaxOpenConnections)
	if ratio > 1 {
		return 1
	}
	return ratio
}

func (m *httpMetrics) registerOperationalMetrics(provider store.OperationalMetricsReader) {
	m.registry.MustRegister(&operationalMetricsCollector{provider: provider})
}

func readOperationalMetrics(provider store.OperationalMetricsReader) (store.OperationalMetrics, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	return provider.ReadOperationalMetrics(ctx)
}

type operationalMetricsCollector struct {
	provider store.OperationalMetricsReader
}

func (c *operationalMetricsCollector) Describe(ch chan<- *prometheus.Desc) {
	ch <- prometheus.NewDesc("autolive_model_accounts", "Current model account count by fixed lifecycle status.", []string{"status"}, nil)
	ch <- prometheus.NewDesc("autolive_model_leases_active", "Current number of active, non-expired model leases.", nil, nil)
	ch <- prometheus.NewDesc("autolive_model_usage_tokens_today", "Total model usage tokens recorded since the current UTC day began.", nil, nil)
	ch <- prometheus.NewDesc("autolive_user_authorization_policies_configured", "Number of users with a non-empty model allowlist or recorded usage guard.", nil, nil)
}

func (c *operationalMetricsCollector) Collect(ch chan<- prometheus.Metric) {
	metrics, err := readOperationalMetrics(c.provider)
	if err != nil {
		return
	}
	accountDesc := prometheus.NewDesc("autolive_model_accounts", "Current model account count by fixed lifecycle status.", []string{"status"}, nil)
	for _, status := range []string{"active", "cooldown", "exhausted", "disabled"} {
		ch <- prometheus.MustNewConstMetric(accountDesc, prometheus.GaugeValue, float64(metrics.ModelAccountStatusCounts[status]), status)
	}
	ch <- prometheus.MustNewConstMetric(prometheus.NewDesc("autolive_model_leases_active", "Current number of active, non-expired model leases.", nil, nil), prometheus.GaugeValue, float64(metrics.ActiveModelLeases))
	ch <- prometheus.MustNewConstMetric(prometheus.NewDesc("autolive_model_usage_tokens_today", "Total model usage tokens recorded since the current UTC day began.", nil, nil), prometheus.GaugeValue, float64(metrics.DailyModelUsageTokens))
	ch <- prometheus.MustNewConstMetric(prometheus.NewDesc("autolive_user_authorization_policies_configured", "Number of users with a non-empty model allowlist or recorded usage guard.", nil, nil), prometheus.GaugeValue, float64(metrics.ConfiguredUserAuthorizationPolicies))
}

func (m *httpMetrics) handler() http.Handler {
	return promhttp.HandlerFor(m.registry, promhttp.HandlerOpts{})
}

func (m *httpMetrics) middleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/metrics" {
			next.ServeHTTP(w, r)
			return
		}

		method := metricMethod(r.Method)
		m.inFlight.WithLabelValues(method).Inc()
		defer m.inFlight.WithLabelValues(method).Dec()

		startedAt := time.Now()
		recorder := &statusRecorder{ResponseWriter: w, statusCode: http.StatusOK}
		next.ServeHTTP(recorder, r)

		route := metricRoute(r)
		code := strconv.Itoa(recorder.statusCode)
		m.requestsTotal.WithLabelValues(method, route, code).Inc()
		m.durationSeconds.WithLabelValues(method, route).Observe(time.Since(startedAt).Seconds())
	})
}

func metricMethod(method string) string {
	switch method {
	case http.MethodConnect, http.MethodDelete, http.MethodGet, http.MethodHead,
		http.MethodOptions, http.MethodPatch, http.MethodPost, http.MethodPut, http.MethodTrace:
		return method
	default:
		return "OTHER"
	}
}

func metricRoute(r *http.Request) string {
	path := r.URL.Path
	switch {
	case path == "/api/v1/health", path == "/api/v1/livez", path == "/api/v1/readyz":
		return path
	case path == "/api/v1/auth/login", path == "/api/v1/auth/refresh", path == "/api/v1/auth/logout":
		return path
	case strings.HasPrefix(path, "/api/v1/admin/users/"):
		return "/api/v1/admin/users/:id"
	case strings.HasPrefix(path, "/api/v1/admin/devices/"):
		return "/api/v1/admin/devices/:id"
	case strings.HasPrefix(path, "/api/v1/admin/activation-codes/"):
		return "/api/v1/admin/activation-codes/:id"
	case strings.HasPrefix(path, "/api/v1/admin/model-pool/"):
		return "/api/v1/admin/model-pool/:id"
	case strings.HasPrefix(path, "/api/v1/client/model-leases/"):
		return "/api/v1/client/model-leases/:id"
	case path == "/api/v1/admin/users", path == "/api/v1/admin/devices", path == "/api/v1/admin/activation-codes", path == "/api/v1/admin/model-pool", path == "/api/v1/admin/model-leases", path == "/api/v1/admin/model-usage", path == "/api/v1/admin/audit-logs", path == "/api/v1/client/activate", path == "/api/v1/client/heartbeat", path == "/api/v1/client/profile", path == "/api/v1/client/model-leases", path == "/api/v1/client/llm/call-records":
		return path
	default:
		return "unmatched"
	}
}
