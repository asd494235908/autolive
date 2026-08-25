package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ActivationPageReader = (*PostgresRepository)(nil)
var _ ProductActivationPageReader = (*PostgresRepository)(nil)

// ListActivationCodesPage reads a bounded, redacted normalized page. Expiry
// is derived from the repository clock so a read never needs to rewrite a
// status row merely because time moved forward.
func (s *PostgresRepository) ListActivationCodesPage(ctx context.Context, offset, limit int) (ActivationCodePage, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return ActivationCodePage{}, errors.New("normalized activation page reader requires normalized read source")
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return ActivationCodePage{}, err
	}
	now := s.Now()
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ActivationCodePage, error) {
		var page ActivationCodePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM activation_codes`).Scan(&page.Total); err != nil {
			return ActivationCodePage{}, err
		}
		rows, err := tx.QueryContext(ctx, `
			SELECT id, product, bound_user_id, code_prefix,
			       CASE WHEN status IN ($1, $4) AND expires_at IS NOT NULL AND expires_at <= $2
			            THEN $3 ELSE status END AS status,
			       expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices
			FROM activation_codes
			ORDER BY id
			LIMIT $5 OFFSET $6
		`, controlplane.ActivationCodeStatusActive, now, controlplane.ActivationCodeStatusExpired, controlplane.ActivationCodeStatusUsed, limit, offset)
		if err != nil {
			return ActivationCodePage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			code, err := scanActivationCodeRow(rows)
			if err != nil {
				return ActivationCodePage{}, err
			}
			page.Items = append(page.Items, code)
		}
		if err := rows.Err(); err != nil {
			return ActivationCodePage{}, fmt.Errorf("scan normalized activation page: %w", err)
		}
		return page, nil
	})
}

func (s *PostgresRepository) ListActivationCodesPageForProduct(ctx context.Context, offset, limit int, product controlplane.ProductCode) (ActivationCodePage, error) {
	if s.modelReadSource != ModelReadSourceNormalized || !product.Valid() {
		return ActivationCodePage{}, controlplane.ErrInvalidRequest
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return ActivationCodePage{}, err
	}
	now := s.Now()
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ActivationCodePage, error) {
		var page ActivationCodePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM activation_codes WHERE product = $1`, product).Scan(&page.Total); err != nil {
			return ActivationCodePage{}, err
		}
		rows, err := tx.QueryContext(ctx, `
			SELECT id, product, bound_user_id, code_prefix,
			       CASE WHEN status IN ($1, $4) AND expires_at IS NOT NULL AND expires_at <= $2
			            THEN $3 ELSE status END AS status,
			       expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices
			FROM activation_codes
			WHERE product = $5
			ORDER BY id
			LIMIT $6 OFFSET $7
		`, controlplane.ActivationCodeStatusActive, now, controlplane.ActivationCodeStatusExpired, controlplane.ActivationCodeStatusUsed, product, limit, offset)
		if err != nil {
			return ActivationCodePage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			code, err := scanActivationCodeRowForProduct(rows, product)
			if err != nil {
				return ActivationCodePage{}, err
			}
			page.Items = append(page.Items, code)
		}
		if err := rows.Err(); err != nil {
			return ActivationCodePage{}, fmt.Errorf("scan product-scoped normalized activation page: %w", err)
		}
		return page, nil
	})
}

func scanActivationCodeRow(scanner interface{ Scan(dest ...any) error }) (controlplane.ActivationCode, error) {
	var (
		code                                      controlplane.ActivationCode
		product                                   sql.NullString
		expiresAt, usedAt                         sql.NullTime
		boundUserID, usedByUserID, usedByDeviceID sql.NullString
		maxDevices, boundDevices                  int
	)
	if err := scanner.Scan(&code.ID, &product, &boundUserID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &maxDevices, &boundDevices); err != nil {
		return controlplane.ActivationCode{}, err
	}
	var err error
	code.Product, err = normalizedAuditProduct(product)
	if err != nil {
		return controlplane.ActivationCode{}, err
	}
	code.MaxDevices = maxDevices
	code.BoundDevices = boundDevices
	code.UserID = boundUserID.String
	if expiresAt.Valid {
		code.ExpiresAt = expiresAt.Time.UTC().Format(time.RFC3339)
	}
	if usedAt.Valid {
		code.UsedAt = usedAt.Time.UTC().Format(time.RFC3339)
	}
	code.UsedByUserID = usedByUserID.String
	code.UsedByDeviceID = usedByDeviceID.String
	return code, nil
}

func scanActivationCodeRowForProduct(scanner interface{ Scan(dest ...any) error }, product controlplane.ProductCode) (controlplane.ActivationCode, error) {
	var (
		code                                      controlplane.ActivationCode
		storedProduct                             string
		expiresAt, usedAt                         sql.NullTime
		boundUserID, usedByUserID, usedByDeviceID sql.NullString
	)
	if err := scanner.Scan(&code.ID, &storedProduct, &boundUserID, &code.CodePrefix, &code.Status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &code.MaxDevices, &code.BoundDevices); err != nil {
		return controlplane.ActivationCode{}, err
	}
	parsedProduct, err := controlplane.ParseProductCode(storedProduct)
	if err != nil || parsedProduct != product {
		return controlplane.ActivationCode{}, controlplane.ErrForbidden
	}
	code.Product = parsedProduct
	code.UserID = boundUserID.String
	if expiresAt.Valid {
		code.ExpiresAt = expiresAt.Time.UTC().Format(time.RFC3339)
	}
	if usedAt.Valid {
		code.UsedAt = usedAt.Time.UTC().Format(time.RFC3339)
	}
	code.UsedByUserID = usedByUserID.String
	code.UsedByDeviceID = usedByDeviceID.String
	return code, nil
}
