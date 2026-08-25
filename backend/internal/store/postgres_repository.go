package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"autoLive/backend/internal/controlplane"
)

// PostgresRepository 是控制面当前版本的事务适配器。
// 旧任务/阶段/模型代理表属于历史迁移兼容边界，新代码不再读写这些表。
type PostgresRepository struct {
	db               *sql.DB
	now              func() time.Time
	secretStore      SecretStore
	modelReadSource  string
	operationTimeout time.Duration
}

var _ TransactionalSessionBinder = (*PostgresRepository)(nil)

const (
	ModelReadSourceSnapshot         = "snapshot"
	ModelReadSourceNormalized       = "normalized"
	defaultPostgresOperationTimeout = 30 * time.Second
)

func NewPostgresRepository(db *sql.DB, now func() time.Time) (*PostgresRepository, error) {
	return NewPostgresRepositoryWithSecretStore(db, now, nil)
}

func NewPostgresRepositoryWithSecretStore(db *sql.DB, now func() time.Time, secretStore SecretStore) (*PostgresRepository, error) {
	return newPostgresRepository(db, now, secretStore, ModelReadSourceSnapshot, defaultPostgresOperationTimeout)
}

func newPostgresRepository(db *sql.DB, now func() time.Time, secretStore SecretStore, modelReadSource string, operationTimeout time.Duration) (*PostgresRepository, error) {
	if db == nil {
		return nil, errors.New("postgres repository database must not be nil")
	}
	if now == nil {
		now = time.Now
	}
	if modelReadSource != ModelReadSourceSnapshot && modelReadSource != ModelReadSourceNormalized {
		return nil, fmt.Errorf("model read source %q is not supported; use %q or %q", modelReadSource, ModelReadSourceSnapshot, ModelReadSourceNormalized)
	}
	if operationTimeout <= 0 {
		return nil, errors.New("postgres repository operation timeout must be greater than zero")
	}
	return &PostgresRepository{db: db, now: now, secretStore: secretStore, modelReadSource: modelReadSource, operationTimeout: operationTimeout}, nil
}

// 启动配置可选择兼容快照读源或规范化领域表读源。规范化模式使用领域表作为唯一运行时事实源，不更新历史快照。
func NewPostgresRepositoryWithSecretStoreAndModelReadSource(db *sql.DB, now func() time.Time, secretStore SecretStore, modelReadSource string) (*PostgresRepository, error) {
	return newPostgresRepository(db, now, secretStore, modelReadSource, defaultPostgresOperationTimeout)
}

// NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout binds the
// database transaction/statement budget to the application request budget.
// The timeout is an upper bound; a shorter caller Context still wins.
func NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(db *sql.DB, now func() time.Time, secretStore SecretStore, modelReadSource string, operationTimeout time.Duration) (*PostgresRepository, error) {
	return newPostgresRepository(db, now, secretStore, modelReadSource, operationTimeout)
}

func (s *PostgresRepository) Now() time.Time { return s.now().UTC() }

func (s *PostgresRepository) UsesNormalizedReadSource() bool {
	return s.modelReadSource == ModelReadSourceNormalized
}

// DBStats exposes only aggregate connection-pool counters for operational
// metrics; it never exposes connection strings or query arguments.
func (s *PostgresRepository) DBStats() sql.DBStats { return s.db.Stats() }

// TryAdvisoryLock uses a dedicated database connection because PostgreSQL
// advisory locks are session-scoped. The connection remains checked out until
// the returned release function is called, so callers must always defer it.
func (s *PostgresRepository) TryAdvisoryLock(ctx context.Context, key int64) (AdvisoryLockRelease, bool, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	if err := ctx.Err(); err != nil {
		return nil, false, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	conn, err := s.db.Conn(operationCtx)
	if err != nil {
		return nil, false, postgresOperationError(operationCtx, err)
	}
	var acquired bool
	if err := conn.QueryRowContext(operationCtx, `SELECT pg_try_advisory_lock($1)`, key).Scan(&acquired); err != nil {
		_ = conn.Close()
		return nil, false, postgresOperationError(operationCtx, err)
	}
	if !acquired {
		_ = conn.Close()
		return nil, false, nil
	}

	var once sync.Once
	var releaseErr error
	release := func(releaseCtx context.Context) error {
		once.Do(func() {
			if releaseCtx == nil {
				releaseCtx = context.Background()
			}
			unlockCtx, unlockCancel := context.WithTimeout(context.WithoutCancel(releaseCtx), s.operationTimeout)
			defer unlockCancel()
			var unlocked bool
			if err := conn.QueryRowContext(unlockCtx, `SELECT pg_advisory_unlock($1)`, key).Scan(&unlocked); err != nil {
				releaseErr = postgresOperationError(unlockCtx, err)
			} else if !unlocked {
				releaseErr = errors.New("postgres advisory lock was not held during release")
			}
			if closeErr := conn.Close(); releaseErr == nil && closeErr != nil {
				releaseErr = closeErr
			}
		})
		return releaseErr
	}
	return release, true, nil
}

func runPostgresReadPage[T any](s *PostgresRepository, ctx context.Context, fn func(context.Context, *sql.Tx) (T, error)) (T, error) {
	var zero T
	if err := ctx.Err(); err != nil {
		return zero, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return zero, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := s.ensureNormalizedCoverageCounts(operationCtx, tx); err != nil {
		return zero, postgresOperationError(operationCtx, err)
	}
	result, err := fn(operationCtx, tx)
	if err != nil {
		return zero, postgresOperationError(operationCtx, err)
	}
	if err := tx.Commit(); err != nil {
		return zero, postgresCommitError(operationCtx, "commit postgres read page", err)
	}
	return result, nil
}

func (s *PostgresRepository) ListUsersPage(ctx context.Context, offset, limit int) (UserPage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return UserPage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page UserPage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.UserSummary, 0, len(state.Users))
			for _, item := range state.Users {
				items = append(items, item)
			}
			slices.SortFunc(items, func(a, b controlplane.UserSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.UserSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (UserPage, error) {
		var page UserPage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM users`).Scan(&page.Total); err != nil {
			return UserPage{}, err
		}
		rows, err := tx.QueryContext(ctx, `
			SELECT id, username, role, status, created_at
			FROM users
			ORDER BY id
			LIMIT $1 OFFSET $2
		`, limit, offset)
		if err != nil {
			return UserPage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			var item controlplane.UserSummary
			var createdAt time.Time
			if err := rows.Scan(&item.ID, &item.Username, &item.Role, &item.Status, &createdAt); err != nil {
				return UserPage{}, err
			}
			item.CreatedAt = createdAt.UTC().Format(time.RFC3339)
			page.Items = append(page.Items, item)
		}
		if err := rows.Err(); err != nil {
			return UserPage{}, err
		}
		return page, nil
	})
}

func (s *PostgresRepository) ListUsersPageForProduct(ctx context.Context, offset, limit int, product controlplane.ProductCode) (UserPage, error) {
	if !product.Valid() {
		return UserPage{}, controlplane.ErrInvalidRequest
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return UserPage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page UserPage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.UserSummary, 0, len(state.Users))
			for _, item := range state.Users {
				if memoryUserHasProduct(state, item.ID, product) {
					items = append(items, item)
				}
			}
			slices.SortFunc(items, func(a, b controlplane.UserSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.UserSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (UserPage, error) {
		var page UserPage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM users u JOIN user_products up ON up.user_id = u.id WHERE up.product = $1 AND up.status = 'active'`, product).Scan(&page.Total); err != nil {
			return UserPage{}, err
		}
		rows, err := tx.QueryContext(ctx, `SELECT u.id, u.username, u.role, u.status, u.created_at FROM users u JOIN user_products up ON up.user_id = u.id WHERE up.product = $1 AND up.status = 'active' ORDER BY u.id LIMIT $2 OFFSET $3`, product, limit, offset)
		if err != nil {
			return UserPage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			var item controlplane.UserSummary
			var createdAt time.Time
			if err := rows.Scan(&item.ID, &item.Username, &item.Role, &item.Status, &createdAt); err != nil {
				return UserPage{}, err
			}
			item.CreatedAt = createdAt.UTC().Format(time.RFC3339)
			page.Items = append(page.Items, item)
		}
		return page, rows.Err()
	})
}

func (s *PostgresRepository) ListDevicesPage(ctx context.Context, offset, limit int) (DevicePage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page DevicePage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.DeviceSummary, 0, len(state.Devices))
			for _, item := range state.Devices {
				items = append(items, item)
			}
			slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (DevicePage, error) {
		var page DevicePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices`).Scan(&page.Total); err != nil {
			return DevicePage{}, err
		}
		rows, err := tx.QueryContext(ctx, devicePageQuery+` ORDER BY id LIMIT $1 OFFSET $2`, limit, offset)
		if err != nil {
			return DevicePage{}, err
		}
		defer rows.Close()
		items, err := scanDeviceRows(rows)
		if err != nil {
			return DevicePage{}, err
		}
		page.Items = items
		return page, nil
	})
}

func (s *PostgresRepository) ListDevicesPageForProduct(ctx context.Context, offset, limit int, product controlplane.ProductCode) (DevicePage, error) {
	if !product.Valid() {
		return DevicePage{}, controlplane.ErrInvalidRequest
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page DevicePage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.DeviceSummary, 0, len(state.Devices))
			for _, item := range state.Devices {
				if memoryResourceProduct(item.Product) == product {
					items = append(items, item)
				}
			}
			slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (DevicePage, error) {
		var page DevicePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE product = $1`, product).Scan(&page.Total); err != nil {
			return DevicePage{}, err
		}
		rows, err := tx.QueryContext(ctx, devicePageQuery+` WHERE product = $1 ORDER BY id LIMIT $2 OFFSET $3`, product, limit, offset)
		if err != nil {
			return DevicePage{}, err
		}
		defer rows.Close()
		items, err := scanDeviceRows(rows)
		if err != nil {
			return DevicePage{}, err
		}
		page.Items = items
		return page, nil
	})
}

func (s *PostgresRepository) ListDevicesForUserPage(ctx context.Context, userID string, offset, limit int) (DevicePage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return DevicePage{}, controlplane.ErrUserNotFound
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page DevicePage
		err := s.Run(ctx, func(state *State) error {
			if _, ok := state.Users[userID]; !ok {
				return controlplane.ErrUserNotFound
			}
			items := make([]controlplane.DeviceSummary, 0)
			for _, item := range state.Devices {
				if item.UserID == userID {
					items = append(items, item)
				}
			}
			slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (DevicePage, error) {
		var exists bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)`, userID).Scan(&exists); err != nil {
			return DevicePage{}, err
		}
		if !exists {
			return DevicePage{}, controlplane.ErrUserNotFound
		}
		var page DevicePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE user_id = $1`, userID).Scan(&page.Total); err != nil {
			return DevicePage{}, err
		}
		rows, err := tx.QueryContext(ctx, devicePageQuery+` WHERE user_id = $1 ORDER BY id LIMIT $2 OFFSET $3`, userID, limit, offset)
		if err != nil {
			return DevicePage{}, err
		}
		defer rows.Close()
		items, err := scanDeviceRows(rows)
		if err != nil {
			return DevicePage{}, err
		}
		page.Items = items
		return page, nil
	})
}

