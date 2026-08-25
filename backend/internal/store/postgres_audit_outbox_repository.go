package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ AuditOutboxRepository = (*PostgresRepository)(nil)

const auditOutboxProcessingGrace = time.Minute

// RecordAuditWithOutbox commits a redacted audit event to the durable queue
// before attempting delivery. A request-id based dedupe key makes an HTTP
// retry safe; events without a request id remain distinct.
func (s *PostgresRepository) RecordAuditWithOutbox(ctx context.Context, input controlplane.AuditLogInput) error {
	if s.modelReadSource != ModelReadSourceNormalized {
		return errors.New("normalized audit outbox requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	now := s.Now()
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	input, err = normalizeAuditInput(input)
	if err != nil {
		return err
	}
	if err := s.enqueueAuditOutboxTx(operationCtx, tx, input, now); err != nil {
		return postgresOperationError(operationCtx, err)
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit audit outbox", err)
	}
	_, err = s.DispatchAuditOutbox(ctx, 1)
	return err
}

// enqueueAuditOutboxTx is the transaction-local half of the Outbox contract.
// Callers that already own a business transaction use this helper so the
// business row and its durable audit event commit or roll back together.
func (s *PostgresRepository) enqueueAuditOutboxTx(ctx context.Context, tx *sql.Tx, input controlplane.AuditLogInput, now time.Time) error {
	if tx == nil {
		return errors.New("audit outbox transaction is required")
	}
	input, err := normalizeAuditInput(input)
	if err != nil {
		return err
	}
	if err := validateNormalizedAuditTargetProduct(ctx, tx, input); err != nil {
		return err
	}
	outboxID, err := newRepositoryID("audit_outbox")
	if err != nil {
		return fmt.Errorf("generate audit outbox id: %w", err)
	}
	dedupeKey := outboxID
	if input.RequestID != "" {
		dedupeKey = "audit-request:" + input.RequestID
	}
	payload, err := json.Marshal(input)
	if err != nil {
		return fmt.Errorf("marshal audit outbox payload: %w", err)
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO audit_outbox (
			id, product, dedupe_key, payload, status, attempts, next_attempt_at,
			created_at, updated_at
		) VALUES ($1, $2, $3, $4::jsonb, 'pending', 0, $5, $5, $5)
		ON CONFLICT (dedupe_key) DO NOTHING
	`, outboxID, input.Product, dedupeKey, payload, now); err != nil {
		return fmt.Errorf("enqueue audit outbox: %w", err)
	}
	return nil
}

// DispatchAuditOutbox claims a bounded set of pending or stale processing
// rows, delivers each row idempotently, and leaves failed rows retryable.
func (s *PostgresRepository) DispatchAuditOutbox(ctx context.Context, batchSize int) (int64, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return 0, errors.New("normalized audit outbox requires normalized read source")
	}
	if ctx == nil {
		return 0, controlplane.ErrInvalidRequest
	}
	if batchSize < 1 || batchSize > MaxRetentionCleanupBatchSize {
		return 0, errors.New("audit outbox batch size is out of range")
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	now := s.Now()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	rows, err := tx.QueryContext(operationCtx, `
		SELECT id, payload, product
		FROM audit_outbox
		WHERE next_attempt_at <= $1
		  AND (status = 'pending' OR (status = 'processing' AND updated_at <= $2))
		ORDER BY next_attempt_at, created_at, id
		FOR UPDATE SKIP LOCKED
		LIMIT $3
	`, now, now.Add(-auditOutboxProcessingGrace), batchSize)
	if err != nil {
		return 0, postgresOperationError(operationCtx, fmt.Errorf("claim audit outbox: %w", err))
	}
	type row struct {
		id      string
		payload []byte
		product sql.NullString
	}
	claimed := make([]row, 0, batchSize)
	for rows.Next() {
		var item row
		if err := rows.Scan(&item.id, &item.payload, &item.product); err != nil {
			_ = rows.Close()
			return 0, postgresOperationError(operationCtx, fmt.Errorf("scan audit outbox: %w", err))
		}
		claimed = append(claimed, item)
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return 0, postgresOperationError(operationCtx, fmt.Errorf("iterate audit outbox: %w", err))
	}
	if err := rows.Close(); err != nil {
		return 0, postgresOperationError(operationCtx, fmt.Errorf("close audit outbox rows: %w", err))
	}
	for _, item := range claimed {
		if _, err := tx.ExecContext(operationCtx, `
			UPDATE audit_outbox SET status = 'processing', updated_at = $2
			WHERE id = $1
		`, item.id, now); err != nil {
			return 0, postgresOperationError(operationCtx, fmt.Errorf("claim audit outbox row: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, postgresCommitError(operationCtx, "commit audit outbox claims", err)
	}

	var delivered int64
	for _, item := range claimed {
		if err := ctx.Err(); err != nil {
			return delivered, err
		}
		if err := s.deliverAuditOutboxRow(ctx, item.id, item.payload, item.product, now); err != nil {
			return delivered, err
		}
		delivered++
	}
	return delivered, nil
}

func (s *PostgresRepository) deliverAuditOutboxRow(ctx context.Context, outboxID string, payload []byte, storedProduct sql.NullString, now time.Time) error {
	var input controlplane.AuditLogInput
	if err := json.Unmarshal(payload, &input); err != nil {
		return s.markAuditOutboxRetry(ctx, outboxID, fmt.Errorf("decode audit outbox payload: %w", err), now)
	}
	if !storedProduct.Valid || strings.TrimSpace(storedProduct.String) == "" {
		storedProduct = sql.NullString{String: string(controlplane.ProductAutoLive), Valid: true}
	}
	product, productErr := controlplane.ParseProductCode(storedProduct.String)
	if productErr != nil {
		return s.markAuditOutboxRetry(ctx, outboxID, fmt.Errorf("validate stored audit outbox product: %w", productErr), now)
	}
	if input.Product == "" {
		input.Product = product
	} else if input.Product != product {
		return s.markAuditOutboxRetry(ctx, outboxID, controlplane.ErrForbidden, now)
	}
	input, err := normalizeAuditInput(input)
	if err != nil {
		return s.markAuditOutboxRetry(ctx, outboxID, fmt.Errorf("validate audit outbox payload: %w", err), now)
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	input, err = clearMissingFailureAuditDeviceReference(operationCtx, tx, input)
	if err != nil {
		_ = tx.Rollback()
		return s.markAuditOutboxRetry(ctx, outboxID, err, now)
	}
	auditID := "audit_" + strings.TrimPrefix(outboxID, "audit_outbox_")
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO audit_logs (
			id, product, actor_user_id, device_id, action, resource_type, resource_id,
			request_id, outcome, status_code, error_code, payload, created_at
		) VALUES ($1, $2, NULLIF($3, ''), NULLIF($4, ''), $5, $6, NULLIF($7, ''), NULLIF($8, ''), $9, $10, NULLIF($11, ''), '{}'::jsonb, $12)
		ON CONFLICT (id) DO NOTHING
		`, auditID, input.Product, input.ActorUserID, input.DeviceID, input.Action, input.TargetType, input.TargetID, input.RequestID, input.Outcome, input.StatusCode, input.ErrorCode, now); err != nil {
		_ = tx.Rollback()
		return s.markAuditOutboxRetry(ctx, outboxID, fmt.Errorf("deliver audit outbox: %w", err), now)
	}
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE audit_outbox
		SET status = 'sent', delivered_at = $2, updated_at = $2, last_error = NULL
		WHERE id = $1 AND status = 'processing'
	`, outboxID, now); err != nil {
		_ = tx.Rollback()
		return s.markAuditOutboxRetry(ctx, outboxID, fmt.Errorf("mark audit outbox sent: %w", err), now)
	}
	if err := tx.Commit(); err != nil {
		return s.markAuditOutboxRetry(ctx, outboxID, postgresCommitError(operationCtx, "commit audit outbox delivery", err), now)
	}
	return nil
}

func (s *PostgresRepository) markAuditOutboxRetry(ctx context.Context, outboxID string, deliveryErr error, now time.Time) error {
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, errors.Join(deliveryErr, err))
	}
	defer func() { _ = tx.Rollback() }()
	message := deliveryErr.Error()
	if len(message) > 256 {
		message = message[:256]
	}
	_, updateErr := tx.ExecContext(operationCtx, `
		UPDATE audit_outbox
		SET status = 'pending', attempts = attempts + 1,
		    next_attempt_at = $2, updated_at = $2, last_error = $3
		WHERE id = $1
	`, outboxID, now.Add(time.Minute), message)
	if updateErr != nil {
		return postgresOperationError(operationCtx, errors.Join(deliveryErr, fmt.Errorf("mark audit outbox retry: %w", updateErr)))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit audit outbox retry", errors.Join(deliveryErr, err))
	}
	return deliveryErr
}
