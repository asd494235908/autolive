//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedDeviceReaderUsesDeviceTables(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Second)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("device_reader_user_%d", suffix)
	deviceID := fmt.Sprintf("device_reader_device_%d", suffix)
	username := fmt.Sprintf("device-reader-%d", suffix)
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "not-used-by-reader", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed normalized device user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, device_name, platform, client_version, status, last_heartbeat_at, current_media_name, playback_state)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
	`, deviceID, userID, "device-key/"+deviceID, "Studio", "windows", "1.2.3", controlplane.DeviceStatusActive, now, "demo.mp4", "playing"); err != nil {
		t.Fatalf("seed normalized device: %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	device, err := repository.GetDevice(ctx, deviceID)
	if err != nil {
		t.Fatalf("GetDevice() error = %v", err)
	}
	if device.ID != deviceID || device.UserID != userID || device.CurrentMediaName != "demo.mp4" || device.PlaybackState != "playing" {
		t.Fatalf("device = %+v", device)
	}
	owned, err := repository.GetOwnedDevice(ctx, userID, deviceID)
	if err != nil || owned.ID != deviceID {
		t.Fatalf("GetOwnedDevice(explicit) = %+v err:%v", owned, err)
	}
	latest, err := repository.GetOwnedDevice(ctx, userID, "")
	if err != nil || latest.ID != deviceID {
		t.Fatalf("GetOwnedDevice(latest) = %+v err:%v", latest, err)
	}
}