func (s *PostgresRepository) ListDevicesForUserPageForProduct(ctx context.Context, userID string, offset, limit int, product controlplane.ProductCode) (DevicePage, error) {
	if !product.Valid() {
		return DevicePage{}, controlplane.ErrInvalidRequest
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return DevicePage{}, controlplane.ErrUserNotFound
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page DevicePage
		err := s.Run(ctx, func(state *State) error {
			if _, ok := state.Users[userID]; !ok {
				return controlplane.ErrUserNotFound
			}
			items := make([]controlplane.DeviceSummary, 0)
			for _, item := range state.Devices {
				if item.UserID == userID && memoryResourceProduct(item.Product) == product {
					items = append(items, item)
				}
			}
			slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (DevicePage, error) {
		var exists bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)`, userID).Scan(&exists); err != nil {
			return DevicePage{}, err
		}
		if !exists {
			return DevicePage{}, controlplane.ErrUserNotFound
		}
		var page DevicePage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE user_id = $1 AND product = $2`, userID, product).Scan(&page.Total); err != nil {
			return DevicePage{}, err
		}
		rows, err := tx.QueryContext(ctx, devicePageQuery+` WHERE user_id = $1 AND product = $2 ORDER BY id LIMIT $3 OFFSET $4`, userID, product, limit, offset)
		if err != nil {
			return DevicePage{}, err
		}
		defer rows.Close()
		items, err := scanDeviceRows(rows)
		if err != nil {
			return DevicePage{}, err
		}
		page.Items = items
		return page, nil
	})
}

func (s *PostgresRepository) ListModelUsagePage(ctx context.Context, offset, limit int) (ModelUsagePage, error) {
	return s.ListModelUsagePageWithOptions(ctx, ModelUsagePageOptions{Offset: offset, Limit: limit})
}

func (s *PostgresRepository) ListModelUsagePageWithOptions(ctx context.Context, options ModelUsagePageOptions) (ModelUsagePage, error) {
	options, err := NormalizeModelUsagePageOptions(options)
	if err != nil {
		return ModelUsagePage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page ModelUsagePage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.ModelUsageRecord, 0, len(state.ModelUsageRecords))
			for _, item := range state.ModelUsageRecords {
				lease := state.ModelLeases[item.LeaseID]
				if !modelUsageMatchesPageOptions(item, options, lease.UserID, lease.DeviceID) {
					continue
				}
				item.Product = compatibilityStoredProduct(item.Product)
				items = append(items, item)
			}
			sortModelUsageRecords(items, options.Sort)
			page.Total = len(items)
			start, end := pageWindow(page.Total, options.Offset, options.Limit)
			page.Items = append([]controlplane.ModelUsageRecord(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ModelUsagePage, error) {
		var page ModelUsagePage
		filterArgs := make([]any, 0, 8)
		where := make([]string, 0, 8)
		addArg := func(value any) string {
			filterArgs = append(filterArgs, value)
			return fmt.Sprintf("$%d", len(filterArgs))
		}
		for _, filter := range []struct {
			column string
			value  string
		}{
			{column: "product", value: string(options.Product)},
			{column: "provider", value: options.Provider},
			{column: "model", value: options.Model},
			{column: "user_id", value: options.UserID},
			{column: "device_id", value: options.DeviceID},
			{column: "request_id", value: options.RequestID},
		} {
			if filter.value != "" {
				where = append(where, fmt.Sprintf("%s = %s", filter.column, addArg(filter.value)))
			}
		}
		if options.CreatedAfter != nil {
			where = append(where, fmt.Sprintf("created_at >= %s", addArg(*options.CreatedAfter)))
		}
		if options.CreatedBefore != nil {
			where = append(where, fmt.Sprintf("created_at <= %s", addArg(*options.CreatedBefore)))
		}
		whereSQL := ""
		if len(where) > 0 {
			whereSQL = " WHERE " + strings.Join(where, " AND ")
		}
		if err := tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM model_usage_records"+whereSQL, filterArgs...).Scan(&page.Total); err != nil {
			return ModelUsagePage{}, err
		}
		orderBy := "created_at DESC, id DESC"
		if options.Sort == ModelUsageSortCreatedAsc {
			orderBy = "created_at ASC, id ASC"
		}
		limitPlaceholder := addArg(options.Limit)
		offsetPlaceholder := addArg(options.Offset)
		query := fmt.Sprintf(`
			SELECT id, product, lease_id, client_call_id, request_id, provider, model,
			       prompt_tokens, completion_tokens, total_tokens, latency_ms,
			       status, usage_source, error_code, created_at
			FROM model_usage_records%s
			ORDER BY %s
			LIMIT %s OFFSET %s
		`, whereSQL, orderBy, limitPlaceholder, offsetPlaceholder)
		rows, err := tx.QueryContext(ctx, query, filterArgs...)
		if err != nil {
			return ModelUsagePage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			var (
				item               controlplane.ModelUsageRecord
				leaseID, errorCode sql.NullString
				product            sql.NullString
				createdAt          time.Time
			)
			dest := []any{&item.ID, &product, &leaseID, &item.ClientCallID, &item.RequestID, &item.Provider, &item.Model,
				&item.InputTokens, &item.OutputTokens, &item.TotalTokens, &item.LatencyMS,
				&item.Status, &item.UsageSource, &errorCode, &createdAt}
			if err := rows.Scan(dest...); err != nil {
				return ModelUsagePage{}, err
			}
			item.LeaseID = leaseID.String
			item.ErrorCode = errorCode.String
			item.Product, err = normalizedStoredProduct(product)
			if err != nil {
				return ModelUsagePage{}, err
			}
			item.CreatedAt = createdAt.UTC().Format(time.RFC3339)
			page.Items = append(page.Items, item)
		}
		if err := rows.Err(); err != nil {
			return ModelUsagePage{}, err
		}
		return page, nil
	})
}

func (s *PostgresRepository) ListAuditLogsPage(ctx context.Context, offset, limit int) (AuditPage, error) {
	return s.ListAuditLogsPageWithOptions(ctx, AuditLogPageOptions{Offset: offset, Limit: limit})
}

func (s *PostgresRepository) ListAuditLogsPageWithOptions(ctx context.Context, options AuditLogPageOptions) (AuditPage, error) {
	options, err := NormalizeAuditLogPageOptions(options)
	if err != nil {
		return AuditPage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page AuditPage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.AuditLog, 0, len(state.AuditLogs))
			for _, item := range state.AuditLogs {
				if auditLogMatchesPageOptions(item, options) {
					item.Product = compatibilityStoredProduct(item.Product)
					items = append(items, item)
				}
			}
			sortAuditLogs(items, options.Sort)
			page.Total = len(items)
			start, end := pageWindow(page.Total, options.Offset, options.Limit)
			page.Items = append([]controlplane.AuditLog(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (AuditPage, error) {
		var page AuditPage
		filterArgs := make([]any, 0, 10)
		where := make([]string, 0, 10)
		addArg := func(value any) string {
			filterArgs = append(filterArgs, value)
			return fmt.Sprintf("$%d", len(filterArgs))
		}
		for _, filter := range []struct {
			column string
			value  string
		}{
			{column: "product", value: string(options.Product)},
			{column: "actor_user_id", value: options.ActorUserID},
			{column: "device_id", value: options.DeviceID},
			{column: "action", value: options.Action},
			{column: "resource_type", value: options.TargetType},
			{column: "outcome", value: options.Outcome},
			{column: "error_code", value: options.ErrorCode},
			{column: "request_id", value: options.RequestID},
		} {
			if filter.value != "" {
				where = append(where, fmt.Sprintf("%s = %s", filter.column, addArg(filter.value)))
			}
		}
		if options.CreatedAfter != nil {
			where = append(where, fmt.Sprintf("created_at >= %s", addArg(*options.CreatedAfter)))
		}
		if options.CreatedBefore != nil {
			where = append(where, fmt.Sprintf("created_at <= %s", addArg(*options.CreatedBefore)))
		}
		whereSQL := ""
		if len(where) > 0 {
			whereSQL = " WHERE " + strings.Join(where, " AND ")
		}
		if err := tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_logs"+whereSQL, filterArgs...).Scan(&page.Total); err != nil {
			return AuditPage{}, err
		}
		orderBy := "created_at DESC, id DESC"
		if options.Sort == AuditLogSortCreatedAsc {
			orderBy = "created_at ASC, id ASC"
		}
		limitPlaceholder := addArg(options.Limit)
		offsetPlaceholder := addArg(options.Offset)
		query := fmt.Sprintf(`
			SELECT id, product, actor_user_id, device_id, action, resource_type, resource_id,
			       request_id, outcome, status_code, error_code, created_at
			FROM audit_logs%s
			ORDER BY %s
			LIMIT %s OFFSET %s
		`, whereSQL, orderBy, limitPlaceholder, offsetPlaceholder)
		rows, err := tx.QueryContext(ctx, query, filterArgs...)
		if err != nil {
			return AuditPage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			var (
				item                                         controlplane.AuditLog
				actorUserID, deviceID, resourceID, requestID sql.NullString
				outcome, errorCode                           sql.NullString
				product                                      sql.NullString
				statusCode                                   sql.NullInt64
				createdAt                                    time.Time
			)
			if err := rows.Scan(&item.ID, &product, &actorUserID, &deviceID, &item.Action, &item.TargetType, &resourceID,
				&requestID, &outcome, &statusCode, &errorCode, &createdAt); err != nil {
				return AuditPage{}, err
			}
			item.ActorUserID = actorUserID.String
			item.Product, err = normalizedAuditProduct(product)
			if err != nil {
				return AuditPage{}, err
			}
			item.DeviceID = deviceID.String
			item.TargetID = resourceID.String
			item.RequestID = requestID.String
			item.Outcome = outcome.String
			item.StatusCode = int(statusCode.Int64)
			item.ErrorCode = errorCode.String
			item.CreatedAt = createdAt.UTC().Format(time.RFC3339)
			page.Items = append(page.Items, item)
		}
		if err := rows.Err(); err != nil {
			return AuditPage{}, err
		}
		return page, nil
	})
}

