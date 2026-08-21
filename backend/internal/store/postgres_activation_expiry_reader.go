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
			SELECT expires_at
			FROM activation_codes
			WHERE used_by_user_id = $1 AND used_by_device_id = $2 AND status = $3
			ORDER BY used_at DESC NULLS LAST, id DESC
			LIMIT 1
		`, userID, deviceID, controlplane.ActivationCodeStatusUsed).Scan(&expiresAt)
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
		condition, productArgs := normalizedProductFilter("product", product, 4)
		query := `
			SELECT expires_at
			FROM activation_codes
			WHERE used_by_user_id = $1 AND used_by_device_id = $2 AND status = $3 AND ` + condition + `
			ORDER BY used_at DESC NULLS LAST, id DESC
			LIMIT 1
		`
		args := append([]any{userID, deviceID, controlplane.ActivationCodeStatusUsed}, productArgs...)
		var expiresAt time.Time
		err := tx.QueryRowContext(ctx, query, args...).Scan(&expiresAt)
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
