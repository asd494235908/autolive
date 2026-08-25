package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"

	"autoLive/backend/internal/controlplane"
)

var _ AuditRepository = (*PostgresRepository)(nil)

func (s *PostgresRepository) RecordAudit(ctx context.Context, input controlplane.AuditLogInput) error {
	if s.modelReadSource != ModelReadSourceNormalized {
		return errors.New("normalized audit requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	input, err := normalizeAuditInput(input)
	if err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := validateNormalizedAuditTargetProduct(operationCtx, tx, input); err != nil {
		return err
	}
	input, err = clearMissingFailureAuditDeviceReference(operationCtx, tx, input)
	if err != nil {
		return err
	}
	id, err := newRepositoryID("audit")
	if err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("generate normalized audit id: %w", err))
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO audit_logs (
			id, product, actor_user_id, device_id, action, resource_type, resource_id,
			request_id, outcome, status_code, error_code, payload, created_at
		) VALUES ($1, $2, NULLIF($3, ''), NULLIF($4, ''), $5, $6, NULLIF($7, ''), NULLIF($8, ''), $9, $10, NULLIF($11, ''), '{}'::jsonb, $12)
		`, id, input.Product, input.ActorUserID, input.DeviceID, input.Action, input.TargetType, input.TargetID, input.RequestID, input.Outcome, input.StatusCode, input.ErrorCode, s.Now()); err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("insert normalized audit: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit normalized audit", err)
	}
	return nil
}

func clearMissingFailureAuditDeviceReference(ctx context.Context, tx *sql.Tx, input controlplane.AuditLogInput) (controlplane.AuditLogInput, error) {
	if input.Outcome != "failure" || input.DeviceID == "" {
		return input, nil
	}
	var deviceID string
	err := tx.QueryRowContext(ctx, "SELECT id FROM devices WHERE id = $1 AND product = $2 LIMIT 1", input.DeviceID, input.Product).Scan(&deviceID)
	if err == nil {
		return input, nil
	}
	if errors.Is(err, sql.ErrNoRows) {
		input.DeviceID = ""
		return input, nil
	}
	return controlplane.AuditLogInput{}, postgresOperationError(ctx, fmt.Errorf("validate failure audit device reference: %w", err))
}

func normalizeAuditInput(input controlplane.AuditLogInput) (controlplane.AuditLogInput, error) {
	input.ActorUserID = strings.TrimSpace(input.ActorUserID)
	input.Product = controlplane.ProductCode(strings.TrimSpace(string(input.Product)))
	input.DeviceID = strings.TrimSpace(input.DeviceID)
	input.Action = strings.TrimSpace(input.Action)
	input.TargetType = strings.TrimSpace(input.TargetType)
	input.TargetID = strings.TrimSpace(input.TargetID)
	input.Outcome = strings.TrimSpace(input.Outcome)
	input.ErrorCode = strings.TrimSpace(input.ErrorCode)
	input.RequestID = strings.TrimSpace(input.RequestID)
	if input.Outcome == "" {
		input.Outcome = "unknown"
	}
	if input.Product == "" {
		input.Product = controlplane.ProductAutoLive
	} else if !input.Product.Valid() {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	if input.Action == "" || input.TargetType == "" || len(input.Action) > 512 || len(input.TargetType) > 128 || len(input.TargetID) > 128 || len(input.ErrorCode) > 128 || len(input.RequestID) > 128 || input.StatusCode < 0 || input.StatusCode > 599 {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	if input.Outcome != "success" && input.Outcome != "failure" && input.Outcome != "unknown" {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	return input, nil
}

func normalizeAuditInputForProduct(input controlplane.AuditLogInput, product controlplane.ProductCode) (controlplane.AuditLogInput, error) {
	if !product.Valid() {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	input.Product = controlplane.ProductCode(strings.TrimSpace(string(input.Product)))
	if input.Product == "" {
		input.Product = product
	} else if input.Product != product {
		return controlplane.AuditLogInput{}, controlplane.ErrForbidden
	}
	return normalizeAuditInput(input)
}

func normalizeOptionalAuditInputForProduct(input controlplane.AuditLogInput, product controlplane.ProductCode) (controlplane.AuditLogInput, error) {
	if strings.TrimSpace(input.Action) == "" {
		return input, nil
	}
	return normalizeAuditInputForProduct(input, product)
}

func validateNormalizedAuditTargetProduct(ctx context.Context, tx *sql.Tx, input controlplane.AuditLogInput) error {
	check := func(table, id string) error {
		var exists int
		err := tx.QueryRowContext(ctx, "SELECT 1 FROM "+table+" WHERE id = $1 AND product = $2", id, input.Product).Scan(&exists)
		if err == nil {
			return nil
		}
		if errors.Is(err, sql.ErrNoRows) {
			// A product-mismatched, NULL, or otherwise malformed resource must
			// not be treated as an absent target. The second query still uses a
			// fixed product predicate and only checks for a conflicting row.
			conflictErr := tx.QueryRowContext(ctx, "SELECT 1 FROM "+table+" WHERE id = $1 AND (product IS NULL OR product <> $2)", id, input.Product).Scan(&exists)
			if conflictErr == nil {
				return controlplane.ErrForbidden
			}
			if errors.Is(conflictErr, sql.ErrNoRows) {
				if input.Outcome == "failure" {
					return nil
				}
				return controlplane.ErrForbidden
			}
			return postgresOperationError(ctx, fmt.Errorf("validate conflicting audit %s product: %w", table, conflictErr))
		}
		return postgresOperationError(ctx, fmt.Errorf("validate audit %s product: %w", table, err))
	}
	if input.DeviceID != "" && !(input.TargetType == "device" && input.TargetID == input.DeviceID) {
		if err := check("devices", input.DeviceID); err != nil {
			return err
		}
	}
	if input.TargetID == "" {
		return nil
	}
	table := map[string]string{
		"device":          "devices",
		"activation_code": "activation_codes",
		"model_account":   "model_accounts",
		"model_lease":     "model_leases",
		"model_usage":     "model_usage_records",
	}[input.TargetType]
	if table == "" {
		return nil
	}
	return check(table, input.TargetID)
}
