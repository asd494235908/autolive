package httpapi

import (
	"context"
	"database/sql"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

type metricsDBStatsProvider struct{}

func (metricsDBStatsProvider) DBStats() sql.DBStats {
	return sql.DBStats{
		OpenConnections:    4,
		InUse:              2,
		Idle:               2,
		WaitCount:          7,
		WaitDuration:       1500 * time.Millisecond,
		MaxOpenConnections: 10,
	}
}

func TestMetricsExposeStableHTTPLabelsWithoutRequestIdentifiers(t *testing.T) {
	handler := newTestRouter(t)

	rec := doJSON(t, handler, http.MethodGet, "/api/v1/health", nil, "", "")
	if rec.Code != http.StatusOK {
		t.Fatalf("health status = %d, want %d", rec.Code, http.StatusOK)
	}

	metricsRec := doJSON(t, handler, http.MethodGet, "/metrics", nil, "", "")
	if metricsRec.Code != http.StatusOK {
		t.Fatalf("metrics status = %d, want %d, body=%s", metricsRec.Code, http.StatusOK, metricsRec.Body.String())
	}
	if contentType := metricsRec.Header().Get("Content-Type"); !strings.Contains(contentType, "text/plain") {
		t.Fatalf("metrics content type = %q, want text/plain", contentType)
	}
	body := metricsRec.Body.String()
	if !strings.Contains(body, `autolive_http_requests_total{code="200",method="GET",route="/api/v1/health"}`) {
		t.Fatalf("health request metric missing: %s", body)
	}
	if strings.Contains(body, "request_id") || strings.Contains(body, "/api/v1/health?x=") {
		t.Fatalf("metrics contain high-cardinality request data: %s", body)
	}
}

func TestMetricsCollapseUnknownPathsToStableRouteLabel(t *testing.T) {
	handler := newTestRouter(t)
	for _, path := range []string{"/not-found/a", "/not-found/b"} {
		rec := doJSON(t, handler, http.MethodGet, path, nil, "", "")
		if rec.Code != http.StatusNotFound {
			t.Fatalf("path %q status = %d, want %d", path, rec.Code, http.StatusNotFound)
		}
	}
	metricsRec := doJSON(t, handler, http.MethodGet, "/metrics", nil, "", "")
	body := metricsRec.Body.String()
	if !strings.Contains(body, `autolive_http_requests_total{code="404",method="GET",route="unmatched"} 2`) {
		t.Fatalf("unknown route metric missing or high cardinality: %s", body)
	}
	if strings.Contains(body, "/not-found/a") || strings.Contains(body, "/not-found/b") {
		t.Fatalf("unknown path leaked into metrics: %s", body)
	}
}

func TestMetricsExposeDatabasePoolStatsAsAggregates(t *testing.T) {
	metrics := newHTTPMetrics(metricsDBStatsProvider{})
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	body := recorder.Body.String()
	for _, metric := range []string{
		`autolive_database_open_connections 4`,
		`autolive_database_in_use_connections 2`,
		`autolive_database_idle_connections 2`,
		`autolive_database_wait_count_total 7`,
		`autolive_database_wait_duration_seconds_total 1.5`,
		`autolive_database_max_open_connections 10`,
		`autolive_database_pool_saturation_ratio 0.2`,
		`autolive_database_pool_saturated 0`,
	} {
		if !strings.Contains(body, metric) {
			t.Fatalf("database metric %q missing: %s", metric, body)
		}
	}
}

func TestDatabasePoolSaturationUsesFixedThresholdAndHandlesUnlimitedPools(t *testing.T) {
	tests := []struct {
		name          string
		stats         sql.DBStats
		wantRatio     float64
		wantSaturated float64
	}{
		{name: "threshold", stats: sql.DBStats{InUse: 8, MaxOpenConnections: 10}, wantRatio: 0.8, wantSaturated: 1},
		{name: "clamped", stats: sql.DBStats{InUse: 12, MaxOpenConnections: 10}, wantRatio: 1, wantSaturated: 1},
		{name: "unlimited", stats: sql.DBStats{InUse: 12}, wantRatio: 0, wantSaturated: 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := databasePoolSaturationRatio(tt.stats); got != tt.wantRatio {
				t.Fatalf("databasePoolSaturationRatio(%+v) = %v, want %v", tt.stats, got, tt.wantRatio)
			}
			got := 0.0
			if databasePoolSaturationRatio(tt.stats) >= databasePoolSaturationThreshold {
				got = 1
			}
			if got != tt.wantSaturated {
				t.Fatalf("pool saturation(%+v) = %v, want %v", tt.stats, got, tt.wantSaturated)
			}
		})
	}
}

