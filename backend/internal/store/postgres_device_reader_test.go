package store

import (
	"context"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryGetDeviceReadsNormalizedRow(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).WithArgs("dev_1").WillReturnRows(
		sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_name", "platform", "client_version", "status",
			"disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores",
			"runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at",
		}).AddRow("dev_1", "usr_1", string(controlplane.ProductAutoLive), "Studio", "windows", "1.2.3", controlplane.DeviceStatusActive,
			int64(100), int64(200), int64(150), 8, "Windows", "11", "kernel", "demo.mp4", "playing", now.Add(-time.Minute)),
	)
	mock.ExpectCommit()

	device, err := repository.GetDevice(context.Background(), "dev_1")
	if err != nil {
		t.Fatalf("GetDevice() error = %v", err)
	}
	if device.ID != "dev_1" || device.UserID != "usr_1" || device.CurrentMediaName != "demo.mp4" || device.PlaybackState != "playing" {
		t.Fatalf("device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetDeviceForProductBindsProductInSQL(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("WHERE id = $1 AND product = $2")).
		WithArgs("dev_1", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_name", "platform", "client_version", "status",
			"disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores",
			"runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at",
		}).AddRow("dev_1", "usr_1", string(controlplane.ProductDouyinDesktop), "Studio", "windows", "1.2.3", controlplane.DeviceStatusActive,
			int64(100), int64(200), int64(150), 8, "Windows", "11", "kernel", "demo.mp4", "playing", now.Add(-time.Minute)))
	mock.ExpectCommit()

	device, err := repository.GetDeviceForProduct(context.Background(), "dev_1", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("GetDeviceForProduct() error = %v", err)
	}
	if device.ID != "dev_1" || device.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetOwnedDeviceSelectsLatestWhenDeviceMissing(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 15, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{
			"id", "user_id", "product", "device_name", "platform", "client_version", "status",
			"disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores",
			"runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at",
		}).AddRow("dev_2", "usr_1", string(controlplane.ProductAutoLive), "Studio", "windows", "1.2.3", controlplane.DeviceStatusActive,
			int64(100), int64(200), int64(150), 8, "Windows", "11", "kernel", "demo.mp4", "playing", now.Add(-time.Minute)),
	)
	mock.ExpectCommit()

	device, err := repository.GetOwnedDevice(context.Background(), "usr_1", "")
	if err != nil {
		t.Fatalf("GetOwnedDevice() error = %v", err)
	}
	if device.ID != "dev_2" || device.UserID != "usr_1" {
		t.Fatalf("device = %+v", device)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetOwnedDeviceMapsMissingDevice(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, user_id, product, device_name, platform, client_version, status")).WithArgs("usr_1", "missing").WillReturnRows(sqlmock.NewRows([]string{
		"id", "user_id", "product", "device_name", "platform", "client_version", "status",
		"disk_free_bytes", "memory_total_bytes", "memory_available_bytes", "cpu_logical_cores",
		"runtime_os_name", "runtime_os_version", "kernel_version", "current_media_name", "playback_state", "last_heartbeat_at",
	}))
	mock.ExpectRollback()
	_, err = repository.GetOwnedDevice(context.Background(), "usr_1", "missing")
	if !errors.Is(err, controlplane.ErrDeviceNotFound) {
		t.Fatalf("GetOwnedDevice() error = %v, want device not found", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
