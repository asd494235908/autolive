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

var _ ModelUsageRepository = (*PostgresRepository)(nil)

func (s *PostgresRepository) RecordDirectLLMCall(ctx context.Context, record ModelUsageWriteRecord) (controlplane.ModelUsageRecord, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelUsageRecord{}, errors.New("normalized model usage requires normalized read source")
	}
	if ctx == nil {
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	record.DeviceID = strings.TrimSpace(record.DeviceID)
	explicitProduct := record.Product != ""
	record.Product = controlplane.ProductCode(strings.TrimSpace(string(record.Product)))
	if record.Product == "" {
		record.Product = controlplane.ProductAutoLive
	}
	record.RequestID = strings.TrimSpace(record.RequestID)
	record.Input.LeaseID = strings.TrimSpace(record.Input.LeaseID)
	record.Input.Provider = strings.TrimSpace(record.Input.Provider)
	record.Input.Model = strings.TrimSpace(record.Input.Model)
	record.Input.ClientCallID = strings.TrimSpace(record.Input.ClientCallID)
	record.Input.Status = strings.TrimSpace(record.Input.Status)
	record.Input.UsageSource = strings.TrimSpace(record.Input.UsageSource)
	record.Input.ErrorCode = strings.TrimSpace(record.Input.ErrorCode)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" || record.DeviceID == "" || !record.Product.Valid() || record.Input.LeaseID == "" || record.Input.ClientCallID == "" || record.Input.Provider == "" || record.Input.Model == "" {
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	var err error
	record.Audit, err = normalizeOptionalAuditInputForProduct(record.Audit, record.Product)
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if record.Input.InputTokens < 0 || record.Input.OutputTokens < 0 || record.Input.TotalTokens < 0 || record.Input.LatencyMS < 0 || record.Input.TotalTokens < record.Input.InputTokens+record.Input.OutputTokens || record.Input.UsageSource != "client_reported" {
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	switch record.Input.Status {
	case "succeeded", "failed", "timeout", "cancelled", "unknown":
	default:
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if user.Status != controlplane.UserStatusActive {
		return controlplane.ModelUsageRecord{}, controlplane.ErrUserDisabled
	}
	var device controlplane.DeviceSummary
	var exists bool
	if explicitProduct {
		device, exists, err = s.loadDeviceForUpdateWithProduct(operationCtx, tx, record.DeviceID, record.Product)
	} else {
		device, exists, err = s.loadDeviceForUpdate(operationCtx, tx, record.DeviceID)
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if !exists || device.UserID != record.UserID {
		return controlplane.ModelUsageRecord{}, controlplane.ErrDeviceNotFound
	}
	if device.Product != record.Product {
		return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
	}
	var lease controlplane.ModelLease
	if explicitProduct {
		lease, err = s.loadModelLeaseForUpdateWithProduct(operationCtx, tx, record.Input.LeaseID, record.Product)
	} else {
		lease, err = s.loadModelLeaseForUpdate(operationCtx, tx, record.Input.LeaseID)
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if explicitProduct && lease.Product != record.Product {
		return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
	}
	if lease.UserID != record.UserID || lease.DeviceID != device.ID {
		return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
	}
	if lease.Provider != record.Input.Provider || lease.Model != record.Input.Model {
		return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if explicitProduct {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, "", s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, "", s.Now())
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
		}
		var usage controlplane.ModelUsageRecord
		if explicitProduct {
			usage, err = s.loadModelUsageByIDWithProduct(operationCtx, tx, storedResourceID, record.Product)
		} else {
			usage, err = s.loadModelUsageByID(operationCtx, tx, storedResourceID)
		}
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		if explicitProduct && usage.Product != record.Product {
			return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			audit := record.Audit
			if strings.TrimSpace(audit.TargetID) == "" {
				audit.TargetID = usage.ID
			}
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue idempotent model usage audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelUsageRecord{}, postgresCommitError(operationCtx, "commit idempotent normalized model usage", err)
		}
		return usage, nil
	}
	var existing controlplane.ModelUsageRecord
	var found bool
	if explicitProduct {
		existing, found, err = s.loadModelUsageByClientCallWithProduct(operationCtx, tx, record.Input.LeaseID, record.Input.ClientCallID, record.Product)
	} else {
		existing, found, err = s.loadModelUsageByClientCall(operationCtx, tx, record.Input.LeaseID, record.Input.ClientCallID)
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if found {
		if explicitProduct && existing.Product != record.Product {
			return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
		}
		if !modelUsageMatchesInput(existing, record) {
			return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
		}
		if _, err := tx.ExecContext(operationCtx, `
			UPDATE idempotency_records SET resource_id = $3
			WHERE scope = $1 AND idempotency_key = $2
		`, record.Scope, record.IdempotencyKey, existing.ID); err != nil {
			return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("bind normalized model usage idempotency: %w", err))
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			audit := record.Audit
			if strings.TrimSpace(audit.TargetID) == "" {
				audit.TargetID = existing.ID
			}
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue duplicate model usage audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelUsageRecord{}, postgresCommitError(operationCtx, "commit duplicate normalized model usage", err)
		}
		return existing, nil
	}
	products := []controlplane.ProductCode(nil)
	if explicitProduct {
		products = []controlplane.ProductCode{record.Product}
	}
	policyLimit, policyConfigured, err := s.loadUserDailyTokenLimit(operationCtx, tx, record.UserID, products...)
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if policyConfigured && policyLimit > 0 {
		used, err := s.dailyModelUsageTokens(operationCtx, tx, "user_id", record.UserID, s.Now(), products...)
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		if used >= policyLimit {
			return controlplane.ModelUsageRecord{}, controlplane.ErrUserRecordedQuotaExceeded
		}
	}
	var account normalizedModelAccount
	if explicitProduct {
		account, err = s.loadModelAccountForUpdateWithProduct(operationCtx, tx, lease.AccountID, record.Product)
	} else {
		account, err = s.loadModelAccountForUpdate(operationCtx, tx, lease.AccountID)
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if explicitProduct && account.product != record.Product {
		return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
	}
	now := s.Now()
	usageID, err := newRepositoryID("usage")
	if err != nil {
		return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model usage id: %w", err))
	}
	insertQuery := `
		INSERT INTO model_usage_records (
			id, account_id, lease_id, user_id, device_id, provider, model,
			prompt_tokens, completion_tokens, total_tokens, latency_ms,
			request_id, client_call_id, usage_source, status, error_code, created_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NULLIF($16, ''), $17)`
	insertArgs := []any{usageID, lease.AccountID, record.Input.LeaseID, record.UserID, record.DeviceID, record.Input.Provider, record.Input.Model,
		record.Input.InputTokens, record.Input.OutputTokens, record.Input.TotalTokens, record.Input.LatencyMS,
		record.RequestID, record.Input.ClientCallID, record.Input.UsageSource, record.Input.Status, record.Input.ErrorCode, now}
	if explicitProduct {
		insertQuery = `
			INSERT INTO model_usage_records (
				id, product, account_id, lease_id, user_id, device_id, provider, model,
				prompt_tokens, completion_tokens, total_tokens, latency_ms,
				request_id, client_call_id, usage_source, status, error_code, created_at
			) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, NULLIF($17, ''), $18)`
		insertArgs = []any{usageID, record.Product, lease.AccountID, record.Input.LeaseID, record.UserID, record.DeviceID, record.Input.Provider, record.Input.Model,
			record.Input.InputTokens, record.Input.OutputTokens, record.Input.TotalTokens, record.Input.LatencyMS,
			record.RequestID, record.Input.ClientCallID, record.Input.UsageSource, record.Input.Status, record.Input.ErrorCode, now}
	}
	if _, err := tx.ExecContext(operationCtx, insertQuery, insertArgs...); err != nil {
		return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("insert normalized model usage: %w", err))
	}
	if account.status != controlplane.ModelAccountStatusDisabled && account.dailyLimit > 0 {
		used, err := s.dailyModelUsageTokens(operationCtx, tx, "account_id", lease.AccountID, now, products...)
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		if used >= account.dailyLimit {
			updateAccountQuery := `UPDATE model_accounts SET status = $2, updated_at = $3 WHERE id = $1 AND status <> $4`
			updateAccountArgs := []any{lease.AccountID, controlplane.ModelAccountStatusExhausted, now, controlplane.ModelAccountStatusDisabled}
			if explicitProduct {
				updateAccountQuery += " AND product = $5"
				updateAccountArgs = append(updateAccountArgs, record.Product)
			}
			if _, err := tx.ExecContext(operationCtx, updateAccountQuery, updateAccountArgs...); err != nil {
				return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("mark normalized model account exhausted: %w", err))
			}
		}
	}
	if _, err := tx.ExecContext(operationCtx, `
		UPDATE idempotency_records SET resource_id = $3
		WHERE scope = $1 AND idempotency_key = $2
	`, record.Scope, record.IdempotencyKey, usageID); err != nil {
		return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized model usage resource: %w", err))
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		audit := record.Audit
		if strings.TrimSpace(audit.TargetID) == "" {
			audit.TargetID = usageID
		}
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, now); err != nil {
			return controlplane.ModelUsageRecord{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model usage audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelUsageRecord{}, postgresCommitError(operationCtx, "commit normalized model usage", err)
	}
	return controlplane.ModelUsageRecord{
		ID: usageID, Product: record.Product, LeaseID: record.Input.LeaseID, ClientCallID: record.Input.ClientCallID, RequestID: record.RequestID,
		Provider: record.Input.Provider, Model: record.Input.Model, InputTokens: record.Input.InputTokens, OutputTokens: record.Input.OutputTokens,
		TotalTokens: record.Input.TotalTokens, LatencyMS: record.Input.LatencyMS, Status: record.Input.Status, UsageSource: record.Input.UsageSource,
		ErrorCode: record.Input.ErrorCode, CreatedAt: now.UTC().Format(time.RFC3339),
	}, nil
}

func (s *PostgresRepository) loadModelUsageByID(ctx context.Context, tx *sql.Tx, usageID string) (controlplane.ModelUsageRecord, error) {
	var usage controlplane.ModelUsageRecord
	var leaseID, errorCode sql.NullString
	var createdAt time.Time
	query := `
		SELECT id, lease_id, client_call_id, request_id, provider, model,
		       prompt_tokens, completion_tokens, total_tokens, latency_ms,
		       status, usage_source, error_code, created_at
		FROM model_usage_records
		WHERE id = $1
		FOR UPDATE
	`
	dest := []any{&usage.ID, &leaseID, &usage.ClientCallID, &usage.RequestID, &usage.Provider, &usage.Model, &usage.InputTokens, &usage.OutputTokens, &usage.TotalTokens, &usage.LatencyMS, &usage.Status, &usage.UsageSource, &errorCode, &createdAt}
	if err := tx.QueryRowContext(ctx, query, usageID).Scan(dest...); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
		}
		return controlplane.ModelUsageRecord{}, postgresOperationError(ctx, fmt.Errorf("load idempotent normalized model usage: %w", err))
	}
	usage.LeaseID = leaseID.String
	usage.ErrorCode = errorCode.String
	usage.Product = controlplane.ProductAutoLive
	usage.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return usage, nil
}

func (s *PostgresRepository) loadModelUsageByClientCall(ctx context.Context, tx *sql.Tx, leaseID, clientCallID string) (controlplane.ModelUsageRecord, bool, error) {
	var usage controlplane.ModelUsageRecord
	var storedLeaseID, errorCode sql.NullString
	var createdAt time.Time
	query := `
		SELECT id, lease_id, client_call_id, request_id, provider, model,
		       prompt_tokens, completion_tokens, total_tokens, latency_ms,
		       status, usage_source, error_code, created_at
		FROM model_usage_records
		WHERE lease_id = $1 AND client_call_id = $2
		FOR UPDATE
	`
	dest := []any{&usage.ID, &storedLeaseID, &usage.ClientCallID, &usage.RequestID, &usage.Provider, &usage.Model, &usage.InputTokens, &usage.OutputTokens, &usage.TotalTokens, &usage.LatencyMS, &usage.Status, &usage.UsageSource, &errorCode, &createdAt}
	err := tx.QueryRowContext(ctx, query, leaseID, clientCallID).Scan(dest...)
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.ModelUsageRecord{}, false, nil
	}
	if err != nil {
		return controlplane.ModelUsageRecord{}, false, postgresOperationError(ctx, fmt.Errorf("load duplicate normalized model usage: %w", err))
	}
	usage.LeaseID = storedLeaseID.String
	usage.ErrorCode = errorCode.String
	usage.Product = controlplane.ProductAutoLive
	usage.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return usage, true, nil
}