func (s *PostgresRepository) ListModelPoolAccountsPage(ctx context.Context, offset, limit int) (ModelPoolPage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return ModelPoolPage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page ModelPoolPage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
			for _, item := range state.ModelPoolAccounts {
				items = append(items, item)
			}
			slices.SortFunc(items, func(a, b controlplane.ModelPoolAccountSummary) int {
				return strings.Compare(a.ID, b.ID)
			})
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.ModelPoolAccountSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}

	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ModelPoolPage, error) {
		var page ModelPoolPage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_accounts`).Scan(&page.Total); err != nil {
			return ModelPoolPage{}, err
		}
		now := s.Now()
		dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
		dayEnd := dayStart.Add(24 * time.Hour)
		rows, err := tx.QueryContext(ctx, `
			SELECT a.id, a.product, a.provider, a.model, a.base_url, a.secret_ref, a.status,
			       a.priority, a.concurrency_limit, a.daily_token_limit, a.cooldown_until,
			       (SELECT COUNT(*)
			          FROM model_leases l
			         WHERE l.account_id = a.id
			           AND l.product = a.product
			           AND l.status = 'active'
			           AND l.expires_at > $1) AS active_leases,
			       COALESCE((SELECT SUM(u.total_tokens)
			                   FROM model_usage_records u
			                  WHERE u.account_id = a.id
			                    AND u.product = a.product
			                    AND u.created_at >= $2
			                    AND u.created_at < $3), 0) AS daily_used_tokens,
			       t.payload, t.created_at
			  FROM model_accounts a
			  LEFT JOIN LATERAL (
				SELECT payload, created_at
				  FROM model_pool_test_results
				 WHERE account_id = a.id
				   AND product = a.product
				 ORDER BY created_at DESC, id DESC
				 LIMIT 1
			  ) t ON TRUE
			 ORDER BY a.id
			 LIMIT $4 OFFSET $5
		`, now, dayStart, dayEnd, limit, offset)
		if err != nil {
			return ModelPoolPage{}, err
		}
		page.Items, err = scanModelPoolAccountRows(rows)
		if err != nil {
			return ModelPoolPage{}, err
		}
		return page, nil
	})
}

func (s *PostgresRepository) ListModelPoolAccountsPageForProduct(ctx context.Context, offset, limit int, product controlplane.ProductCode) (ModelPoolPage, error) {
	if !product.Valid() {
		return ModelPoolPage{}, controlplane.ErrInvalidRequest
	}
	if err := validatePageWindow(offset, limit); err != nil {
		return ModelPoolPage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page ModelPoolPage
		err := s.Run(ctx, func(state *State) error {
			items := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
			for _, item := range state.ModelPoolAccounts {
				if memoryResourceProduct(item.Product) == product {
					items = append(items, item)
				}
			}
			slices.SortFunc(items, func(a, b controlplane.ModelPoolAccountSummary) int { return strings.Compare(a.ID, b.ID) })
			page.Total = len(items)
			start, end := pageWindow(page.Total, offset, limit)
			page.Items = append([]controlplane.ModelPoolAccountSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ModelPoolPage, error) {
		var page ModelPoolPage
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_accounts WHERE product = $1`, product).Scan(&page.Total); err != nil {
			return ModelPoolPage{}, err
		}
		now := s.Now()
		dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
		dayEnd := dayStart.Add(24 * time.Hour)
		rows, err := tx.QueryContext(ctx, `
			SELECT a.id, a.product, a.provider, a.model, a.base_url, a.secret_ref, a.status,
			       a.priority, a.concurrency_limit, a.daily_token_limit, a.cooldown_until,
			       (SELECT COUNT(*) FROM model_leases l WHERE l.account_id = a.id AND l.product = a.product AND l.status = 'active' AND l.expires_at > $1),
			       COALESCE((SELECT SUM(u.total_tokens) FROM model_usage_records u WHERE u.account_id = a.id AND u.product = a.product AND u.created_at >= $2 AND u.created_at < $3), 0),
			       t.payload, t.created_at
			  FROM model_accounts a
			  LEFT JOIN LATERAL (SELECT payload, created_at FROM model_pool_test_results WHERE account_id = a.id AND product = a.product ORDER BY created_at DESC, id DESC LIMIT 1) t ON TRUE
			 WHERE a.product = $4
			 ORDER BY a.id LIMIT $5 OFFSET $6
		`, now, dayStart, dayEnd, product, limit, offset)
		if err != nil {
			return ModelPoolPage{}, err
		}
		items, err := scanModelPoolAccountRows(rows)
		if err != nil {
			return ModelPoolPage{}, err
		}
		page.Items = items
		return page, nil
	})
}

// ListModelPoolHealthAccounts applies eligibility predicates before LIMIT so
// disabled/cooldown/exhausted rows cannot starve later healthy accounts.
func (s *PostgresRepository) ListModelPoolHealthAccounts(ctx context.Context, limit int) ([]controlplane.ModelPoolAccountSummary, error) {
	if err := validatePageWindow(0, limit); err != nil {
		return nil, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		page, err := s.ListModelPoolAccountsPage(ctx, 0, limit)
		return page.Items, err
	}

	page, err := runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ModelPoolPage, error) {
		now := s.Now()
		dayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
		dayEnd := dayStart.Add(24 * time.Hour)
		rows, err := tx.QueryContext(ctx, `
			SELECT a.id, a.product, a.provider, a.model, a.base_url, a.secret_ref, a.status,
			       a.priority, a.concurrency_limit, a.daily_token_limit, a.cooldown_until,
			       (SELECT COUNT(*)
			          FROM model_leases l
			         WHERE l.account_id = a.id
			           AND l.product = a.product
			           AND l.status = 'active'
			           AND l.expires_at > $1) AS active_leases,
			       COALESCE((SELECT SUM(u.total_tokens)
			                   FROM model_usage_records u
			                  WHERE u.account_id = a.id
			                    AND u.product = a.product
			                    AND u.created_at >= $2
			                    AND u.created_at < $3), 0) AS daily_used_tokens,
			       t.payload, t.created_at
			  FROM model_accounts a
			  LEFT JOIN LATERAL (
				SELECT payload, created_at
				  FROM model_pool_test_results
				 WHERE account_id = a.id
				   AND product = a.product
				 ORDER BY created_at DESC, id DESC
				 LIMIT 1
			  ) t ON TRUE
			 WHERE a.status <> 'disabled'
			   AND a.secret_ref <> ''
			   AND (a.cooldown_until IS NULL OR a.cooldown_until <= $1)
			   AND (a.daily_token_limit <= 0 OR COALESCE((SELECT SUM(u.total_tokens)
				                                               FROM model_usage_records u
				                                              WHERE u.account_id = a.id
				                                                AND u.product = a.product
				                                                AND u.created_at >= $2
			                                                 AND u.created_at < $3), 0) < a.daily_token_limit)
			 ORDER BY a.id
			 LIMIT $4
		`, now, dayStart, dayEnd, limit)
		if err != nil {
			return ModelPoolPage{}, err
		}
		items, err := scanModelPoolAccountRows(rows)
		if err != nil {
			return ModelPoolPage{}, err
		}
		return ModelPoolPage{Items: items}, nil
	})
	if err != nil {
		return nil, err
	}
	return page.Items, nil
}

