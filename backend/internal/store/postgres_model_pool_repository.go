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

var _ ModelPoolRepository = (*PostgresRepository)(nil)
var _ ModelPoolAccountCreator = (*PostgresRepository)(nil)

type normalizedModelPoolAccount struct {
	id               string
	product          controlplane.ProductCode
	provider         string
	model            string
	baseURL          string
	secretRef        string
	status           string
	priority         int
	concurrencyLimit int
	dailyLimit       int
	cooldownUntil    string
}

func (s *PostgresRepository) CreateModelPoolAccount(ctx context.Context, record ModelPoolAccountCreateRecord) (controlplane.ModelPoolAccountSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelPoolAccountSummary{}, errors.New("normalized model account creation requires normalized read source")
	}
	if err := validateModelPoolAccountCreateRecord(ctx, record); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.Provider = strings.TrimSpace(record.Provider)
	record.Model = strings.TrimSpace(record.Model)
	record.BaseURL = strings.TrimRight(strings.TrimSpace(record.BaseURL), "/")
	record.APIKey = strings.TrimSpace(record.APIKey)
	record.Status = strings.TrimSpace(record.Status)
	if record.Product.Valid() {
		audit, err := normalizeOptionalAuditInputForProduct(record.Audit, record.Product)
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		record.Audit = audit
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	accountID, err := newRepositoryID("mpa")
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("generate normalized model account id: %w", err))
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if record.Product != "" {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, accountID, s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, accountID, s.Now())
	}
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
		}
		var account normalizedModelPoolAccount
		if record.Product != "" {
			account, err = s.loadNormalizedModelPoolAccountWithProduct(operationCtx, tx, storedResourceID, record.Product)
		} else {
			account, err = s.loadNormalizedModelPoolAccount(operationCtx, tx, storedResourceID)
		}
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			audit := record.Audit
			if strings.TrimSpace(audit.TargetID) == "" {
				audit.TargetID = account.id
			}
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue idempotent model account audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit idempotent normalized model account creation", err)
		}
		return summary, nil
	}
	secretWriter, ok := s.secretStore.(TransactionalSecretWriter)
	if !ok {
		return controlplane.ModelPoolAccountSummary{}, ErrTransactionalSecretStoreRequired
	}
	secretRef := "model-account/" + accountID
	now := s.Now()
	var insertErr error
	if record.Product != "" {
		if !record.Product.Valid() {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
		}
		_, insertErr = tx.ExecContext(operationCtx, `
			INSERT INTO model_accounts (id, product, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, active_requests, daily_reserved_tokens, created_at, updated_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, 0, $11, $11)
		`, accountID, record.Product, record.Provider, record.Model, record.BaseURL, secretRef, record.Status, record.Priority, record.ConcurrencyLimit, record.DailyLimit, now)
	} else {
		_, insertErr = tx.ExecContext(operationCtx, `
			INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, active_requests, daily_reserved_tokens, created_at, updated_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
		`, accountID, record.Provider, record.Model, record.BaseURL, secretRef, record.Status, record.Priority, record.ConcurrencyLimit, record.DailyLimit, now)
	}
	if insertErr != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("create normalized model account: %w", insertErr))
	}
	if err := secretWriter.PutTx(operationCtx, tx, secretRef, record.APIKey); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("store normalized model account secret: %w", err))
	}
	account := normalizedModelPoolAccount{
		id: accountID, product: record.Product, provider: record.Provider, model: record.Model, baseURL: record.BaseURL,
		secretRef: secretRef, status: record.Status, priority: record.Priority,
		concurrencyLimit: record.ConcurrencyLimit, dailyLimit: record.DailyLimit,
	}
	summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, now)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		audit := record.Audit
		if strings.TrimSpace(audit.TargetID) == "" {
			audit.TargetID = accountID
		}
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, now); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model account creation audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit normalized model account creation", err)
	}
	return summary, nil
}