func (s *PostgresRepository) loadModelUsageByIDWithProduct(ctx context.Context, tx *sql.Tx, usageID string, product controlplane.ProductCode) (controlplane.ModelUsageRecord, error) {
	var usage controlplane.ModelUsageRecord
	var storedProduct, leaseID, errorCode sql.NullString
	var createdAt time.Time
	condition, productArgs := normalizedProductFilter("product", product, 2)
	query := `
		SELECT id, product, lease_id, client_call_id, request_id, provider, model,
		       prompt_tokens, completion_tokens, total_tokens, latency_ms,
		       status, usage_source, error_code, created_at
		FROM model_usage_records
		WHERE id = $1 AND ` + condition + `
		FOR UPDATE`
	args := append([]any{usageID}, productArgs...)
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&usage.ID, &storedProduct, &leaseID, &usage.ClientCallID, &usage.RequestID, &usage.Provider, &usage.Model, &usage.InputTokens, &usage.OutputTokens, &usage.TotalTokens, &usage.LatencyMS, &usage.Status, &usage.UsageSource, &errorCode, &createdAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
		}
		return controlplane.ModelUsageRecord{}, postgresOperationError(ctx, fmt.Errorf("load product normalized model usage: %w", err))
	}
	var err error
	usage.Product, err = normalizedStoredProduct(storedProduct)
	if err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	usage.LeaseID = leaseID.String
	usage.ErrorCode = errorCode.String
	usage.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return usage, nil
}