func scanModelPoolAccountRows(rows *sql.Rows) ([]controlplane.ModelPoolAccountSummary, error) {
	defer rows.Close()
	items := make([]controlplane.ModelPoolAccountSummary, 0)
	for rows.Next() {
		var (
			item                          controlplane.ModelPoolAccountSummary
			product                       sql.NullString
			cooldownUntil, testedAt       sql.NullTime
			activeLeases, dailyUsedTokens int
			payload                       []byte
		)
		if err := rows.Scan(&item.ID, &product, &item.Provider, &item.Model, &item.BaseURL, &item.SecretRef, &item.Status,
			&item.Priority, &item.ConcurrencyLimit, &item.DailyLimit, &cooldownUntil,
			&activeLeases, &dailyUsedTokens, &payload, &testedAt); err != nil {
			return nil, err
		}
		var err error
		item.Product, err = normalizedStoredProduct(product)
		if err != nil {
			return nil, err
		}
		item.SecretConfigured = item.SecretRef != ""
		item.ActiveLeases = activeLeases
		item.DailyUsedTokens = dailyUsedTokens
		if cooldownUntil.Valid {
			item.CooldownUntil = cooldownUntil.Time.UTC().Format(time.RFC3339)
		}
		if len(payload) > 0 {
			var result controlplane.ModelPoolConnectivityTestResult
			if err := json.Unmarshal(payload, &result); err != nil {
				return nil, fmt.Errorf("decode model pool test result for account %s: %w", item.ID, err)
			}
			item.LastTestStatus = result.Status
			item.LastTestedAt = result.TestedAt
		} else if testedAt.Valid {
			item.LastTestedAt = testedAt.Time.UTC().Format(time.RFC3339)
		}
		items = append(items, item)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return items, nil
}

func (s *PostgresRepository) ListModelLeasesPage(ctx context.Context, offset, limit int) (ModelLeasePage, error) {
	return s.ListModelLeasesPageWithOptions(ctx, ModelLeasePageOptions{Offset: offset, Limit: limit})
}

func (s *PostgresRepository) ListModelLeasesPageWithOptions(ctx context.Context, options ModelLeasePageOptions) (ModelLeasePage, error) {
	options, err := NormalizeModelLeasePageOptions(options)
	if err != nil {
		return ModelLeasePage{}, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		var page ModelLeasePage
		err = s.Run(ctx, func(state *State) error {
			items := make([]controlplane.ModelLeaseAdminSummary, 0, len(state.ModelLeases))
			now := s.Now()
			for _, lease := range state.ModelLeases {
				item := modelLeaseAdminSummary(lease)
				if !modelLeaseMatchesPageOptions(item, options, now) {
					continue
				}
				normalizeModelLeaseSummaryStatus(&item, now)
				items = append(items, item)
			}
			sortModelLeaseSummaries(items, options.Sort)
			page.Total = len(items)
			start, end := pageWindow(page.Total, options.Offset, options.Limit)
			page.Items = append([]controlplane.ModelLeaseAdminSummary(nil), items[start:end]...)
			return nil
		})
		return page, err
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (ModelLeasePage, error) {
		var page ModelLeasePage
		filterArgs := make([]any, 0, 7)
		where := make([]string, 0, 6)
		addArg := func(value any) string {
			filterArgs = append(filterArgs, value)
			return fmt.Sprintf("$%d", len(filterArgs))
		}
		if options.Status != "" {
			switch options.Status {
			case controlplane.ModelLeaseStatusActive:
				where = append(where, fmt.Sprintf("status = 'active' AND expires_at > %s", addArg(s.Now())))
			case controlplane.ModelLeaseStatusExpired:
				where = append(where, fmt.Sprintf("(status = 'expired' OR (status = 'active' AND expires_at <= %s))", addArg(s.Now())))
			default:
				where = append(where, fmt.Sprintf("status = %s", addArg(options.Status)))
			}
		}
		for _, filter := range []struct {
			column string
			value  string
		}{
			{column: "product", value: string(options.Product)},
			{column: "provider", value: options.Provider},
			{column: "model", value: options.Model},
			{column: "user_id", value: options.UserID},
			{column: "device_id", value: options.DeviceID},
			{column: "account_id", value: options.AccountID},
		} {
			if filter.value != "" {
				where = append(where, fmt.Sprintf("%s = %s", filter.column, addArg(filter.value)))
			}
		}
		whereSQL := ""
		if len(where) > 0 {
			whereSQL = " WHERE " + strings.Join(where, " AND ")
		}
		countQuery := "SELECT COUNT(*) FROM model_leases" + whereSQL
		if err := tx.QueryRowContext(ctx, countQuery, filterArgs...).Scan(&page.Total); err != nil {
			return ModelLeasePage{}, err
		}
		orderBy := "expires_at DESC, id DESC"
		switch options.Sort {
		case ModelLeaseSortExpiresAsc:
			orderBy = "expires_at ASC, id ASC"
		case ModelLeaseSortStatus:
			nowPlaceholder := addArg(s.Now())
			orderBy = fmt.Sprintf("CASE WHEN status = 'active' AND expires_at <= %s THEN 'expired' ELSE status END ASC, expires_at DESC, id DESC", nowPlaceholder)
		case ModelLeaseSortProviderModel:
			orderBy = "provider ASC, model ASC, id DESC"
		}
		limitPlaceholder := addArg(options.Limit)
		offsetPlaceholder := addArg(options.Offset)
		query := fmt.Sprintf(`
			SELECT id, product, account_id, user_id, device_id, purpose, status, expires_at,
			       provider, model, proxy_mode, concurrency_limit
			  FROM model_leases%s
			 ORDER BY %s
			 LIMIT %s OFFSET %s
		`, whereSQL, orderBy, limitPlaceholder, offsetPlaceholder)
		rows, err := tx.QueryContext(ctx, query, filterArgs...)
		if err != nil {
			return ModelLeasePage{}, err
		}
		defer rows.Close()
		for rows.Next() {
			var (
				item      controlplane.ModelLeaseAdminSummary
				product   sql.NullString
				expiresAt time.Time
			)
			dest := []any{&item.ID, &product, &item.AccountID, &item.UserID, &item.DeviceID, &item.Purpose, &item.Status,
				&expiresAt, &item.Provider, &item.Model, &item.ProxyMode, &item.ConcurrencyLimit}
			if err := rows.Scan(dest...); err != nil {
				return ModelLeasePage{}, err
			}
			item.Product, err = normalizedStoredProduct(product)
			if err != nil {
				return ModelLeasePage{}, err
			}
			item.ExpiresAt = expiresAt.UTC().Format(time.RFC3339)
			page.Items = append(page.Items, item)
		}
		if err := rows.Err(); err != nil {
			return ModelLeasePage{}, err
		}
		return page, nil
	})
}

func (s *PostgresRepository) GetModelLeaseAdminDetail(ctx context.Context, leaseID string) (controlplane.ModelLeaseAdminDetail, error) {
	return s.GetModelLeaseAdminDetailForProduct(ctx, leaseID, controlplane.ProductAutoLive)
}

func (s *PostgresRepository) GetModelLeaseAdminDetailForProduct(ctx context.Context, leaseID string, requestedProduct controlplane.ProductCode) (controlplane.ModelLeaseAdminDetail, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ModelLeaseAdminDetail{}, errors.New("normalized model lease detail requires normalized read source")
	}
	leaseID = strings.TrimSpace(leaseID)
	if leaseID == "" || !requestedProduct.Valid() {
		return controlplane.ModelLeaseAdminDetail{}, controlplane.ErrInvalidRequest
	}
	return runPostgresReadPage(s, ctx, func(ctx context.Context, tx *sql.Tx) (controlplane.ModelLeaseAdminDetail, error) {
		var (
			detail               controlplane.ModelLeaseAdminDetail
			product              sql.NullString
			createdAt, expiresAt time.Time
			releasedAt           sql.NullTime
		)
		err := tx.QueryRowContext(ctx, `
			SELECT id, product, account_id, user_id, device_id, purpose, status, created_at,
			       expires_at, released_at, provider, model, proxy_mode, concurrency_limit
		      FROM model_leases
		     WHERE id = $1 AND product = $2
		`, leaseID, requestedProduct).Scan(
			&detail.ID, &product, &detail.AccountID, &detail.UserID, &detail.DeviceID, &detail.Purpose,
			&detail.Status, &createdAt, &expiresAt, &releasedAt, &detail.Provider,
			&detail.Model, &detail.ProxyMode, &detail.ConcurrencyLimit,
		)
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.ModelLeaseAdminDetail{}, controlplane.ErrModelLeaseNotFound
		}
		if err != nil {
			return controlplane.ModelLeaseAdminDetail{}, err
		}
		detail.Product, err = normalizedStoredProduct(product)
		if err != nil {
			return controlplane.ModelLeaseAdminDetail{}, err
		}
		detail.CreatedAt = createdAt.UTC().Format(time.RFC3339)
		detail.ExpiresAt = expiresAt.UTC().Format(time.RFC3339)
		if releasedAt.Valid {
			detail.ReleasedAt = releasedAt.Time.UTC().Format(time.RFC3339)
		}
		now := s.Now()
		if detail.Status == controlplane.ModelLeaseStatusActive && !now.Before(expiresAt) {
			detail.Status = controlplane.ModelLeaseStatusExpired
			if detail.ReleasedAt == "" {
				detail.ReleasedAt = now.Format(time.RFC3339)
			}
		}
		return detail, nil
	})
}

var _ ProductModelLeaseDetailReader = (*PostgresRepository)(nil)

const devicePageQuery = `
	SELECT id, user_id, product, device_name, platform, client_version, status,
	       disk_free_bytes, memory_total_bytes, memory_available_bytes,
	       cpu_logical_cores, runtime_os_name, runtime_os_version,
	       kernel_version, current_media_name, playback_state, last_heartbeat_at
	FROM devices`

func scanDeviceRows(rows *sql.Rows) ([]controlplane.DeviceSummary, error) {
	items := make([]controlplane.DeviceSummary, 0)
	for rows.Next() {
		item, err := scanDeviceSummary(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return items, nil
}

type deviceRowScanner interface {
	Scan(dest ...any) error
}

func scanDeviceSummary(scanner deviceRowScanner) (controlplane.DeviceSummary, error) {
	var (
		item                                                            controlplane.DeviceSummary
		userID, product, runtimeOSName, runtimeOSVersion, kernelVersion sql.NullString
		currentMediaName, playbackState                                 sql.NullString
		lastHeartbeatAt                                                 sql.NullTime
	)
	if err := scanner.Scan(
		&item.ID, &userID, &product, &item.DeviceName, &item.Platform, &item.AppVersion, &item.Status,
		&item.DiskFreeBytes, &item.MemoryTotalBytes, &item.MemoryAvailableBytes,
		&item.CPULogicalCores, &runtimeOSName, &runtimeOSVersion,
		&kernelVersion, &currentMediaName, &playbackState, &lastHeartbeatAt,
	); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	item.UserID = userID.String
	storedProduct, err := normalizedStoredProduct(product)
	if err != nil {
		return controlplane.DeviceSummary{}, err
	}
	item.Product = storedProduct
	item.RuntimeOSName = runtimeOSName.String
	item.RuntimeOSVersion = runtimeOSVersion.String
	item.KernelVersion = kernelVersion.String
	item.CurrentMediaName = currentMediaName.String
	item.PlaybackState = playbackState.String
	if lastHeartbeatAt.Valid {
		item.LastSeenAt = lastHeartbeatAt.Time.UTC().Format(time.RFC3339)
	}
	return item, nil
}

// normalizedProductFilter keeps strict product lookups parameterized for
// non-default products while preserving the legacy autolive SQL shape used by
// the compatibility readers and their existing contracts.
func normalizedProductFilter(column string, product controlplane.ProductCode, placeholder int) (string, []any) {
	if product == controlplane.ProductAutoLive {
		return column + " = 'autolive'", nil
	}
	return fmt.Sprintf("%s = $%d", column, placeholder), []any{product}
}

func (s *PostgresRepository) operationContext(ctx context.Context) (context.Context, context.CancelFunc) {
	if s.operationTimeout <= 0 {
		return ctx, func() {}
	}
	return context.WithTimeout(ctx, s.operationTimeout)
}

func postgresOperationError(ctx context.Context, err error) error {
	if err == nil {
		return nil
	}
	if contextErr := ctx.Err(); contextErr != nil {
		return contextErr
	}
	return err
}

// ErrCommitOutcomeUnknown means the database may have committed the
// transaction even though the client did not receive a successful COMMIT
// response. Callers must not compensate by deleting facts that could now be
// referenced by committed business rows.
var ErrCommitOutcomeUnknown = controlplane.ErrCommitOutcomeUnknown

func postgresCommitError(ctx context.Context, operation string, err error) error {
	if err == nil {
		return nil
	}
	if contextErr := ctx.Err(); contextErr != nil {
		return contextErr
	}
	if errors.Is(err, context.Canceled) {
		return context.Canceled
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return context.DeadlineExceeded
	}
	return fmt.Errorf("%s: %w", operation, ErrCommitOutcomeUnknown)
}

func (s *PostgresRepository) Run(ctx context.Context, fn StateOperation) error {
	return s.run(ctx, fn, nil)
}

// RunWithSessionBinding atomically binds the authenticated session to a
// device and applies the control-plane state mutation. The session row is
// locked before the domain state is loaded, so an activation/heartbeat cannot
// commit a session binding without its corresponding device state (or vice
// versa).
func (s *PostgresRepository) RunWithSessionBinding(ctx context.Context, accessTokenHash, userID, deviceID string, fn StateOperation) error {
	if strings.TrimSpace(accessTokenHash) == "" || strings.TrimSpace(userID) == "" || strings.TrimSpace(deviceID) == "" {
		return errors.New("session binding requires access token hash, user id and device id")
	}
	return s.run(ctx, fn, func(operationCtx context.Context, tx *sql.Tx) error {
		var sessionUserID string
		var currentDeviceID sql.NullString
		err := tx.QueryRowContext(operationCtx, `
			SELECT user_id, device_id
			FROM auth_sessions
			WHERE access_token_hash = $1 AND revoked_at IS NULL
			FOR UPDATE
		`, accessTokenHash).Scan(&sessionUserID, &currentDeviceID)
		if errors.Is(err, sql.ErrNoRows) {
			return errors.New("auth session not found for device binding")
		}
		if err != nil {
			return postgresOperationError(operationCtx, fmt.Errorf("lock auth session for device binding: %w", err))
		}
		if sessionUserID != userID {
			return errors.New("auth session user does not match request user")
		}
		if currentDeviceID.Valid && currentDeviceID.String != deviceID {
			return ErrSessionDeviceBindingConflict
		}
		if !currentDeviceID.Valid {
			if _, err := tx.ExecContext(operationCtx, `
				UPDATE auth_sessions
				SET device_id = $2, device_bound_at = CURRENT_TIMESTAMP
				WHERE access_token_hash = $1 AND revoked_at IS NULL
			`, accessTokenHash, deviceID); err != nil {
				return postgresOperationError(operationCtx, fmt.Errorf("bind auth session to device: %w", err))
			}
		}
		return nil
	})
}

func (s *PostgresRepository) run(ctx context.Context, fn StateOperation, before func(context.Context, *sql.Tx) error) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	ctx = operationCtx
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	if s.modelReadSource == ModelReadSourceNormalized {
		// A transaction-scoped advisory lock gives all StateOperation callers one
		// cross-instance serialization point while domain repositories are being
		// migrated. It avoids last-writer-wins reads across API instances.
		if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()`); err != nil {
			_ = tx.Rollback()
			return postgresOperationError(ctx, fmt.Errorf("lock normalized control plane: %w", err))
		}
	}
	if before != nil {
		if err := before(ctx, tx); err != nil {
			_ = tx.Rollback()
			return postgresOperationError(ctx, err)
		}
	}
	state, err := s.loadState(ctx, tx)
	if err != nil {
		_ = tx.Rollback()
		return postgresOperationError(ctx, err)
	}
	previousRefs := modelPoolSecretRefs(state)
	committed := false
	commitAttempted := false
	defer func() {
		_ = tx.Rollback()
		// Once COMMIT has been sent, an error no longer proves rollback. Keep
		// staged references until the bounded reconciler can compare them with
		// the durable active reference.
		if !committed && !commitAttempted && s.secretStore != nil {
			cleanupCtx, cleanupCancel := context.WithTimeout(context.WithoutCancel(ctx), s.operationTimeout)
			defer cleanupCancel()
			for _, reference := range newModelPoolSecretRefs(previousRefs, modelPoolSecretRefs(state)) {
				_ = s.secretStore.Delete(cleanupCtx, reference)
			}
		}
	}()
	if err := fn(state); err != nil {
		return err
	}
	if err := s.syncReferenceRows(ctx, tx, state); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("sync postgres reference rows: %w", err))
	}
	if s.modelReadSource == ModelReadSourceSnapshot {
		payload, err := marshalStateSnapshot(state)
		if err != nil {
			return postgresOperationError(ctx, fmt.Errorf("marshal postgres state snapshot: %w", err))
		}
		if _, err := tx.ExecContext(ctx, `
			UPDATE control_plane_state SET state = $1, updated_at = CURRENT_TIMESTAMP WHERE id = TRUE
		`, payload); err != nil {
			return postgresOperationError(ctx, err)
		}
	}
	commitAttempted = true
	if err := tx.Commit(); err != nil {
		return postgresCommitError(ctx, "commit control-plane transaction", err)
	}
	committed = true
	return nil
}

// BackfillNormalized copies the legacy snapshot into the normalized shadow
// tables in one transaction. It is an explicit migration operation, never an
// automatic startup fallback; callers must run it before enabling normalized
// reads and verify its result in the same transaction.
func (s *PostgresRepository) BackfillNormalized(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	ctx = operationCtx
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return postgresOperationError(ctx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0))`); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("lock normalized backfill: %w", err))
	}
	var backfillStatus string
	if err := tx.QueryRowContext(ctx, `SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR UPDATE`).Scan(&backfillStatus); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return errors.New("normalized backfill state is missing; run database migrations before backfill")
		}
		return postgresOperationError(ctx, fmt.Errorf("read normalized backfill state: %w", err))
	}
	if backfillStatus != "pending" {
		return errors.New("normalized backfill is already completed; refusing to rerun and overwrite normalized data")
	}
	state, err := s.loadLocked(ctx, tx)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("load snapshot for normalized backfill: %w", err))
	}
	if err := s.syncReferenceRows(ctx, tx, state); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("write normalized backfill: %w", err))
	}
	if err := verifyNormalizedRowCounts(ctx, tx, state); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("verify normalized backfill: %w", err))
	}
	result, err := tx.ExecContext(ctx, `
		UPDATE normalized_backfill_state
		SET status = 'completed', completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
		WHERE id = TRUE
	`)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("mark normalized backfill completed: %w", err))
	}
	rowsAffected, err := result.RowsAffected()
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("count normalized backfill completion marker: %w", err))
	}
	if rowsAffected != 1 {
		return postgresOperationError(ctx, errors.New("normalized backfill completion marker is missing"))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(ctx, "commit normalized backfill", err)
	}
	return nil
}

