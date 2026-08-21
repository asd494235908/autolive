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
		var raw sql.NullString
		err := tx.QueryRowContext(ctx, "SELECT product FROM "+table+" WHERE id = $1", id).Scan(&raw)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return postgresOperationError(ctx, fmt.Errorf("validate audit %s product: %w", table, err))
		}
		product, err := normalizedAuditProduct(raw)
		if err != nil {
			return err
		}
		if product != input.Product {
			return controlplane.ErrForbidden
		}
		return nil
	}
	if input.DeviceID != "" {
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
		"model_lease":     "model_leases",
		"model_usage":     "model_usage_records",
	}[input.TargetType]
	if table == "" {
		return nil
	}
	return check(table, input.TargetID)
}
