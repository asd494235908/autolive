package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryNormalizedOperationalMetricsUseAggregateQueries(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(
		database,
		func() time.Time { return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC) },
		nil,
		ModelReadSourceNormalized,
	)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0))")).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT status, COUNT(*) FROM model_accounts GROUP BY status")).
		WillReturnRows(sqlmock.NewRows([]string{"status", "count"}).
			AddRow("active", int64(2)).
			AddRow("cooldown", int64(1)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM model_leases WHERE status = 'active' AND expires_at > $1")).
		WithArgs(sqlmock.AnyArg()).
		WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(int64(3)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COALESCE(SUM(total_tokens), 0) FROM model_usage_records WHERE created_at >= $1 AND created_at < $2")).
		WithArgs(sqlmock.AnyArg(), sqlmock.AnyArg()).
		WillReturnRows(sqlmock.NewRows([]string{"tokens"}).AddRow(int64(42)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM user_authorization_policies WHERE daily_token_limit > 0 OR allowed_models <> '[]'::jsonb")).
		WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(int64(1)))
	mock.ExpectRollback()

	metrics, err := repository.ReadOperationalMetrics(context.Background())
	if err != nil {
		t.Fatalf("ReadOperationalMetrics() error = %v", err)
	}
	if metrics.ModelAccountStatusCounts["active"] != 2 || metrics.ModelAccountStatusCounts["cooldown"] != 1 ||
		metrics.ActiveModelLeases != 3 || metrics.DailyModelUsageTokens != 42 ||
		metrics.ConfiguredUserAuthorizationPolicies != 1 {
		t.Fatalf("operational metrics = %+v", metrics)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
