package service

import "time"

// ModelPoolHealthProbeRunStatus is a bounded outcome label for one scheduler
// pass. It intentionally contains no account, provider, or request identity.
type ModelPoolHealthProbeRunStatus string

const (
	ModelPoolHealthProbeRunCompleted ModelPoolHealthProbeRunStatus = "completed"
	ModelPoolHealthProbeRunFailed    ModelPoolHealthProbeRunStatus = "failed"
	ModelPoolHealthProbeRunCancelled ModelPoolHealthProbeRunStatus = "cancelled"
	ModelPoolHealthProbeRunTimedOut  ModelPoolHealthProbeRunStatus = "timeout"
	ModelPoolHealthProbeRunUnknown   ModelPoolHealthProbeRunStatus = "unknown"
)

// ModelPoolHealthProbeStatus is a bounded outcome label for one account probe.
type ModelPoolHealthProbeStatus string

const (
	ModelPoolHealthProbeSucceeded ModelPoolHealthProbeStatus = "succeeded"
	ModelPoolHealthProbeFailed    ModelPoolHealthProbeStatus = "failed"
	ModelPoolHealthProbeUnhealthy ModelPoolHealthProbeStatus = "unhealthy"
	ModelPoolHealthProbeCancelled ModelPoolHealthProbeStatus = "cancelled"
	ModelPoolHealthProbeTimedOut  ModelPoolHealthProbeStatus = "timeout"
	ModelPoolHealthProbeUnknown   ModelPoolHealthProbeStatus = "unknown"
)

// ModelPoolHealthProbeSkipReason is a bounded reason for not scheduling an
// account in a pass. These values are deliberately stable and low-cardinality.
type ModelPoolHealthProbeSkipReason string

const (
	ModelPoolHealthProbeSkipLimit           ModelPoolHealthProbeSkipReason = "limit"
	ModelPoolHealthProbeSkipInactive        ModelPoolHealthProbeSkipReason = "inactive"
	ModelPoolHealthProbeSkipMissingSecret   ModelPoolHealthProbeSkipReason = "missing_secret"
	ModelPoolHealthProbeSkipBackoff         ModelPoolHealthProbeSkipReason = "backoff"
	ModelPoolHealthProbeSkipDistributedLock ModelPoolHealthProbeSkipReason = "distributed_lock"
	ModelPoolHealthProbeSkipUnknown         ModelPoolHealthProbeSkipReason = "unknown"
)

// ModelPoolHealthProbeTelemetry is the service boundary for scheduler
// observability. Implementations can export these events to Prometheus,
// OpenTelemetry, or a test snapshot without coupling the scheduler to HTTP or
// a particular metrics registry. Implementations must keep labels bounded and
// must not include account IDs, provider URLs, error strings, or secrets.
type ModelPoolHealthProbeTelemetry interface {
	ObserveRun(status ModelPoolHealthProbeRunStatus, duration time.Duration)
	ObserveProbe(status ModelPoolHealthProbeStatus)
	ObserveSkip(reason ModelPoolHealthProbeSkipReason)
}

type discardModelPoolHealthProbeTelemetry struct{}

func (discardModelPoolHealthProbeTelemetry) ObserveRun(ModelPoolHealthProbeRunStatus, time.Duration) {
}
func (discardModelPoolHealthProbeTelemetry) ObserveProbe(ModelPoolHealthProbeStatus)    {}
func (discardModelPoolHealthProbeTelemetry) ObserveSkip(ModelPoolHealthProbeSkipReason) {}
