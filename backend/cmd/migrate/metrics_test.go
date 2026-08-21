package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"autoLive/backend/migrations"
)

func TestRenderMigrationMetricsSuccessUsesFixedLowCardinalityFields(t *testing.T) {
	content, err := renderMigrationMetrics(nil, 1500*time.Millisecond, 30*time.Second)
	if err != nil {
		t.Fatalf("renderMigrationMetrics() error = %v", err)
	}
	text := string(content)
	for _, expected := range []string{
		`autolive_migration_result{status="success"} 1`,
		`autolive_migration_result{status="failure"} 0`,
		`autolive_migration_result{status="timeout"} 0`,
		`autolive_migration_result{status="cancelled"} 0`,
		"autolive_migration_duration_seconds 1.5",
		"autolive_migration_timeout_seconds 30",
		"autolive_migration_version " + strconv.Itoa(migrations.LatestVersion),
	} {
		if !strings.Contains(text, expected) {
			t.Fatalf("metrics missing %q:\n%s", expected, text)
		}
	}
	if strings.Contains(text, "postgres://") || strings.Contains(text, "secret") || strings.Contains(text, "error") {
		t.Fatalf("metrics contain sensitive/high-cardinality data:\n%s", text)
	}
}

func TestRenderMigrationMetricsTimeoutUsesTimeoutStatus(t *testing.T) {
	content, err := renderMigrationMetrics(
		wrappedContextError{err: context.DeadlineExceeded},
		2*time.Second,
		5*time.Second,
	)
	if err != nil {
		t.Fatalf("renderMigrationMetrics() error = %v", err)
	}
	text := string(content)
	if !strings.Contains(text, `autolive_migration_result{status="timeout"} 1`) {
		t.Fatalf("timeout status missing:\n%s", text)
	}
	if !strings.Contains(text, `autolive_migration_result{status="success"} 0`) {
		t.Fatalf("success status should be zero:\n%s", text)
	}
}

func TestWriteMigrationMetricsFilePublishesCompleteSnapshots(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "migration.prom")
	first := []byte("# TYPE first gauge\nfirst 1\n")
	second := []byte("# TYPE second gauge\nsecond 2\n")
	if err := writeMigrationMetricsFile(path, first); err != nil {
		t.Fatalf("first write error = %v", err)
	}
	if err := writeMigrationMetricsFile(path, second); err != nil {
		t.Fatalf("second write error = %v", err)
	}
	content, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	if string(content) != string(second) {
		t.Fatalf("published content = %q, want %q", content, second)
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatalf("ReadDir() error = %v", err)
	}
	if len(entries) != 1 || entries[0].Name() != "migration.prom" {
		t.Fatalf("temporary files remain after publish: %+v", entries)
	}
}

func TestWriteMigrationMetricsFileRejectsMissingDirectory(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing", "migration.prom")
	if err := writeMigrationMetricsFile(path, []byte("metrics\n")); err == nil {
		t.Fatal("writeMigrationMetricsFile() error = nil, want missing directory error")
	}
}

func TestMigrationMetricStatusClassifiesWrappedContextErrors(t *testing.T) {
	if got := migrationMetricStatus(wrappedContextError{err: context.DeadlineExceeded}); got != migrationMetricStatusTimeout {
		t.Fatalf("deadline status = %q, want %q", got, migrationMetricStatusTimeout)
	}
	if got := migrationMetricStatus(wrappedContextError{err: context.Canceled}); got != migrationMetricStatusCancelled {
		t.Fatalf("cancel status = %q, want %q", got, migrationMetricStatusCancelled)
	}
	if got := migrationMetricStatus(errors.New("database unavailable")); got != migrationMetricStatusFailure {
		t.Fatalf("failure status = %q, want %q", got, migrationMetricStatusFailure)
	}
}

type wrappedContextError struct {
	err error
}

func (e wrappedContextError) Error() string { return "wrapped migration error" }
func (e wrappedContextError) Unwrap() error { return e.err }