func (s *PostgresRepository) loadState(ctx context.Context, tx *sql.Tx) (*State, error) {
	if s.modelReadSource == ModelReadSourceNormalized {
		return s.loadNormalized(ctx, tx)
	}
	return s.loadLocked(ctx, tx)
}

func (s *PostgresRepository) loadLocked(ctx context.Context, tx *sql.Tx) (*State, error) {
	var raw []byte
	if err := tx.QueryRowContext(ctx, `SELECT state FROM control_plane_state WHERE id = TRUE FOR UPDATE`).Scan(&raw); err != nil {
		return nil, err
	}
	state, err := unmarshalStateSnapshot(raw)
	if err != nil {
		return nil, err
	}
	// Secret references are deliberately omitted from the JSON snapshot. Hydrate
	// the active reference from the protected model_accounts row so staged key
	// rotation survives a process restart without exposing the reference in JSONB.
	rows, err := tx.QueryContext(ctx, `SELECT id, secret_ref FROM model_accounts`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var id, secretRef string
		if err := rows.Scan(&id, &secretRef); err != nil {
			return nil, err
		}
		if account, exists := state.ModelPoolAccounts[id]; exists && secretRef != "" {
			account.SecretRef = secretRef
			state.ModelPoolAccounts[id] = account
		}
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	return state, nil
}

func (s *PostgresRepository) loadNormalized(ctx context.Context, tx *sql.Tx) (*State, error) {
	state := NewState()

	users, err := tx.QueryContext(ctx, `
		SELECT id, username, password_hash, role, status, created_at
		FROM users
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for users.Next() {
		var id, username, passwordHash, role, status string
		var createdAt time.Time
		if err := users.Scan(&id, &username, &passwordHash, &role, &status, &createdAt); err != nil {
			_ = users.Close()
			return nil, err
		}
		state.Users[id] = controlplane.UserSummary{
			ID: id, Username: username, Role: role, Status: status,
			CreatedAt: createdAt.UTC().Format(time.RFC3339),
		}
		state.UserCredentialHashes[id] = []byte(passwordHash)
	}
	if err := users.Err(); err != nil {
		_ = users.Close()
		return nil, err
	}
	if err := users.Close(); err != nil {
		return nil, err
	}

	policies, err := tx.QueryContext(ctx, `
		SELECT user_id, allowed_models, daily_token_limit, updated_at
		FROM user_authorization_policies
		ORDER BY user_id
	`)
	if err != nil {
		return nil, err
	}
	for policies.Next() {
		var userID string
		var allowedModelsJSON []byte
		var dailyTokenLimit int
		var updatedAt time.Time
		if err := policies.Scan(&userID, &allowedModelsJSON, &dailyTokenLimit, &updatedAt); err != nil {
			_ = policies.Close()
			return nil, err
		}
		var allowedModels []string
		if err := json.Unmarshal(allowedModelsJSON, &allowedModels); err != nil {
			_ = policies.Close()
			return nil, fmt.Errorf("decode user authorization policy %s: %w", userID, err)
		}
		if _, ok := state.Users[userID]; !ok {
			_ = policies.Close()
			return nil, fmt.Errorf("user authorization policy %s references missing user", userID)
		}
		state.UserAuthorizationPolicies[userID] = controlplane.UserAuthorizationPolicy{
			UserID: userID, AllowedModels: allowedModels, DailyTokenLimit: dailyTokenLimit,
			UpdatedAt: updatedAt.UTC().Format(time.RFC3339),
		}
	}
	if err := policies.Err(); err != nil {
		_ = policies.Close()
		return nil, err
	}
	if err := policies.Close(); err != nil {
		return nil, err
	}

	devices, err := tx.QueryContext(ctx, `
		SELECT id, user_id, product, device_name, platform, client_version, status,
		       disk_free_bytes, memory_total_bytes, memory_available_bytes,
		       cpu_logical_cores, runtime_os_name, runtime_os_version,
		       kernel_version, current_media_name, playback_state, last_heartbeat_at
		FROM devices
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for devices.Next() {
		var (
			id, deviceName, platform, clientVersion, status                 string
			userID, product, runtimeOSName, runtimeOSVersion, kernelVersion sql.NullString
			currentMediaName, playbackState                                 sql.NullString
			lastHeartbeatAt                                                 sql.NullTime
			diskFreeBytes, memoryTotalBytes, memoryAvailableBytes           int64
			cpuLogicalCores                                                 int
		)
		if err := devices.Scan(
			&id, &userID, &product, &deviceName, &platform, &clientVersion, &status,
			&diskFreeBytes, &memoryTotalBytes, &memoryAvailableBytes,
			&cpuLogicalCores, &runtimeOSName, &runtimeOSVersion,
			&kernelVersion, &currentMediaName, &playbackState, &lastHeartbeatAt,
		); err != nil {
			_ = devices.Close()
			return nil, err
		}
		storedProduct, err := normalizedStoredProduct(product)
		if err != nil {
			_ = devices.Close()
			return nil, err
		}
		device := controlplane.DeviceSummary{
			ID: id, UserID: userID.String, Product: storedProduct, DeviceName: deviceName, Platform: platform,
			AppVersion: clientVersion, Status: status, DiskFreeBytes: diskFreeBytes,
			MemoryTotalBytes: memoryTotalBytes, MemoryAvailableBytes: memoryAvailableBytes,
			CPULogicalCores: cpuLogicalCores, RuntimeOSName: runtimeOSName.String,
			RuntimeOSVersion: runtimeOSVersion.String, KernelVersion: kernelVersion.String,
			CurrentMediaName: currentMediaName.String, PlaybackState: playbackState.String,
		}
		if lastHeartbeatAt.Valid {
			device.LastSeenAt = lastHeartbeatAt.Time.UTC().Format(time.RFC3339)
		}
		state.Devices[id] = device
	}
	if err := devices.Err(); err != nil {
		_ = devices.Close()
		return nil, err
	}
	if err := devices.Close(); err != nil {
		return nil, err
	}

	activationCodes, err := tx.QueryContext(ctx, `
		SELECT id, product, bound_user_id, code_hash, code_prefix, status, expires_at, used_at,
		       used_by_user_id, used_by_device_id, max_devices, bound_devices
		FROM activation_codes
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for activationCodes.Next() {
		var (
			id, codeHash, codePrefix, status                   string
			expiresAt, usedAt                                  sql.NullTime
			product, boundUserID, usedByUserID, usedByDeviceID sql.NullString
			maxDevices, boundDevices                           int
		)
		if err := activationCodes.Scan(&id, &product, &boundUserID, &codeHash, &codePrefix, &status, &expiresAt, &usedAt, &usedByUserID, &usedByDeviceID, &maxDevices, &boundDevices); err != nil {
			_ = activationCodes.Close()
			return nil, err
		}
		storedProduct, err := normalizedStoredProduct(product)
		if err != nil {
			_ = activationCodes.Close()
			return nil, err
		}
		record := ActivationCodeRecord{
			ActivationCode: controlplane.ActivationCode{
				ID: id, Product: storedProduct, UserID: boundUserID.String, Status: status, CodePrefix: codePrefix, MaxDevices: maxDevices, BoundDevices: boundDevices,
				UsedByUserID: usedByUserID.String, UsedByDeviceID: usedByDeviceID.String,
			},
			CodePrefix: codePrefix, UsedByUserID: usedByUserID.String,
			UsedByDeviceID: usedByDeviceID.String,
		}
		if expiresAt.Valid {
			record.ActivationCode.ExpiresAt = expiresAt.Time.UTC().Format(time.RFC3339)
		}
		if usedAt.Valid {
			record.ActivationCode.UsedAt = usedAt.Time.UTC().Format(time.RFC3339)
			record.UsedAt = record.ActivationCode.UsedAt
		}
		state.ActivationCodes[id] = record
		state.ActivationCodeIndex[codeHash] = id
	}
	if err := activationCodes.Err(); err != nil {
		_ = activationCodes.Close()
		return nil, err
	}
	if err := activationCodes.Close(); err != nil {
		return nil, err
	}

	activationBindings, err := tx.QueryContext(ctx, `
		SELECT activation_code_id, device_id
		FROM activation_device_bindings
		ORDER BY device_id
	`)
	if err != nil {
		return nil, err
	}
	for activationBindings.Next() {
		var activationCodeID, deviceID string
		if err := activationBindings.Scan(&activationCodeID, &deviceID); err != nil {
			_ = activationBindings.Close()
			return nil, err
		}
		state.ActivationDeviceBindings[deviceID] = activationCodeID
	}
	if err := activationBindings.Err(); err != nil {
		_ = activationBindings.Close()
		return nil, err
	}
	if err := activationBindings.Close(); err != nil {
		return nil, err
	}

	accounts, err := tx.QueryContext(ctx, `
		SELECT id, provider, model, base_url, secret_ref, status, priority,
		       concurrency_limit, daily_token_limit, cooldown_until
		FROM model_accounts
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for accounts.Next() {
		var id, provider, model, baseURL, secretRef, status string
		var priority, dailyLimit, concurrencyLimit int
		var cooldownUntil sql.NullTime
		if err := accounts.Scan(&id, &provider, &model, &baseURL, &secretRef, &status, &priority, &concurrencyLimit, &dailyLimit, &cooldownUntil); err != nil {
			_ = accounts.Close()
			return nil, err
		}
		account := controlplane.ModelPoolAccountSummary{
			ID: id, Provider: provider, Model: model, BaseURL: baseURL, Status: status,
			Priority: priority, DailyLimit: dailyLimit, ConcurrencyLimit: concurrencyLimit,
			SecretConfigured: secretRef != "", SecretRef: secretRef,
		}
		if cooldownUntil.Valid {
			account.CooldownUntil = cooldownUntil.Time.UTC().Format(time.RFC3339)
		}
		state.ModelPoolAccounts[id] = account
	}
	if err := accounts.Err(); err != nil {
		_ = accounts.Close()
		return nil, err
	}
	if err := accounts.Close(); err != nil {
		return nil, err
	}

	leases, err := tx.QueryContext(ctx, `
		SELECT id, account_id, user_id, device_id, purpose, status, expires_at,
		       created_at, released_at, provider, model, proxy_mode, concurrency_limit
		FROM model_leases
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for leases.Next() {
		var id, accountID, userID, deviceID, purpose, status, provider, model, proxyMode string
		var expiresAt, createdAt time.Time
		var releasedAt sql.NullTime
		var concurrencyLimit int
		if err := leases.Scan(&id, &accountID, &userID, &deviceID, &purpose, &status, &expiresAt, &createdAt, &releasedAt, &provider, &model, &proxyMode, &concurrencyLimit); err != nil {
			_ = leases.Close()
			return nil, err
		}
		lease := controlplane.ModelLease{
			ID: id, AccountID: accountID, UserID: userID, DeviceID: deviceID,
			Purpose: purpose, Provider: provider, Model: model, Status: status,
			CreatedAt: createdAt.UTC().Format(time.RFC3339),
			ExpiresAt: expiresAt.UTC().Format(time.RFC3339), ProxyMode: proxyMode,
			ConcurrencyLimit: concurrencyLimit,
		}
		if releasedAt.Valid {
			lease.ReleasedAt = releasedAt.Time.UTC().Format(time.RFC3339)
		}
		state.ModelLeases[id] = lease
	}
	if err := leases.Err(); err != nil {
		_ = leases.Close()
		return nil, err
	}
	if err := leases.Close(); err != nil {
		return nil, err
	}

	usageRows, err := tx.QueryContext(ctx, `
		SELECT id, lease_id, client_call_id, request_id, provider, model,
		       prompt_tokens, completion_tokens, total_tokens, latency_ms,
		       status, usage_source, error_code, created_at
		FROM model_usage_records
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for usageRows.Next() {
		var id, requestID, clientCallID, provider, model, status, usageSource string
		var leaseID, errorCode sql.NullString
		var inputTokens, outputTokens, totalTokens int
		var latencyMS int64
		var createdAt time.Time
		if err := usageRows.Scan(&id, &leaseID, &clientCallID, &requestID, &provider, &model, &inputTokens, &outputTokens, &totalTokens, &latencyMS, &status, &usageSource, &errorCode, &createdAt); err != nil {
			_ = usageRows.Close()
			return nil, err
		}
		state.ModelUsageRecords[id] = controlplane.ModelUsageRecord{
			ID: id, LeaseID: leaseID.String, ClientCallID: clientCallID, RequestID: requestID,
			Provider: provider, Model: model, InputTokens: inputTokens, OutputTokens: outputTokens,
			TotalTokens: totalTokens, LatencyMS: latencyMS, Status: status,
			UsageSource: usageSource, ErrorCode: errorCode.String,
			CreatedAt: createdAt.UTC().Format(time.RFC3339),
		}
	}
	if err := usageRows.Err(); err != nil {
		_ = usageRows.Close()
		return nil, err
	}
	if err := usageRows.Close(); err != nil {
		return nil, err
	}

	testRows, err := tx.QueryContext(ctx, `
		SELECT id, payload
		FROM model_pool_test_results
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for testRows.Next() {
		var id string
		var payload []byte
		if err := testRows.Scan(&id, &payload); err != nil {
			_ = testRows.Close()
			return nil, err
		}
		var result controlplane.ModelPoolConnectivityTestResult
		if err := json.Unmarshal(payload, &result); err != nil {
			_ = testRows.Close()
			return nil, fmt.Errorf("decode model pool test result %s: %w", id, err)
		}
		state.ModelPoolTestResults[id] = result
		if account, ok := state.ModelPoolAccounts[result.AccountID]; ok {
			account.LastTestStatus = result.Status
			account.LastTestedAt = result.TestedAt
			state.ModelPoolAccounts[result.AccountID] = account
		}
	}
	if err := testRows.Err(); err != nil {
		_ = testRows.Close()
		return nil, err
	}
	if err := testRows.Close(); err != nil {
		return nil, err
	}

	idempotencyRows, err := tx.QueryContext(ctx, `
		SELECT idempotency_key, fingerprint, resource_id
		FROM idempotency_records
		WHERE scope = 'control-plane-state'
		ORDER BY idempotency_key
	`)
	if err != nil {
		return nil, err
	}
	for idempotencyRows.Next() {
		var key, fingerprint, resourceID string
		if err := idempotencyRows.Scan(&key, &fingerprint, &resourceID); err != nil {
			_ = idempotencyRows.Close()
			return nil, err
		}
		state.IdempotencyRecords[key] = IdempotencyRecord{Fingerprint: fingerprint, ResourceID: resourceID}
	}
	if err := idempotencyRows.Err(); err != nil {
		_ = idempotencyRows.Close()
		return nil, err
	}
	if err := idempotencyRows.Close(); err != nil {
		return nil, err
	}

	auditRows, err := tx.QueryContext(ctx, `
		SELECT id, product, actor_user_id, device_id, action, resource_type, resource_id, request_id, outcome, status_code, error_code, created_at
		FROM audit_logs
		ORDER BY id
	`)
	if err != nil {
		return nil, err
	}
	for auditRows.Next() {
		var id, action, resourceType string
		var product sql.NullString
		var actorUserID, deviceID, resourceID, requestID, outcome, errorCode sql.NullString
		var statusCode sql.NullInt64
		var createdAt time.Time
		if err := auditRows.Scan(&id, &product, &actorUserID, &deviceID, &action, &resourceType, &resourceID, &requestID, &outcome, &statusCode, &errorCode, &createdAt); err != nil {
			_ = auditRows.Close()
			return nil, err
		}
		storedProduct, err := normalizedAuditProduct(product)
		if err != nil {
			_ = auditRows.Close()
			return nil, err
		}
		state.AuditLogs[id] = controlplane.AuditLog{
			ID: id, Product: storedProduct, ActorUserID: actorUserID.String, DeviceID: deviceID.String, Action: action,
			TargetType: resourceType, TargetID: resourceID.String, RequestID: requestID.String,
			Outcome: outcome.String, StatusCode: int(statusCode.Int64), ErrorCode: errorCode.String,
			CreatedAt: createdAt.UTC().Format(time.RFC3339),
		}
	}
	if err := auditRows.Err(); err != nil {
		_ = auditRows.Close()
		return nil, err
	}
	if err := auditRows.Close(); err != nil {
		return nil, err
	}
	if err := s.ensureNormalizedCoverage(ctx, tx, state); err != nil {
		return nil, err
	}

	for id, account := range state.ModelPoolAccounts {
		account.ActiveLeases = activeLeasesForAccount(state, id)
		account.DailyUsedTokens = dailyUsedTokensForAccount(state, id, s.Now())
		state.ModelPoolAccounts[id] = account
	}
	seedSequenceCounters(state)
	return state, nil
}

// ensureNormalizedCoverage keeps the explicit backfill marker as the only
// normalized-read gate. The legacy snapshot is a migration input, not a
// runtime completeness oracle; consulting it here would keep normalized
// reads dependent on the table they are retiring.
func (s *PostgresRepository) ensureNormalizedCoverage(ctx context.Context, tx *sql.Tx, _ *State) error {
	return ensureNormalizedBackfillReady(ctx, tx)
}

// ensureNormalizedCoverageCounts applies the same explicit backfill gate to
// bounded page reads without loading or counting the legacy snapshot.
func (s *PostgresRepository) ensureNormalizedCoverageCounts(ctx context.Context, tx *sql.Tx) error {
	return ensureNormalizedBackfillReady(ctx, tx)
}

func verifyNormalizedRowCounts(ctx context.Context, tx *sql.Tx, snapshot *State) error {
	checks := []struct {
		name     string
		query    string
		expected int
	}{
		{"users", `SELECT COUNT(*) FROM users`, len(snapshot.Users)},
		{"user_authorization_policies", `SELECT COUNT(*) FROM user_authorization_policies`, len(snapshot.UserAuthorizationPolicies)},
		{"devices", `SELECT COUNT(*) FROM devices`, len(snapshot.Devices)},
		{"activation_codes", `SELECT COUNT(*) FROM activation_codes`, len(snapshot.ActivationCodes)},
		{"activation_device_bindings", `SELECT COUNT(*) FROM activation_device_bindings`, len(snapshot.ActivationDeviceBindings)},
		{"model_accounts", `SELECT COUNT(*) FROM model_accounts`, len(snapshot.ModelPoolAccounts)},
		{"model_leases", `SELECT COUNT(*) FROM model_leases`, len(snapshot.ModelLeases)},
		{"model_usage_records", `SELECT COUNT(*) FROM model_usage_records`, len(snapshot.ModelUsageRecords)},
		{"model_pool_test_results", `SELECT COUNT(*) FROM model_pool_test_results`, len(snapshot.ModelPoolTestResults)},
		{"idempotency_records", `SELECT COUNT(*) FROM idempotency_records WHERE scope = 'control-plane-state'`, len(snapshot.IdempotencyRecords)},
		{"audit_logs", `SELECT COUNT(*) FROM audit_logs`, len(snapshot.AuditLogs)},
	}
	for _, check := range checks {
		var actual int
		if err := tx.QueryRowContext(ctx, check.query).Scan(&actual); err != nil {
			return fmt.Errorf("count normalized table %s: %w", check.name, err)
		}
		if actual < check.expected {
			return fmt.Errorf("normalized table %s contains %d rows after backfill, want at least %d", check.name, actual, check.expected)
		}
	}
	return nil
}

func seedSequenceCounters(state *State) {
	for _, entries := range [][]string{
		mapKeys(state.Users), mapKeys(state.Devices), mapKeys(state.ActivationCodes),
		mapKeys(state.ModelPoolAccounts), mapKeys(state.ModelLeases), mapKeys(state.ModelUsageRecords),
		mapKeys(state.ModelPoolTestResults), mapKeys(state.AuditLogs),
	} {
		for _, id := range entries {
			prefix, value, ok := splitSequenceID(id)
			if !ok || value <= state.SequenceCounters[prefix] {
				continue
			}
			state.SequenceCounters[prefix] = value
		}
	}
}

func mapKeys[T any](values map[string]T) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	return keys
}

func splitSequenceID(value string) (string, int, bool) {
	separator := strings.LastIndexByte(value, '_')
	if separator <= 0 || separator == len(value)-1 {
		return "", 0, false
	}
	number, err := strconv.Atoi(value[separator+1:])
	if err != nil {
		return "", 0, false
	}
	return value[:separator], number, true
}

func (s *PostgresRepository) syncReferenceRows(ctx context.Context, tx *sql.Tx, state *State) error {
	type priorActivationBinding struct {
		activationCodeID string
		product          controlplane.ProductCode
		userID           string
		boundAt          time.Time
	}
	priorBindings := make(map[string]priorActivationBinding)
	// Returning the deleted rows allows parent records to be updated safely while
	// preserving bound_at for bindings whose ownership did not change.
	rows, err := tx.QueryContext(ctx, `
		DELETE FROM activation_device_bindings
		RETURNING activation_code_id, device_id, product, user_id, bound_at
	`)
	if err != nil {
		return err
	}
	for rows.Next() {
		var activationCodeID, deviceID, userID string
		var product controlplane.ProductCode
		var boundAt time.Time
		if err := rows.Scan(&activationCodeID, &deviceID, &product, &userID, &boundAt); err != nil {
			_ = rows.Close()
			return err
		}
		priorBindings[deviceID] = priorActivationBinding{activationCodeID: activationCodeID, product: product, userID: userID, boundAt: boundAt.UTC()}
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return err
	}
	if err := rows.Close(); err != nil {
		return err
	}
	for id, user := range state.Users {
		passwordHash := string(state.UserCredentialHashes[id])
		if passwordHash == "" {
			passwordHash = "!configured-outside-control-plane!"
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO users (id, username, password_hash, role, status, created_at, disabled_at)
			VALUES ($1, $2, $3, $4, $5, $6, NULL)
			ON CONFLICT (id) DO UPDATE SET username = EXCLUDED.username, password_hash = CASE WHEN EXCLUDED.password_hash = '!configured-outside-control-plane!' THEN users.password_hash ELSE EXCLUDED.password_hash END, role = EXCLUDED.role, status = EXCLUDED.status
		`, id, user.Username, passwordHash, user.Role, user.Status, user.CreatedAt); err != nil {
			return err
		}
	}
	for userID, policy := range state.UserAuthorizationPolicies {
		allowedModels, err := json.Marshal(policy.AllowedModels)
		if err != nil {
			return fmt.Errorf("marshal user authorization policy %s: %w", userID, err)
		}
		updatedAt := any(s.Now())
		if policy.UpdatedAt != "" {
			parsed, parseErr := time.Parse(time.RFC3339, policy.UpdatedAt)
			if parseErr != nil {
				return fmt.Errorf("user authorization policy %s has invalid updated_at: %w", userID, parseErr)
			}
			updatedAt = parsed
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO user_authorization_policies (user_id, allowed_models, daily_token_limit, updated_at)
			VALUES ($1, $2::jsonb, $3, $4)
			ON CONFLICT (user_id) DO UPDATE SET allowed_models = EXCLUDED.allowed_models, daily_token_limit = EXCLUDED.daily_token_limit, updated_at = EXCLUDED.updated_at
		`, userID, allowedModels, policy.DailyTokenLimit, updatedAt); err != nil {
			return err
		}
	}
	for id, device := range state.Devices {
		var userID any
		if device.UserID != "" {
			userID = device.UserID
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO devices (id, user_id, product, device_key, device_name, platform, client_version, status, disk_free_bytes, memory_total_bytes, memory_available_bytes, cpu_logical_cores, runtime_os_name, runtime_os_version, kernel_version, current_media_name, playback_state, last_heartbeat_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, NULLIF($18, '')::timestamptz)
			ON CONFLICT (id) DO UPDATE SET user_id = EXCLUDED.user_id, product = EXCLUDED.product, device_name = EXCLUDED.device_name, platform = EXCLUDED.platform, client_version = EXCLUDED.client_version, status = EXCLUDED.status, disk_free_bytes = EXCLUDED.disk_free_bytes, memory_total_bytes = EXCLUDED.memory_total_bytes, memory_available_bytes = EXCLUDED.memory_available_bytes, cpu_logical_cores = EXCLUDED.cpu_logical_cores, runtime_os_name = EXCLUDED.runtime_os_name, runtime_os_version = EXCLUDED.runtime_os_version, kernel_version = EXCLUDED.kernel_version, current_media_name = EXCLUDED.current_media_name, playback_state = EXCLUDED.playback_state, last_heartbeat_at = EXCLUDED.last_heartbeat_at
		`, id, userID, compatibilityStoredProduct(device.Product), "state-device/"+id, device.DeviceName, device.Platform, device.AppVersion, device.Status, device.DiskFreeBytes, device.MemoryTotalBytes, device.MemoryAvailableBytes, device.CPULogicalCores, device.RuntimeOSName, device.RuntimeOSVersion, device.KernelVersion, device.CurrentMediaName, device.PlaybackState, device.LastSeenAt); err != nil {
			return err
		}
	}
	for id, record := range state.ActivationCodes {
		digest, ok := activationCodeHash(state, id)
		if !ok {
			return fmt.Errorf("activation code %s has no hash index", id)
		}
		expiresAt, err := time.Parse(time.RFC3339, record.ActivationCode.ExpiresAt)
		if err != nil {
			return fmt.Errorf("activation code %s has invalid expiry: %w", id, err)
		}
		prefix := record.CodePrefix
		if prefix == "" {
			prefix = "code_"
		}
		var usedAt any
		if record.UsedAt != "" {
			parsed, parseErr := time.Parse(time.RFC3339, record.UsedAt)
			if parseErr != nil {
				return fmt.Errorf("activation code %s has invalid used_at: %w", id, parseErr)
			}
			usedAt = parsed
		} else if record.ActivationCode.Status == controlplane.ActivationCodeStatusUsed {
			usedAt = s.Now()
		}
		usedByUserID := record.UsedByUserID
		if usedByUserID == "" && record.UsedByDeviceID != "" {
			if device, exists := state.Devices[record.UsedByDeviceID]; exists {
				usedByUserID = device.UserID
			}
		}
		boundUserID := strings.TrimSpace(record.ActivationCode.UserID)
		status := record.ActivationCode.Status
		if boundUserID == "" && (status == controlplane.ActivationCodeStatusActive || status == controlplane.ActivationCodeStatusUsed) {
			status = controlplane.ActivationCodeStatusRevoked
			record.ActivationCode.Status = status
			state.ActivationCodes[id] = record
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO activation_codes (id, product, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, used_at, used_by_user_id, used_by_device_id, max_devices, bound_devices)
			VALUES ($1, $2, NULLIF($3, ''), $4, $5, $6, CURRENT_TIMESTAMP, $7, $8, NULLIF($9, ''), NULLIF($10, ''), $11, $12)
			ON CONFLICT (id) DO UPDATE SET product = EXCLUDED.product, bound_user_id = EXCLUDED.bound_user_id, code_hash = EXCLUDED.code_hash, code_prefix = EXCLUDED.code_prefix, status = EXCLUDED.status, expires_at = EXCLUDED.expires_at, used_by_user_id = EXCLUDED.used_by_user_id, used_by_device_id = EXCLUDED.used_by_device_id, max_devices = EXCLUDED.max_devices, bound_devices = EXCLUDED.bound_devices
		`, id, compatibilityStoredProduct(record.ActivationCode.Product), boundUserID, digest, prefix, status, expiresAt, usedAt, usedByUserID, record.UsedByDeviceID, max(1, record.ActivationCode.MaxDevices), max(0, record.ActivationCode.BoundDevices)); err != nil {
			return err
		}
	}
	for deviceID, activationCodeID := range state.ActivationDeviceBindings {
		device, deviceExists := state.Devices[deviceID]
		record, codeExists := state.ActivationCodes[activationCodeID]
		if !deviceExists || !codeExists || device.UserID == "" || record.ActivationCode.UserID == "" {
			delete(state.ActivationDeviceBindings, deviceID)
			continue
		}
		product := compatibilityStoredProduct(device.Product)
		if product != compatibilityStoredProduct(record.ActivationCode.Product) || device.UserID != record.ActivationCode.UserID {
			return fmt.Errorf("activation binding %s has mismatched account or product", deviceID)
		}
		boundAt := s.Now()
		if prior, ok := priorBindings[deviceID]; ok && prior.activationCodeID == activationCodeID && prior.product == product && prior.userID == device.UserID {
			boundAt = prior.boundAt
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO activation_device_bindings (activation_code_id, device_id, product, user_id, bound_at)
			VALUES ($1, $2, $3, $4, $5)
		`, activationCodeID, deviceID, product, device.UserID, boundAt); err != nil {
			return err
		}
	}
	for id, account := range state.ModelPoolAccounts {
		if account.SecretRef == "" {
			return fmt.Errorf("model account %s has no secret reference", id)
		}
		activeLeases := activeLeasesForAccount(state, id)
		dailyUsedTokens := dailyUsedTokensForAccount(state, id, s.Now())
		var cooldownUntil any
		if account.CooldownUntil != "" {
			parsed, err := time.Parse(time.RFC3339, account.CooldownUntil)
			if err != nil {
				return fmt.Errorf("model account %s has invalid cooldown_until: %w", id, err)
			}
			cooldownUntil = parsed
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_accounts (id, provider, model, base_url, secret_ref, status, priority, concurrency_limit, daily_token_limit, cooldown_until, active_requests, daily_reserved_tokens, created_at, updated_at)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
			ON CONFLICT (id) DO UPDATE SET provider = EXCLUDED.provider, model = EXCLUDED.model, base_url = EXCLUDED.base_url, secret_ref = EXCLUDED.secret_ref, status = EXCLUDED.status, priority = EXCLUDED.priority, concurrency_limit = EXCLUDED.concurrency_limit, daily_token_limit = EXCLUDED.daily_token_limit, cooldown_until = EXCLUDED.cooldown_until, active_requests = EXCLUDED.active_requests, daily_reserved_tokens = EXCLUDED.daily_reserved_tokens, updated_at = CURRENT_TIMESTAMP
		`, id, account.Provider, account.Model, account.BaseURL, account.SecretRef, account.Status, account.Priority, account.ConcurrencyLimit, account.DailyLimit, cooldownUntil, activeLeases, dailyUsedTokens); err != nil {
			return err
		}
	}
	for id, lease := range state.ModelLeases {
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			return fmt.Errorf("model lease %s has invalid expiry: %w", id, err)
		}
		createdAt := any(s.Now())
		if lease.CreatedAt != "" {
			parsed, parseErr := time.Parse(time.RFC3339, lease.CreatedAt)
			if parseErr != nil {
				return fmt.Errorf("model lease %s has invalid creation time: %w", id, parseErr)
			}
			createdAt = parsed
		}
		releasedAt := any(nil)
		if lease.ReleasedAt != "" {
			parsed, parseErr := time.Parse(time.RFC3339, lease.ReleasedAt)
			if parseErr != nil {
				return fmt.Errorf("model lease %s has invalid release time: %w", id, parseErr)
			}
			releasedAt = parsed
		} else if lease.Status == controlplane.ModelLeaseStatusReleased || lease.Status == controlplane.ModelLeaseStatusExpired {
			releasedAt = s.Now()
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_leases (id, account_id, user_id, device_id, purpose, status, expires_at, created_at, released_at, provider, model, proxy_mode, concurrency_limit)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
			ON CONFLICT (id) DO UPDATE SET account_id = EXCLUDED.account_id, user_id = EXCLUDED.user_id, device_id = EXCLUDED.device_id, purpose = EXCLUDED.purpose, status = EXCLUDED.status, expires_at = EXCLUDED.expires_at, released_at = COALESCE(model_leases.released_at, EXCLUDED.released_at), provider = EXCLUDED.provider, model = EXCLUDED.model, proxy_mode = EXCLUDED.proxy_mode, concurrency_limit = EXCLUDED.concurrency_limit
		`, id, lease.AccountID, lease.UserID, lease.DeviceID, lease.Purpose, lease.Status, expiresAt, createdAt, releasedAt, lease.Provider, lease.Model, controlplane.ModelLeaseProxyModeDirectLease, lease.ConcurrencyLimit); err != nil {
			return err
		}
	}
	for id, usage := range state.ModelUsageRecords {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_usage_records (id, account_id, lease_id, user_id, device_id, provider, model, prompt_tokens, completion_tokens, total_tokens, latency_ms, request_id, client_call_id, usage_source, status, error_code, created_at)
			SELECT $1, l.account_id, $2, l.user_id, l.device_id, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NULLIF($13, ''), $14 FROM model_leases l WHERE l.id = $2
			ON CONFLICT (id) DO UPDATE SET lease_id = EXCLUDED.lease_id, provider = EXCLUDED.provider, model = EXCLUDED.model, prompt_tokens = EXCLUDED.prompt_tokens, completion_tokens = EXCLUDED.completion_tokens, total_tokens = EXCLUDED.total_tokens, latency_ms = EXCLUDED.latency_ms, request_id = EXCLUDED.request_id, client_call_id = EXCLUDED.client_call_id, usage_source = EXCLUDED.usage_source, status = EXCLUDED.status, error_code = EXCLUDED.error_code
		`, id, usage.LeaseID, usage.Provider, usage.Model, usage.InputTokens, usage.OutputTokens, usage.TotalTokens, usage.LatencyMS, usage.RequestID, usage.ClientCallID, usage.UsageSource, usage.Status, usage.ErrorCode, usage.CreatedAt); err != nil {
			return err
		}
	}
	for id, result := range state.ModelPoolTestResults {
		payload, err := json.Marshal(result)
		if err != nil {
			return fmt.Errorf("marshal model pool test result %s: %w", id, err)
		}
		testedAt, err := time.Parse(time.RFC3339, result.TestedAt)
		if err != nil {
			return fmt.Errorf("model pool test result %s has invalid tested_at: %w", id, err)
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO model_pool_test_results (id, account_id, payload, created_at)
			VALUES ($1, $2, $3, $4)
			ON CONFLICT (id) DO UPDATE SET account_id = EXCLUDED.account_id, payload = EXCLUDED.payload, created_at = EXCLUDED.created_at
		`, id, result.AccountID, payload, testedAt); err != nil {
			return err
		}
	}
	for scope, record := range state.IdempotencyRecords {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)
			VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
			ON CONFLICT (scope, idempotency_key) DO UPDATE SET fingerprint = EXCLUDED.fingerprint, resource_id = EXCLUDED.resource_id
		`, "control-plane-state", scope, record.Fingerprint, record.ResourceID); err != nil {
			return err
		}
	}
	for id, audit := range state.AuditLogs {
		product := audit.Product
		if product == "" {
			product = controlplane.ProductAutoLive
		}
		if !product.Valid() {
			return controlplane.ErrInvalidRequest
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO audit_logs (id, product, actor_user_id, device_id, action, resource_type, resource_id, request_id, outcome, status_code, error_code, payload, created_at)
			VALUES ($1, $2, NULLIF($3, ''), NULLIF($4, ''), $5, $6, NULLIF($7, ''), NULLIF($8, ''), $9, $10, NULLIF($11, ''), '{}'::jsonb, $12)
			ON CONFLICT (id) DO NOTHING
		`, id, product, audit.ActorUserID, audit.DeviceID, audit.Action, audit.TargetType, audit.TargetID, audit.RequestID, audit.Outcome, audit.StatusCode, audit.ErrorCode, audit.CreatedAt); err != nil {
			return err
		}
	}
	return nil
}