func TestMetricsKeepDatabasePoolBoundaryExplicitForMemoryStore(t *testing.T) {
	metrics := newHTTPMetrics(store.NewMemoryStore(time.Now))
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	if strings.Contains(recorder.Body.String(), "autolive_database_pool_saturation") {
		t.Fatalf("memory mode exposed PostgreSQL pool metrics: %s", recorder.Body.String())
	}
}

func TestMetricRouteIncludesAdminModelLeaseList(t *testing.T) {
	req := httptest.NewRequest(http.MethodGet, "/api/v1/admin/model-leases?page=2", nil)
	if got := metricRoute(req); got != "/api/v1/admin/model-leases" {
		t.Fatalf("metricRoute() = %q, want admin model lease list route", got)
	}
}

func TestMetricsExposeRateLimitAndAuditFailureCounters(t *testing.T) {
	handler := newTestRouter(t)
	start := make(chan struct{})
	const attempts = 21
	responses := make(chan int, attempts)
	var group sync.WaitGroup
	for attempt := 0; attempt < attempts; attempt++ {
		group.Add(1)
		go func() {
			defer group.Done()
			<-start
			rec := doJSON(t, handler, http.MethodPost, "/api/v1/auth/login", map[string]any{
				"username": "admin",
				"password": "wrong-password",
				"product":  "autolive",
			}, "", "")
			responses <- rec.Code
		}()
	}
	close(start)
	group.Wait()
	close(responses)
	unauthorized, rateLimited := 0, 0
	for status := range responses {
		switch status {
		case http.StatusUnauthorized:
			unauthorized++
		case http.StatusTooManyRequests:
			rateLimited++
		default:
			t.Fatalf("login status = %d, want 401 or 429", status)
		}
	}
	if unauthorized != authLoginAccountRateLimitPolicy.burst || rateLimited != 1 {
		t.Fatalf("login statuses = unauthorized %d, rate_limited %d; want %d/1", unauthorized, rateLimited, authLoginAccountRateLimitPolicy.burst)
	}

	metricsRec := doJSON(t, handler, http.MethodGet, "/metrics", nil, "", "")
	body := metricsRec.Body.String()
	if !strings.Contains(body, `autolive_http_rate_limited_total{policy="auth-login"} 1`) {
		t.Fatalf("rate-limit metric missing: %s", body)
	}

	metrics := newHTTPMetrics(nil)
	metrics.recordAuditWriteFailure()
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	if !strings.Contains(recorder.Body.String(), "autolive_audit_write_failures_total 1") {
		t.Fatalf("audit failure metric missing: %s", recorder.Body.String())
	}
}

func TestMetricsExposeLowCardinalityModelAndAuthorizationAggregates(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.ModelPoolAccounts["mpa_metrics"] = controlplane.ModelPoolAccountSummary{ID: "mpa_metrics", Status: controlplane.ModelAccountStatusActive}
		state.ModelLeases["lease_metrics"] = controlplane.ModelLease{ID: "lease_metrics", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_metrics"] = controlplane.ModelUsageRecord{ID: "usage_metrics", TotalTokens: 42, CreatedAt: now.Add(-time.Minute).Format(time.RFC3339)}
		state.UserAuthorizationPolicies["usr_metrics"] = controlplane.UserAuthorizationPolicy{UserID: "usr_metrics", DailyTokenLimit: 100}
		return nil
	}); err != nil {
		t.Fatalf("seed metrics state: %v", err)
	}
	metrics := newHTTPMetrics(repository)
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	body := recorder.Body.String()
	for _, metric := range []string{
		`autolive_model_accounts{status="active"} 1`,
		`autolive_model_accounts{status="disabled"} 0`,
		`autolive_model_leases_active 1`,
		`autolive_model_usage_tokens_today 42`,
		`autolive_user_authorization_policies_configured 1`,
	} {
		if !strings.Contains(body, metric) {
			t.Fatalf("operational metric %q missing: %s", metric, body)
		}
	}
	if strings.Contains(body, "mpa_metrics") || strings.Contains(body, "usr_metrics") {
		t.Fatalf("operational metrics leaked high-cardinality IDs: %s", body)
	}
}

