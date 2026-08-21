package store

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"autoLive/backend/internal/controlplane"
)

var _ DeviceReader = (*PostgresRepository)(nil)

var ErrNormalizedDeviceReaderRequired = errors.New("normalized device reader is required")

// GetDevice reads one normalized device row without loading the legacy
// control-plane snapshot. Online state is derived by the service boundary.
func (s *PostgresRepository) GetDevice(ctx context.Context, deviceID string) (controlplane.DeviceSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.DeviceSummary{}, ErrNormalizedDeviceReaderRequired
	}
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (controlplane.DeviceSummary, error) {
		row := tx.QueryRowContext(ctx, devicePageQuery+` WHERE id = $1`, deviceID)
		device, err := scanDeviceSummary(row)
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		return device, err
	})
}

// GetOwnedDevice resolves the authenticated user's device. An empty device ID
// selects the most recently seen owned device, matching the compatibility
// profile behavior without consulting the snapshot.
func (s *PostgresRepository) GetOwnedDevice(ctx context.Context, userID, deviceID string) (controlplane.DeviceSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.DeviceSummary{}, ErrNormalizedDeviceReaderRequired
	}
	userID = strings.TrimSpace(userID)
	deviceID = strings.TrimSpace(deviceID)
	if userID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrUserNotFound
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (controlplane.DeviceSummary, error) {
		var query string
		var args []any
		if deviceID == "" {
			query = devicePageQuery + `
				WHERE user_id = $1
				ORDER BY last_heartbeat_at DESC NULLS LAST, id DESC
				LIMIT 1`
			args = []any{userID}
		} else {
			query = devicePageQuery + ` WHERE user_id = $1 AND id = $2`
			args = []any{userID, deviceID}
		}
		row := tx.QueryRowContext(ctx, query, args...)
		device, err := scanDeviceSummary(row)
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		return device, err
	})
}