func activationCodeHash(state *State, codeID string) (string, bool) {
	for digest, id := range state.ActivationCodeIndex {
		if id == codeID {
			return digest, true
		}
	}
	return "", false
}

func activeLeasesForAccount(state *State, accountID string) int {
	count := 0
	for _, lease := range state.ModelLeases {
		if lease.AccountID == accountID && lease.Status == controlplane.ModelLeaseStatusActive {
			count++
		}
	}
	return count
}

func dailyUsedTokensForAccount(state *State, accountID string, now time.Time) int {
	day := now.UTC().Format("2006-01-02")
	leaseAccountByID := make(map[string]string, len(state.ModelLeases))
	for id, lease := range state.ModelLeases {
		leaseAccountByID[id] = lease.AccountID
	}
	used := 0
	for _, usage := range state.ModelUsageRecords {
		if leaseAccountByID[usage.LeaseID] != accountID {
			continue
		}
		createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
		if err != nil || createdAt.UTC().Format("2006-01-02") != day {
			continue
		}
		used += usage.TotalTokens
	}
	return used
}

func modelPoolSecretRefs(state *State) map[string]struct{} {
	refs := map[string]struct{}{}
	for _, account := range state.ModelPoolAccounts {
		if account.SecretRef != "" {
			refs[account.SecretRef] = struct{}{}
		}
	}
	return refs
}

