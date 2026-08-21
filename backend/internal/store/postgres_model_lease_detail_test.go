package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryNormalizedModelLeaseDetailUsesBoundedQuery(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	createdAt := now.Add(-time.Minute)
	expiresAt := now.Add(time.Hour)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, created_at,")).
		WithArgs("lease-detail-1").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "account_id", "user_id", "device_id", "purpose", "status", "created_at", "expires_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit",
		}).AddRow("lease-detail-1", "account-1", "user-1", "device-1", "validation", "active", createdAt, expiresAt, nil, "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectCommit()

	detail, err := repository.GetModelLeaseAdminDetail(context.Background(), " lease-detail-1 ")
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetail() error = %v", err)
	}
	if detail.ID != "lease-detail-1" || detail.CreatedAt != createdAt.Format(time.RFC3339) || detail.ExpiresAt != expiresAt.Format(time.RFC3339) || detail.Status != controlplane.ModelLeaseStatusActive {
		t.Fatalf("unexpected lease detail = %+v", detail)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryNormalizedModelLeaseDetailDerivesExpiryWithoutSnapshotWrite(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	expiresAt := now.Add(-time.Minute)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, account_id, user_id, device_id, purpose, status, created_at,")).
		WithArgs("lease-expired-1").
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "account_id", "user_id", "device_id", "purpose", "status", "created_at", "expires_at", "released_at", "provider", "model", "proxy_mode", "concurrency_limit",
		}).AddRow("lease-expired-1", "account-1", "user-1", "device-1", "validation", "active", now.Add(-time.Hour), expiresAt, nil, "openai", "rewrite", controlplane.ModelLeaseProxyModeDirectLease, 2))
	mock.ExpectCommit()

	detail, err := repository.GetModelLeaseAdminDetail(context.Background(), "lease-expired-1")
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetail() error = %v", err)
	}
	if detail.Status != controlplane.ModelLeaseStatusExpired || detail.ReleasedAt != now.Format(time.RFC3339) {
		t.Fatalf("expired lease detail = %+v", detail)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

var _ ModelLeaseDetailReader = (*PostgresRepository)(nil)