func validateModelPoolAccountCreateRecord(ctx context.Context, record ModelPoolAccountCreateRecord) error {
	if ctx == nil || strings.TrimSpace(record.Scope) == "" || strings.TrimSpace(record.IdempotencyKey) == "" || strings.TrimSpace(record.Fingerprint) == "" {
		return controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if record.Product != "" && !record.Product.Valid() {
		return controlplane.ErrInvalidRequest
	}
	if len(strings.TrimSpace(record.Provider)) == 0 || len(strings.TrimSpace(record.Provider)) > 64 || len(strings.TrimSpace(record.Model)) == 0 || len(strings.TrimSpace(record.Model)) > 128 {
		return controlplane.ErrInvalidRequest
	}
	if len(strings.TrimSpace(record.APIKey)) < 8 || len(strings.TrimSpace(record.APIKey)) > 4096 || len(strings.TrimSpace(record.BaseURL)) > 2048 {
		return controlplane.ErrInvalidRequest
	}
	if record.Priority < 0 || record.DailyLimit < 0 || record.ConcurrencyLimit < 1 || !validNormalizedModelAccountStatus(record.Status) {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func (s *PostgresRepository) DisableModelPoolAccount(ctx context.Context, record ModelPoolAccountMutationRecord) (controlplane.ModelPoolAccountSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelPoolAccountSummary{}, errors.New("normalized model account disable requires normalized read source")
	}
	if err := validateModelPoolMutationRecord(ctx, record); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	return s.mutateNormalizedModelPoolAccount(ctx, record, nil)
}

func (s *PostgresRepository) UpdateModelPoolAccount(ctx context.Context, record ModelPoolAccountUpdateRecord) (controlplane.ModelPoolAccountSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelPoolAccountSummary{}, errors.New("normalized model account update requires normalized read source")
	}
	if err := validateModelPoolMutationRecord(ctx, record.ModelPoolAccountMutationRecord); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if record.Input.BaseURL == nil && record.Input.Priority == nil && record.Input.DailyLimit == nil && record.Input.ConcurrencyLimit == nil && record.Input.Status == nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if record.Input.Priority != nil && *record.Input.Priority < 0 || record.Input.DailyLimit != nil && *record.Input.DailyLimit < 0 || record.Input.ConcurrencyLimit != nil && *record.Input.ConcurrencyLimit < 1 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if record.Input.Status != nil && !validNormalizedModelAccountStatus(*record.Input.Status) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if record.Input.BaseURL != nil && len(*record.Input.BaseURL) > 2048 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	return s.mutateNormalizedModelPoolAccount(ctx, record.ModelPoolAccountMutationRecord, &record.Input)
}

func validateModelPoolMutationRecord(ctx context.Context, record ModelPoolAccountMutationRecord) error {
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	if strings.TrimSpace(record.Scope) == "" || strings.TrimSpace(record.IdempotencyKey) == "" || strings.TrimSpace(record.Fingerprint) == "" || strings.TrimSpace(record.AccountID) == "" {
		return controlplane.ErrInvalidRequest
	}
	if record.Product != "" && !record.Product.Valid() {
		return controlplane.ErrInvalidRequest
	}
	return ctx.Err()
}

func validNormalizedModelAccountStatus(status string) bool {
	switch status {
	case controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown, controlplane.ModelAccountStatusExhausted, controlplane.ModelAccountStatusDisabled:
		return true
	default:
		return false
	}
}

func (s *PostgresRepository) mutateNormalizedModelPoolAccount(ctx context.Context, record ModelPoolAccountMutationRecord, input *controlplane.UpdateModelPoolAccountInput) (controlplane.ModelPoolAccountSummary, error) {
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.AccountID = strings.TrimSpace(record.AccountID)
	if record.Product.Valid() {
		audit, err := normalizeOptionalAuditInputForProduct(record.Audit, record.Product)
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		record.Audit = audit
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	var account normalizedModelPoolAccount
	if record.Product != "" {
		account, err = s.loadNormalizedModelPoolAccountWithProduct(operationCtx, tx, record.AccountID, record.Product)
	} else {
		account, err = s.loadNormalizedModelPoolAccount(operationCtx, tx, record.AccountID)
	}
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	var storedFingerprint, storedResourceID string
	var inserted bool
	if record.Product != "" {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotencyForProduct(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.AccountID, s.Now(), record.Product)
	} else {
		storedFingerprint, storedResourceID, inserted, err = s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.AccountID, s.Now())
	}
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != record.AccountID {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
		}
		summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		if strings.TrimSpace(record.Audit.Action) != "" {
			audit := record.Audit
			if strings.TrimSpace(audit.TargetID) == "" {
				audit.TargetID = record.AccountID
			}
			if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
				return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue idempotent model account mutation audit: %w", err))
			}
		}
		if err := tx.Commit(); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit idempotent normalized model account mutation", err)
		}
		return summary, nil
	}
	var expireErr error
	if record.Product != "" {
		expireErr = expireNormalizedModelPoolLeasesForProduct(operationCtx, tx, record.AccountID, s.Now(), record.Product)
	} else {
		expireErr = expireNormalizedModelPoolLeases(operationCtx, tx, record.AccountID, s.Now())
	}
	if expireErr != nil {
		return controlplane.ModelPoolAccountSummary{}, expireErr
	}
	var activeLeases int
	if record.Product != "" {
		activeLeases, err = countNormalizedActiveModelPoolLeasesForProduct(operationCtx, tx, record.AccountID, s.Now(), record.Product)
	} else {
		activeLeases, err = countNormalizedActiveModelPoolLeases(operationCtx, tx, record.AccountID, s.Now())
	}
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if input == nil {
		if activeLeases > 0 {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountInUse
		}
		query := `UPDATE model_accounts SET status = $2, updated_at = $3 WHERE id = $1`
		args := []any{record.AccountID, controlplane.ModelAccountStatusDisabled, s.Now()}
		if record.Product != "" {
			filter, filterArgs := normalizedProductFilter("product", record.Product, len(args)+1)
			query += " AND " + filter
			args = append(args, filterArgs...)
		}
		if _, err := tx.ExecContext(operationCtx, query, args...); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("disable normalized model account: %w", err))
		}
		account.status = controlplane.ModelAccountStatusDisabled
	} else {
		if input.Status != nil && *input.Status == controlplane.ModelAccountStatusDisabled && activeLeases > 0 {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountInUse
		}
		if input.ConcurrencyLimit != nil && activeLeases > *input.ConcurrencyLimit {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolConcurrencyConflict
		}
		assignments := make([]string, 0, 6)
		args := []any{record.AccountID}
		add := func(column string, value any) {
			args = append(args, value)
			assignments = append(assignments, fmt.Sprintf("%s = $%d", column, len(args)))
		}
		if input.BaseURL != nil {
			add("base_url", *input.BaseURL)
		}
		if input.Priority != nil {
			add("priority", *input.Priority)
		}
		if input.DailyLimit != nil {
			add("daily_token_limit", *input.DailyLimit)
		}
		if input.ConcurrencyLimit != nil {
			add("concurrency_limit", *input.ConcurrencyLimit)
		}
		if input.Status != nil {
			add("status", *input.Status)
			if *input.Status != controlplane.ModelAccountStatusCooldown {
				assignments = append(assignments, "cooldown_until = NULL")
			}
		}
		assignments = append(assignments, fmt.Sprintf("updated_at = $%d", len(args)+1))
		args = append(args, s.Now())
		query := fmt.Sprintf("UPDATE model_accounts SET %s WHERE id = $1", strings.Join(assignments, ", "))
		if record.Product != "" {
			filter, filterArgs := normalizedProductFilter("product", record.Product, len(args)+1)
			query += " AND " + filter
			args = append(args, filterArgs...)
		}
		if _, err := tx.ExecContext(operationCtx, query, args...); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized model account: %w", err))
		}
		if input.BaseURL != nil {
			account.baseURL = *input.BaseURL
		}
		if input.Priority != nil {
			account.priority = *input.Priority
		}
		if input.DailyLimit != nil {
			account.dailyLimit = *input.DailyLimit
		}
		if input.ConcurrencyLimit != nil {
			account.concurrencyLimit = *input.ConcurrencyLimit
		}
		if input.Status != nil {
			account.status = *input.Status
			if *input.Status != controlplane.ModelAccountStatusCooldown {
				account.cooldownUntil = ""
			}
		}
	}
	summary, err := s.loadNormalizedModelPoolAccountSummary(operationCtx, tx, account, s.Now())
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		audit := record.Audit
		if strings.TrimSpace(audit.TargetID) == "" {
			audit.TargetID = record.AccountID
		}
		if err := s.enqueueAuditOutboxTx(operationCtx, tx, audit, s.Now()); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(operationCtx, fmt.Errorf("enqueue model account mutation audit: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresCommitError(operationCtx, "commit normalized model account mutation", err)
	}
	return summary, nil
}