func newModelPoolSecretRefs(before, after map[string]struct{}) []string {
	refs := make([]string, 0)
	for ref := range after {
		if _, exists := before[ref]; !exists {
			refs = append(refs, ref)
		}
	}
	return refs
}

type stateSnapshot struct {
	Version int    `json:"version"`
	State   *State `json:"state,omitempty"`
}

func marshalStateSnapshot(state *State) ([]byte, error) {
	if state == nil {
		return nil, errors.New("state must not be nil")
	}
	copyState := *state
	copyState.ModelPoolAccounts = make(map[string]controlplane.ModelPoolAccountSummary, len(state.ModelPoolAccounts))
	for id, account := range state.ModelPoolAccounts {
		account.SecretRef = ""
		copyState.ModelPoolAccounts[id] = account
	}
	copyState.ActivationCodes = make(map[string]ActivationCodeRecord, len(state.ActivationCodes))
	for id, record := range state.ActivationCodes {
		record.PlainCode = ""
		record.ActivationCode.PlainCode = nil
		copyState.ActivationCodes[id] = record
	}
	return json.Marshal(stateSnapshot{Version: 1, State: &copyState})
}

func unmarshalStateSnapshot(raw []byte) (*State, error) {
	var snapshot stateSnapshot
	if err := json.Unmarshal(raw, &snapshot); err != nil {
		return nil, fmt.Errorf("decode postgres state snapshot: %w", err)
	}
	if snapshot.Version != 1 {
		return nil, fmt.Errorf("unsupported postgres state snapshot version %d", snapshot.Version)
	}
	if snapshot.State == nil {
		return nil, errors.New("postgres state snapshot is missing state payload")
	}
	state := ensureStateMaps(snapshot.State)
	for id, account := range state.ModelPoolAccounts {
		if account.SecretConfigured {
			account.SecretRef = "model-account/" + id
			state.ModelPoolAccounts[id] = account
		}
	}
	return state, nil
}
