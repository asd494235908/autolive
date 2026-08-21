package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestMemoryModelUsagePageWithOptionsFiltersSortsAndPaginates(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := NewMemoryStore(func() time.Time { return now })
	if err := repository.Run(context.Background(), func(state *State) error {
		state.ModelLeases["lease-a"] = controlplane.ModelLease{ID: "lease-a", UserID: "user-a", DeviceID: "device-a"}
		state.ModelLeases["lease-b"] = controlplane.ModelLease{ID: "lease-b", UserID: "user-b", DeviceID: "device-b"}
		state.ModelUsageRecords["usage-new"] = controlplane.ModelUsageRecord{ID: "usage-new", LeaseID: "lease-a", Provider: "openai", Model: "rewrite", RequestID: "request-a", CreatedAt: "2026-08-21T10:00:00Z"}
		state.ModelUsageRecords["usage-old"] = controlplane.ModelUsageRecord{ID: "usage-old", LeaseID: "lease-a", Provider: "openai", Model: "rewrite", RequestID: "request-b", CreatedAt: "2026-08-20T10:00:00Z"}
		state.ModelUsageRecords["usage-other"] = controlplane.ModelUsageRecord{ID: "usage-other", LeaseID: "lease-b", Provider: "other", Model: "chat", RequestID: "request-c", CreatedAt: "2026-08-21T11:00:00Z"}
		return nil
	}); err != nil {
		t.Fatalf("seed usage state: %v", err)
	}
	createdAfter := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	page, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{
		Offset: 0, Limit: 1, Provider: " openai ", Model: "rewrite", UserID: "user-a", DeviceID: "device-a",
		CreatedAfter: &createdAfter, Sort: ModelUsageSortCreatedAsc,
	})
	if err != nil {
		t.Fatalf("ListModelUsagePageWithOptions() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].ID != "usage-new" {
		t.Fatalf("filtered usage page = %+v", page)
	}

	if _, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{Offset: 0, Limit: 1, Sort: "created_at desc"}); err == nil {
		t.Fatal("invalid usage sort unexpectedly accepted")
	}
}

func TestPostgresModelUsagePageWithOptionsBindsFiltersAndTimeRange(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	after := time.Date(2026, 8, 20, 0, 0, 0, 0, time.UTC)
	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_usage_records WHERE provider = $1 AND user_id = $2 AND created_at >= $3")).
		WithArgs("openai", "user-a", after).
		WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, product, lease_id, client_call_id, request_id, provider, model")).
		WithArgs("openai", "user-a", after, 20, 0).
		WillReturnRows(sqlmock.NewRows([]string{
			"id", "product", "lease_id", "client_call_id", "request_id", "provider", "model", "prompt_tokens", "completion_tokens", "total_tokens", "latency_ms", "status", "usage_source", "error_code", "created_at",
		}).AddRow("usage-a", string(controlplane.ProductAutoLive), "lease-a", "call-a", "request-a", "openai", "rewrite", 1, 2, 3, int64(10), "succeeded", "client_reported", nil, after))
	mock.ExpectCommit()

	page, err := repository.ListModelUsagePageWithOptions(context.Background(), ModelUsagePageOptions{
		Offset: 0, Limit: 20, Provider: "openai", UserID: "user-a", CreatedAfter: &after,
	})
	if err != nil {
		t.Fatalf("ListModelUsagePageWithOptions() error = %v", err)
	}
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].RequestID != "request-a" {
		t.Fatalf("usage page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