func (s *PostgresRepository) loadNormalizedModelPoolAccount(ctx context.Context, tx *sql.Tx, accountID string) (normalizedModelPoolAccount, error) {
	var account normalizedModelPoolAccount
	var cooldownUntil sql.NullTime
	if err := tx.QueryRowContext(ctx, `
		SELECT id, provider, model, base_url, secret_ref, status, priority,
		       concurrency_limit, daily_token_limit, cooldown_until
		FROM model_accounts
		WHERE id = $1
		FOR UPDATE
	`, accountID).Scan(&account.id, &account.provider, &account.model, &account.baseURL, &account.secretRef, &account.status, &account.priority, &account.concurrencyLimit, &account.dailyLimit, &cooldownUntil); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return normalizedModelPoolAccount{}, controlplane.ErrModelPoolAccountNotFound
		}
		return normalizedModelPoolAccount{}, postgresOperationError(ctx, fmt.Errorf("lock normalized model account: %w", err))
	}
	if cooldownUntil.Valid {
		account.cooldownUntil = cooldownUntil.Time.UTC().Format(time.RFC3339)
	}
	return account, nil
}

func (s *PostgresRepository) loadNormalizedModelPoolAccountWithProduct(ctx context.Context, tx *sql.Tx, accountID string, product controlplane.ProductCode) (normalizedModelPoolAccount, error) {
	var account normalizedModelPoolAccount
	var cooldownUntil sql.NullTime
	productFilter, productArgs := normalizedProductFilter("product", product, 2)
	args := []any{accountID}
	args = append(args, productArgs...)
	query := fmt.Sprintf(`
		SELECT id, product, provider, model, base_url, secret_ref, status, priority,
		       concurrency_limit, daily_token_limit, cooldown_until
		FROM model_accounts
		WHERE id = $1 AND %s
		FOR UPDATE
	`, productFilter)
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&account.id, &account.product, &account.provider, &account.model, &account.baseURL, &account.secretRef, &account.status, &account.priority, &account.concurrencyLimit, &account.dailyLimit, &cooldownUntil); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return normalizedModelPoolAccount{}, controlplane.ErrModelPoolAccountNotFound
		}
		return normalizedModelPoolAccount{}, postgresOperationError(ctx, fmt.Errorf("lock normalized model account for product: %w", err))
	}
	if cooldownUntil.Valid {
		account.cooldownUntil = cooldownUntil.Time.UTC().Format(time.RFC3339)
	}
	return account, nil
}

