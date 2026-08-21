package httpapi

import (
	"time"

	"autoLive/backend/internal/service"
	"github.com/prometheus/client_golang/prometheus"
)

// RetentionCleanupMetrics adapts bounded scheduler events to Prometheus.
// Dataset and status values are normalized before they become labels.
type RetentionCleanupMetrics struct {
	runsTotal     *prometheus.CounterVec
	runDuration   *prometheus.HistogramVec
	datasetsTotal *prometheus.CounterVec
	deletedTotal  *prometheus.CounterVec
}

var _ service.RetentionCleanupTelemetry = (*RetentionCleanupMetrics)(nil)
var _ prometheus.Collector = (*RetentionCleanupMetrics)(nil)

func NewRetentionCleanupMetrics() *RetentionCleanupMetrics {
	return &RetentionCleanupMetrics{
		runsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "retention_cleanup",
			Name:      "runs_total",
			Help:      "Total retention cleanup scheduler runs by bounded outcome.",
		}, []string{"status"}),
		runDuration: prometheus.NewHistogramVec(prometheus.HistogramOpts{
			Namespace: "autolive",
			Subsystem: "retention_cleanup",
			Name:      "run_duration_seconds",
			Help:      "Retention cleanup scheduler run duration in seconds.",
			Buckets:   prometheus.DefBuckets,
		}, []string{"status"}),
		datasetsTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "retention_cleanup",
			Name:      "datasets_total",
			Help:      "Retention cleanup dataset outcomes by bounded dataset and status.",
		}, []string{"dataset", "status"}),
		deletedTotal: prometheus.NewCounterVec(prometheus.CounterOpts{
			Namespace: "autolive",
			Subsystem: "retention_cleanup",
			Name:      "records_deleted_total",
			Help:      "Total records deleted by bounded retention dataset.",
		}, []string{"dataset"}),
	}
}

func (m *RetentionCleanupMetrics) ObserveRun(status service.RetentionCleanupRunStatus, duration time.Duration) {
	if m == nil {
		return
	}
	if duration < 0 {
		duration = 0
	}
	value := boundedRetentionRunStatus(status)
	m.runsTotal.WithLabelValues(value).Inc()
	m.runDuration.WithLabelValues(value).Observe(duration.Seconds())
}

func (m *RetentionCleanupMetrics) ObserveDataset(dataset service.RetentionCleanupDataset, status service.RetentionCleanupDatasetStatus, deleted int64) {
	if m == nil {
		return
	}
	if deleted < 0 {
		deleted = 0
	}
	datasetValue := boundedRetentionDataset(dataset)
	m.datasetsTotal.WithLabelValues(datasetValue, boundedRetentionDatasetStatus(status)).Inc()
	if deleted > 0 {
		m.deletedTotal.WithLabelValues(datasetValue).Add(float64(deleted))
	}
}

func boundedRetentionRunStatus(status service.RetentionCleanupRunStatus) string {
	switch status {
	case service.RetentionCleanupRunCompleted, service.RetentionCleanupRunFailed, service.RetentionCleanupRunCancelled, service.RetentionCleanupRunTimedOut, service.RetentionCleanupRunUnknown:
		return string(status)
	default:
		return string(service.RetentionCleanupRunUnknown)
	}
}

func boundedRetentionDataset(dataset service.RetentionCleanupDataset) string {
	switch dataset {
	case service.RetentionCleanupDatasetAuthSessions, service.RetentionCleanupDatasetOrphanedDeviceBindings, service.RetentionCleanupDatasetStagedSecrets, service.RetentionCleanupDatasetIdempotencyRecords, service.RetentionCleanupDatasetModelTestResults, service.RetentionCleanupDatasetAuditLogs, service.RetentionCleanupDatasetUnknown:
		return string(dataset)
	default:
		return string(service.RetentionCleanupDatasetUnknown)
	}
}

func boundedRetentionDatasetStatus(status service.RetentionCleanupDatasetStatus) string {
	switch status {
	case service.RetentionCleanupDatasetCompleted, service.RetentionCleanupDatasetFailed, service.RetentionCleanupDatasetSkipped, service.RetentionCleanupDatasetCancelled, service.RetentionCleanupDatasetTimedOut, service.RetentionCleanupDatasetStatusUnknown:
		return string(status)
	default:
		return string(service.RetentionCleanupDatasetStatusUnknown)
	}
}

func (m *RetentionCleanupMetrics) Describe(ch chan<- *prometheus.Desc) {
	if m == nil {
		return
	}
	m.runsTotal.Describe(ch)
	m.runDuration.Describe(ch)
	m.datasetsTotal.Describe(ch)
	m.deletedTotal.Describe(ch)
}

func (m *RetentionCleanupMetrics) Collect(ch chan<- prometheus.Metric) {
	if m == nil {
		return
	}
	m.runsTotal.Collect(ch)
	m.runDuration.Collect(ch)
	m.datasetsTotal.Collect(ch)
	m.deletedTotal.Collect(ch)
}
