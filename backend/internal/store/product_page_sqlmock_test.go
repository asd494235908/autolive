package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryProductScopedPagesKeepProductInCountAndListPredicates(t *testing.T) {
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	product := string(controlplane.ProductDouyinDesktop)

	t.Run("activation", func(t *testing.T) {
		database, mock, err := sqlmock.New()
		if err != nil {
			t.Fatalf("sqlmock.New() error = %v", err)
		}
		defer database.Close()
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("constructor error = %v", err)
		}

		mock.ExpectBegin()
		expectNormalizedPageCoverage(mock)
		mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM activation_codes WHERE product = $1")).WithArgs(product).
			WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
		mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, code_prefix")).WithArgs(controlplane.ActivationCodeStatusActive, now, controlplane.ActivationCodeStatusExpired, product, 20, 0).
			WillReturnRows(sqlmock.NewRows([]string{"id", "product", "code_prefix", "status", "expires_at", "used_at", "used_by_user_id", "used_by_device_id", "max_devices", "bound_devices"}).
				AddRow("ac_douyin", product, "code_douyin", controlplane.ActivationCodeStatusActive, now.Add(time.Hour), nil, nil, nil, 1, 0))
		mock.ExpectCommit()

		page, err := repository.ListActivationCodesPageForProduct(context.Background(), 0, 20, controlplane.ProductDouyinDesktop)
		if err != nil || page.Total != 1 || len(page.Items) != 1 || page.Items[0].Product != controlplane.ProductDouyinDesktop {
			t.Fatalf("activation page = (%+v, %v)", page, err)
		}
		if err := mock.ExpectationsWereMet(); err != nil {
			t.Fatalf("sql expectations: %v", err)
		}
	})

	t.Run("usage", func(t *testing.T) {
		database, mock, err := sqlmock.New()
		if err != nil {
			t.Fatalf("sqlmock.New() error = %v", err)
		}
		defer database.Close()
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("constructor error = %v", err)
		}

		mock.ExpectBegin()
		expectNormalizedPageCoverage(mock)
		mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_usage_records WHERE product = $1")).WithArgs(product).
			WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
		mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, lease_id, client_call_id, request_id, provider, model")).WithArgs(product, 20, 0).
			WillReturnRows(sqlmock.NewRows([]string{"id", "product", "lease_id", "client_call_id", "request_id", "provider", "model", "prompt_tokens", "completion_tokens", "total_tokens", "latency_ms", "status", "usage_source", "error_code", "created_at"}).
				AddRow("usage_douyin", product, "lease_douyin", "call_douyin", "req_douyin", "openai", "rewrite", 1, 2, 3, int64(10), "succeeded", "client_reported", nil, now))
		mock.ExpectCommit()

		page, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
		if err != nil || page.Total != 1 || len(page.Items) != 1 || page.Items[0].Product != controlplane.ProductDouyinDesktop {
			t.Fatalf("usage page = (%+v, %v)", page, err)
		}
		if err := mock.ExpectationsWereMet(); err != nil {
			t.Fatalf("sql expectations: %v", err)
		}
	})

	t.Run("lease", func(t *testing.T) {
		database, mock, err := sqlmock.New()
		if err != nil {
			t.Fatalf("sqlmock.New() error = %v", err)
		}
		defer database.Close()
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("constructor error = %v", err)
		}

		mock.ExpectBegin()
		expectNormalizedPageCoverage(mock)
		mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases WHERE product = $1")).WithArgs(product).
			WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
		mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, account_id, user_id, device_id, purpose, status, expires_at")).WithArgs(product, 20, 0).
			WillReturnRows(sqlmock.NewRows([]string{"id", "product", "account_id", "user_id", "device_id", "purpose", "status", "expires_at", "provider", "model", "proxy_mode", "concurrency_limit"}).
				AddRow("lease_douyin", product, "account_douyin", "user_douyin", "device_douyin", "client", "active", now.Add(time.Hour), "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 1))
		mock.ExpectCommit()

		page, err := repository.ListModelLeasesPageWithOptions(context.Background(), ModelLeasePageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
		if err != nil || page.Total != 1 || len(page.Items) != 1 || page.Items[0].Product != controlplane.ProductDouyinDesktop {
			t.Fatalf("lease page = (%+v, %v)", page, err)
		}
		if err := mock.ExpectationsWereMet(); err != nil {
			t.Fatalf("sql expectations: %v", err)
		}
	})

	t.Run("audit", func(t *testing.T) {
		database, mock, err := sqlmock.New()
		if err != nil {
			t.Fatalf("sqlmock.New() error = %v", err)
		}
		defer database.Close()
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("constructor error = %v", err)
		}

		mock.ExpectBegin()
		expectNormalizedPageCoverage(mock)
		mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM audit_logs WHERE product = $1")).WithArgs(product).
			WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
		mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, actor_user_id, device_id, action, resource_type")).WithArgs(product, 20, 0).
			WillReturnRows(sqlmock.NewRows([]string{"id", "product", "actor_user_id", "device_id", "action", "resource_type", "resource_id", "request_id", "outcome", "status_code", "error_code", "created_at"}).
				AddRow("audit_douyin", product, "user_douyin", "device_douyin", "seed", "user", "user_douyin", "request_douyin", "success", 200, nil, now))
		mock.ExpectCommit()

		page, err := repository.ListAuditLogsPageWithOptions(context.Background(), AuditLogPageOptions{Offset: 0, Limit: 20, Product: controlplane.ProductDouyinDesktop})
		if err != nil || page.Total != 1 || len(page.Items) != 1 || page.Items[0].Product != controlplane.ProductDouyinDesktop {
			t.Fatalf("audit page = (%+v, %v)", page, err)
		}
		if err := mock.ExpectationsWereMet(); err != nil {
			t.Fatalf("sql expectations: %v", err)
		}
	})
}
