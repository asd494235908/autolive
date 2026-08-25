package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ActivationExpiryReader = (*PostgresRepository)(nil)
var _ ProductActivationExpiryReader = (*PostgresRepository)(nil)

// GetActivationExpiry reads only the expiry timestamp of the code currently
// bound to the requested device. The plaintext activation code is never read.
func (s *PostgresRepository) GetActivationExpiry(ctx context.Context, userID, deviceID string) (*time.Time, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, errors.New("normalized activation expiry reader requires normalized read source")
	}
	userID = strings.TrimSpace(userID)
	deviceID = strings.TrimSpace(deviceID)
	if userID == "" || deviceID == "" {
		return nil, controlplane.ErrDeviceNotFound
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (*time.Time, error) {
		var expiresAt time.Time
		err := tx.QueryRowContext(ctx, `
			SELECT ac.expires_at
			FROM activation_device_bindings AS binding
			JOIN activation_codes AS ac ON ac.id = binding.activation_code_id
			WHERE binding.user_id = $1 AND binding.device_id = $2
			  AND ac.bound_user_id = $1
			ORDER BY binding.bound_at DESC, ac.id DESC
			LIMIT 1
		`, userID, deviceID).Scan(&expiresAt)
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		if err != nil {
			return nil, fmt.Errorf("read normalized activation expiry: %w", err)
		}
		expiresAt = expiresAt.UTC()
		return &expiresAt, nil
	})
}

func (s *PostgresRepository) GetActivationExpiryForProduct(ctx context.Context, userID, deviceID string, product controlplane.ProductCode) (*time.Time, error) {
	if s.modelReadSource != ModelReadSourceNormalized || !product.Valid() {
		return nil, errors.New("normalized activation expiry reader requires normalized read source")
	}
	userID = strings.TrimSpace(userID)
	deviceID = strings.TrimSpace(deviceID)
	if userID == "" || deviceID == "" {
		return nil, controlplane.ErrDeviceNotFound
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (*time.Time, error) {
		query := `
			SELECT ac.expires_at
			FROM activation_device_bindings AS binding
			JOIN activation_codes AS ac ON ac.id = binding.activation_code_id
			WHERE binding.user_id = $1 AND binding.device_id = $2 AND binding.product = $3
			  AND ac.bound_user_id = $1 AND ac.product = $3
			ORDER BY binding.bound_at DESC, ac.id DESC
			LIMIT 1
		`
		var expiresAt time.Time
		err := tx.QueryRowContext(ctx, query, userID, deviceID, product).Scan(&expiresAt)
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		if err != nil {
			return nil, fmt.Errorf("read normalized product activation expiry: %w", err)
		}
		expiresAt = expiresAt.UTC()
		return &expiresAt, nil
	})
}