func TestMetricsExposeHealthProbeTelemetryWithBoundedLabels(t *testing.T) {
	telemetry := NewModelPoolHealthProbeMetrics()
	telemetry.ObserveRun(service.ModelPoolHealthProbeRunCompleted, 250*time.Millisecond)
	telemetry.ObserveProbe(service.ModelPoolHealthProbeSucceeded)
	telemetry.ObserveProbe(service.ModelPoolHealthProbeUnhealthy)
	telemetry.ObserveSkip(service.ModelPoolHealthProbeSkipBackoff)
	telemetry.ObserveSkip(service.ModelPoolHealthProbeSkipDistributedLock)
	telemetry.ObserveRun(service.ModelPoolHealthProbeRunTimedOut, 10*time.Millisecond)
	telemetry.ObserveProbe(service.ModelPoolHealthProbeTimedOut)
	telemetry.ObserveRun(service.ModelPoolHealthProbeRunStatus("account-secret"), -time.Second)
	telemetry.ObserveProbe(service.ModelPoolHealthProbeStatus("provider-url"))
	telemetry.ObserveSkip(service.ModelPoolHealthProbeSkipReason("request-id"))

	metrics := newHTTPMetrics(nil, telemetry)
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	body := recorder.Body.String()
	for _, metric := range []string{
		`autolive_model_pool_health_runs_total{status="completed"} 1`,
		`autolive_model_pool_health_runs_total{status="timeout"} 1`,
		`autolive_model_pool_health_probes_total{status="succeeded"} 1`,
		`autolive_model_pool_health_probes_total{status="unhealthy"} 1`,
		`autolive_model_pool_health_probes_total{status="timeout"} 1`,
		`autolive_model_pool_health_skips_total{reason="backoff"} 1`,
		`autolive_model_pool_health_skips_total{reason="distributed_lock"} 1`,
		`autolive_model_pool_health_runs_total{status="unknown"} 1`,
		`autolive_model_pool_health_probes_total{status="unknown"} 1`,
		`autolive_model_pool_health_skips_total{reason="unknown"} 1`,
		`autolive_model_pool_health_run_duration_seconds_count{status="completed"} 1`,
	} {
		if !strings.Contains(body, metric) {
			t.Fatalf("health telemetry metric %q missing: %s", metric, body)
		}
	}
	if strings.Contains(body, "account_id") || strings.Contains(body, "provider") || strings.Contains(body, "secret") {
		t.Fatalf("health telemetry exposed forbidden high-cardinality fields: %s", body)
	}
}

func TestMetricsExposeRetentionCleanupTelemetryWithBoundedLabels(t *testing.T) {
	telemetry := NewRetentionCleanupMetrics()
	telemetry.ObserveRun(service.RetentionCleanupRunCompleted, 150*time.Millisecond)
	telemetry.ObserveRun(service.RetentionCleanupRunTimedOut, 10*time.Millisecond)
	telemetry.ObserveDataset(service.RetentionCleanupDatasetAuditLogs, service.RetentionCleanupDatasetCompleted, 4)
	telemetry.ObserveDataset(service.RetentionCleanupDatasetIdempotencyRecords, service.RetentionCleanupDatasetTimedOut, 0)
	telemetry.ObserveDataset(service.RetentionCleanupDataset("table-secret"), service.RetentionCleanupDatasetStatus("error-text"), -1)
	telemetry.ObserveRun(service.RetentionCleanupRunStatus("request-id"), -time.Second)

	metrics := newHTTPMetricsWithTelemetry(nil, nil, telemetry)
	recorder := httptest.NewRecorder()
	metrics.handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	body := recorder.Body.String()
	for _, metric := range []string{
		`autolive_retention_cleanup_runs_total{status="completed"} 1`,
		`autolive_retention_cleanup_runs_total{status="timeout"} 1`,
		`autolive_retention_cleanup_datasets_total{dataset="audit_logs",status="completed"} 1`,
		`autolive_retention_cleanup_records_deleted_total{dataset="audit_logs"} 4`,
		`autolive_retention_cleanup_datasets_total{dataset="idempotency_records",status="timeout"} 1`,
		`autolive_retention_cleanup_runs_total{status="unknown"} 1`,
		`autolive_retention_cleanup_datasets_total{dataset="unknown",status="unknown"} 1`,
	} {
		if !strings.Contains(body, metric) {
			t.Fatalf("retention telemetry metric %q missing: %s", metric, body)
		}
	}
	if strings.Contains(body, "table-secret") || strings.Contains(body, "error-text") || strings.Contains(body, "request-id") {
		t.Fatalf("retention telemetry exposed unbounded labels: %s", body)
	}
}