func expireNormalizedModelPoolLeases(ctx context.Context, tx *sql.Tx, accountID string, now time.Time) error {
	if _, err := tx.ExecContext(ctx, `
		UPDATE model_leases
		SET status = $2, released_at = COALESCE(released_at, $3)
		WHERE account_id = $1 AND status = $4 AND expires_at <= $3
	`, accountID, controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("expire normalized model account leases: %w", err))
	}
	return nil
}

func expireNormalizedModelPoolLeasesForProduct(ctx context.Context, tx *sql.Tx, accountID string, now time.Time, product controlplane.ProductCode) error {
	productFilter, productArgs := normalizedProductFilter("product", product, 5)
	args := []any{accountID, controlplane.ModelLeaseStatusExpired, now, controlplane.ModelLeaseStatusActive}
	args = append(args, productArgs...)
	query := fmt.Sprintf(`
		UPDATE model_leases
		SET status = $2, released_at = COALESCE(released_at, $3)
		WHERE account_id = $1 AND status = $4 AND expires_at <= $3 AND %s
	`, productFilter)
	if _, err := tx.ExecContext(ctx, query, args...); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("expire normalized model account leases for product: %w", err))
	}
	return nil
}

func countNormalizedActiveModelPoolLeases(ctx context.Context, tx *sql.Tx, accountID string, now time.Time) (int, error) {
	var count int
	if err := tx.QueryRowContext(ctx, `
		SELECT COUNT(*) FROM model_leases
		WHERE account_id = $1 AND status = $2 AND expires_at > $3
	`, accountID, controlplane.ModelLeaseStatusActive, now).Scan(&count); err != nil {
		return 0, postgresOperationError(ctx, fmt.Errorf("count normalized model account leases: %w", err))
	}
	return count, nil
}

func countNormalizedActiveModelPoolLeasesForProduct(ctx context.Context, tx *sql.Tx, accountID string, now time.Time, product controlplane.ProductCode) (int, error) {
	productFilter, productArgs := normalizedProductFilter("product", product, 4)
	args := []any{accountID, controlplane.ModelLeaseStatusActive, now}
	args = append(args, productArgs...)
	query := fmt.Sprintf(`
		SELECT COUNT(*) FROM model_leases
		WHERE account_id = $1 AND status = $2 AND expires_at > $3 AND %s
	`, productFilter)
	var count int
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&count); err != nil {
		return 0, postgresOperationError(ctx, fmt.Errorf("count normalized model account leases for product: %w", err))
	}
	return count, nil
}