func (s *PostgresRepository) loadModelUsageByClientCallWithProduct(ctx context.Context, tx *sql.Tx, leaseID, clientCallID string, product controlplane.ProductCode) (controlplane.ModelUsageRecord, bool, error) {
	var usage controlplane.ModelUsageRecord
	var storedProduct, storedLeaseID, errorCode sql.NullString
	var createdAt time.Time
	condition, productArgs := normalizedProductFilter("product", product, 3)
	query := `
		SELECT id, product, lease_id, client_call_id, request_id, provider, model,
		       prompt_tokens, completion_tokens, total_tokens, latency_ms,
		       status, usage_source, error_code, created_at
		FROM model_usage_records
		WHERE lease_id = $1 AND client_call_id = $2 AND ` + condition + `
		FOR UPDATE`
	args := append([]any{leaseID, clientCallID}, productArgs...)
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&usage.ID, &storedProduct, &storedLeaseID, &usage.ClientCallID, &usage.RequestID, &usage.Provider, &usage.Model, &usage.InputTokens, &usage.OutputTokens, &usage.TotalTokens, &usage.LatencyMS, &usage.Status, &usage.UsageSource, &errorCode, &createdAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelUsageRecord{}, false, nil
		}
		return controlplane.ModelUsageRecord{}, false, postgresOperationError(ctx, fmt.Errorf("load duplicate product normalized model usage: %w", err))
	}
	var err error
	usage.Product, err = normalizedStoredProduct(storedProduct)
	if err != nil {
		return controlplane.ModelUsageRecord{}, false, err
	}
	usage.LeaseID = storedLeaseID.String
	usage.ErrorCode = errorCode.String
	usage.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return usage, true, nil
}

