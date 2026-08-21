package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"autoLive/backend/migrations"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/common/expfmt"
)

const (
	migrationMetricStatusSuccess   = "success"
	migrationMetricStatusFailure   = "failure"
	migrationMetricStatusTimeout   = "timeout"
	migrationMetricStatusCancelled = "cancelled"
)

// renderMigrationMetrics creates a complete one-shot text exposition. The
// file is a snapshot rather than a process counter: every execution replaces
// it atomically, so a node exporter/textfile collector can scrape the latest
// migration result after the short-lived command exits.
func renderMigrationMetrics(migrationErr error, duration, timeout time.Duration) ([]byte, error) {
	registry := prometheus.NewRegistry()
	result := prometheus.NewGaugeVec(prometheus.GaugeOpts{
		Namespace: "autolive",
		Subsystem: "migration",
		Name:      "result",
		Help:      "Latest one-shot database migration result; exactly one bounded status is 1.",
	}, []string{"status"})
	durationMetric := prometheus.NewGauge(prometheus.GaugeOpts{
		Namespace: "autolive",
		Subsystem: "migration",
		Name:      "duration_seconds",
		Help:      "Duration of the latest one-shot database migration in seconds.",
	})
	timeoutMetric := prometheus.NewGauge(prometheus.GaugeOpts{
		Namespace: "autolive",
		Subsystem: "migration",
		Name:      "timeout_seconds",
		Help:      "Configured timeout budget for the latest one-shot database migration in seconds.",
	})
	versionMetric := prometheus.NewGauge(prometheus.GaugeOpts{
		Namespace: "autolive",
		Subsystem: "migration",
		Name:      "version",
		Help:      "Target schema migration version required by the application.",
	})
	if err := registry.Register(result); err != nil {
		return nil, fmt.Errorf("register migration result metric: %w", err)
	}
	if err := registry.Register(durationMetric); err != nil {
		return nil, fmt.Errorf("register migration duration metric: %w", err)
	}
	if err := registry.Register(timeoutMetric); err != nil {
		return nil, fmt.Errorf("register migration timeout metric: %w", err)
	}
	if err := registry.Register(versionMetric); err != nil {
		return nil, fmt.Errorf("register migration version metric: %w", err)
	}

	status := migrationMetricStatus(migrationErr)
	for _, candidate := range []string{
		migrationMetricStatusSuccess,
		migrationMetricStatusFailure,
		migrationMetricStatusTimeout,
		migrationMetricStatusCancelled,
	} {
		value := float64(0)
		if candidate == status {
			value = 1
		}
		result.WithLabelValues(candidate).Set(value)
	}
	if duration < 0 {
		duration = 0
	}
	if timeout < 0 {
		timeout = 0
	}
	durationMetric.Set(duration.Seconds())
	timeoutMetric.Set(timeout.Seconds())
	versionMetric.Set(float64(migrations.LatestVersion))

	families, err := registry.Gather()
	if err != nil {
		return nil, fmt.Errorf("gather migration metrics: %w", err)
	}
	var output bytes.Buffer
	encoder := expfmt.NewEncoder(&output, expfmt.NewFormat(expfmt.TypeTextPlain))
	for _, family := range families {
		if err := encoder.Encode(family); err != nil {
			return nil, fmt.Errorf("encode migration metrics: %w", err)
		}
	}
	if closer, ok := encoder.(expfmt.Closer); ok {
		if err := closer.Close(); err != nil {
			return nil, fmt.Errorf("close migration metrics encoder: %w", err)
		}
	}
	return output.Bytes(), nil
}

func migrationMetricStatus(err error) string {
	if err == nil {
		return migrationMetricStatusSuccess
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return migrationMetricStatusTimeout
	}
	if errors.Is(err, context.Canceled) {
		return migrationMetricStatusCancelled
	}
	return migrationMetricStatusFailure
}

// writeMigrationMetricsFile publishes a complete exposition with a same
// directory temporary file and rename. Readers therefore see either the old
// snapshot or the new complete snapshot, never a partial write.
func writeMigrationMetricsFile(path string, content []byte) error {
	if path == "" {
		return nil
	}
	directory := filepath.Dir(path)
	file, err := os.CreateTemp(directory, ".migration-metrics-*")
	if err != nil {
		return fmt.Errorf("create migration metrics temp file: %w", err)
	}
	temporaryPath := file.Name()
	removeTemporary := true
	defer func() {
		if removeTemporary {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := file.Chmod(0o640); err != nil {
		_ = file.Close()
		return fmt.Errorf("set migration metrics permissions: %w", err)
	}
	if _, err := file.Write(content); err != nil {
		_ = file.Close()
		return fmt.Errorf("write migration metrics: %w", err)
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return fmt.Errorf("sync migration metrics: %w", err)
	}
	if err := file.Close(); err != nil {
		return fmt.Errorf("close migration metrics: %w", err)
	}
	if err := os.Rename(temporaryPath, path); err != nil {
		return fmt.Errorf("publish migration metrics: %w", err)
	}
	removeTemporary = false
	return nil
}