func (s *PostgresRepository) loadNormalizedModelPoolAccountSummary(ctx context.Context, tx *sql.Tx, account normalizedModelPoolAccount, now time.Time) (controlplane.ModelPoolAccountSummary, error) {
	dayStart := time.Date(now.UTC().Year(), now.UTC().Month(), now.UTC().Day(), 0, 0, 0, 0, time.UTC)
	dayEnd := dayStart.Add(24 * time.Hour)
	var activeLeases, dailyUsedTokens int
	activeLeases, err := s.countNormalizedActiveModelPoolLeases(ctx, tx, account, now)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(ctx, fmt.Errorf("load normalized account active leases: %w", err))
	}
	dailyUsedTokens, err = s.sumNormalizedDailyModelUsage(ctx, tx, account, dayStart, dayEnd)
	if err != nil {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(ctx, fmt.Errorf("load normalized account daily usage: %w", err))
	}
	var payload []byte
	var testedAt sql.NullTime
	testQuery := `
		SELECT payload, created_at
		FROM model_pool_test_results
		WHERE account_id = $1
		ORDER BY created_at DESC, id DESC
		LIMIT 1
	`
	testArgs := []any{account.id}
	if account.product != "" {
		filter, filterArgs := normalizedProductFilter("product", account.product, 2)
		testQuery = fmt.Sprintf(`
			SELECT payload, created_at
			FROM model_pool_test_results
			WHERE account_id = $1 AND %s
			ORDER BY created_at DESC, id DESC
			LIMIT 1
		`, filter)
		testArgs = append(testArgs, filterArgs...)
	}
	if err := tx.QueryRowContext(ctx, testQuery, testArgs...).Scan(&payload, &testedAt); err != nil && !errors.Is(err, sql.ErrNoRows) {
		return controlplane.ModelPoolAccountSummary{}, postgresOperationError(ctx, fmt.Errorf("load normalized account test result: %w", err))
	}
	summary := controlplane.ModelPoolAccountSummary{
		ID: account.id, Product: effectiveModelAccountProduct(account.product), Provider: account.provider, Model: account.model, BaseURL: account.baseURL, Status: account.status,
		Priority: account.priority, DailyLimit: account.dailyLimit, ConcurrencyLimit: account.concurrencyLimit,
		SecretConfigured: account.secretRef != "", ActiveLeases: activeLeases, DailyUsedTokens: dailyUsedTokens, CooldownUntil: account.cooldownUntil,
	}
	if testedAt.Valid {
		summary.LastTestedAt = testedAt.Time.UTC().Format(time.RFC3339)
	}
	if len(payload) > 0 {
		var result controlplane.ModelPoolConnectivityTestResult
		if err := json.Unmarshal(payload, &result); err != nil {
			return controlplane.ModelPoolAccountSummary{}, postgresOperationError(ctx, fmt.Errorf("decode normalized account test result: %w", err))
		}
		summary.LastTestStatus = result.Status
		if result.TestedAt != "" {
			summary.LastTestedAt = result.TestedAt
		}
	}
	return summary, nil
}

func effectiveModelAccountProduct(product controlplane.ProductCode) controlplane.ProductCode {
	if product == "" {
		return controlplane.ProductAutoLive
	}
	return product
}

func (s *PostgresRepository) countNormalizedActiveModelPoolLeases(ctx context.Context, tx *sql.Tx, account normalizedModelPoolAccount, now time.Time) (int, error) {
	if account.product == "" {
		return countNormalizedActiveModelPoolLeases(ctx, tx, account.id, now)
	}
	return countNormalizedActiveModelPoolLeasesForProduct(ctx, tx, account.id, now, account.product)
}

func (s *PostgresRepository) sumNormalizedDailyModelUsage(ctx context.Context, tx *sql.Tx, account normalizedModelPoolAccount, dayStart, dayEnd time.Time) (int, error) {
	var total int
	if account.product == "" {
		if err := tx.QueryRowContext(ctx, `
			SELECT COALESCE(SUM(total_tokens), 0)
			FROM model_usage_records
			WHERE account_id = $1 AND created_at >= $2 AND created_at < $3
		`, account.id, dayStart, dayEnd).Scan(&total); err != nil {
			return 0, err
		}
		return total, nil
	}
	filter, filterArgs := normalizedProductFilter("product", account.product, 4)
	args := []any{account.id, dayStart, dayEnd}
	args = append(args, filterArgs...)
	query := fmt.Sprintf(`
		SELECT COALESCE(SUM(total_tokens), 0)
		FROM model_usage_records
		WHERE account_id = $1 AND created_at >= $2 AND created_at < $3 AND %s
	`, filter)
	if err := tx.QueryRowContext(ctx, query, args...).Scan(&total); err != nil {
		return 0, err
	}
	return total, nil
}