func modelUsageMatchesInput(existing controlplane.ModelUsageRecord, record ModelUsageWriteRecord) bool {
	return existing.LeaseID == record.Input.LeaseID && existing.ClientCallID == record.Input.ClientCallID &&
		existing.Provider == record.Input.Provider && existing.Model == record.Input.Model &&
		existing.InputTokens == record.Input.InputTokens && existing.OutputTokens == record.Input.OutputTokens &&
		existing.TotalTokens == record.Input.TotalTokens && existing.LatencyMS == record.Input.LatencyMS &&
		existing.Status == record.Input.Status && existing.UsageSource == record.Input.UsageSource && existing.ErrorCode == record.Input.ErrorCode
}

func (s *PostgresRepository) loadUserDailyTokenLimit(ctx context.Context, tx *sql.Tx, userID string, products ...controlplane.ProductCode) (int, bool, error) {
	var limit int
	query := `
		SELECT daily_token_limit
		FROM user_authorization_policies
		WHERE user_id = $1
		FOR UPDATE
	`
	args := []any{userID}
	if len(products) > 0 && products[0] != "" {
		query = strings.Replace(query, "WHERE user_id = $1", "WHERE user_id = $1 AND product = $2", 1)
		args = append(args, products[0])
	}
	err := tx.QueryRowContext(ctx, query, args...).Scan(&limit)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, false, nil
	}
	if err != nil {
		return 0, false, postgresOperationError(ctx, fmt.Errorf("lock normalized user usage policy: %w", err))
	}
	return limit, true, nil
}

func (s *PostgresRepository) dailyModelUsageTokens(ctx context.Context, tx *sql.Tx, column, value string, now time.Time, products ...controlplane.ProductCode) (int, error) {
	if column != "user_id" && column != "account_id" {
		return 0, controlplane.ErrInvalidRequest
	}
	dayStart := time.Date(now.UTC().Year(), now.UTC().Month(), now.UTC().Day(), 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	query := fmt.Sprintf(`
		SELECT COALESCE(SUM(total_tokens), 0)
		FROM model_usage_records
		WHERE %s = $1 AND created_at >= $2 AND created_at < $3
	`, column)
	args := []any{value, dayStart, dayEnd}
	if len(products) > 0 && products[0] != "" {
		query = strings.Replace(query, " AND created_at >= $2", " AND product = $4 AND created_at >= $2", 1)
		args = append(args, products[0])
	}
	var used int
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&used); err != nil {
		return 0, postgresOperationError(ctx, fmt.Errorf("sum normalized daily model usage: %w", err))
	}
	return used, nil
}
