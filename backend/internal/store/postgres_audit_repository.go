package store

import (
	"context"
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
	id, err := newRepositoryID("audit")
	if err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("generate normalized audit id: %w", err))
	}
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO audit_logs (
			id, actor_user_id, device_id, action, resource_type, resource_id,
			request_id, outcome, status_code, error_code, payload, created_at
		) VALUES ($1, NULLIF($2, ''), NULLIF($3, ''), $4, $5, NULLIF($6, ''), NULLIF($7, ''), $8, $9, NULLIF($10, ''), '{}'::jsonb, $11)
	`, id, input.ActorUserID, input.DeviceID, input.Action, input.TargetType, input.TargetID, input.RequestID, input.Outcome, input.StatusCode, input.ErrorCode, s.Now()); err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("insert normalized audit: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit normalized audit", err)
	}
	return nil
}

func normalizeAuditInput(input controlplane.AuditLogInput) (controlplane.AuditLogInput, error) {
	input.ActorUserID = strings.TrimSpace(input.ActorUserID)
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
	if input.Action == "" || input.TargetType == "" || len(input.Action) > 512 || len(input.TargetType) > 128 || len(input.TargetID) > 128 || len(input.ErrorCode) > 128 || len(input.RequestID) > 128 || input.StatusCode < 0 || input.StatusCode > 599 {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	if input.Outcome != "success" && input.Outcome != "failure" && input.Outcome != "unknown" {
		return controlplane.AuditLogInput{}, controlplane.ErrInvalidRequest
	}
	return input, nil
}
