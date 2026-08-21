package store

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryRecordAuditWithOutboxDeliversOnce(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 18, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-1", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, payload, product\n\t\tFROM audit_outbox")).WithArgs(now, now.Add(-auditOutboxProcessingGrace), 1).
		WillReturnRows(sqlmock.NewRows([]string{"id", "payload", "product"}).AddRow("audit_outbox_1", []byte(`{"Action":"model.lease.release","TargetType":"model_lease","RequestID":"req-1","Outcome":"success","StatusCode":200}`), nil))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE audit_outbox SET status = 'processing'")).WithArgs("audit_outbox_1", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_logs (")).WithArgs("audit_1", controlplane.ProductAutoLive, "", "", "model.lease.release", "model_lease", "", "req-1", "success", 200, "", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE audit_outbox\n\t\tSET status = 'sent'")).WithArgs("audit_outbox_1", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	err = repository.RecordAuditWithOutbox(context.Background(), controlplane.AuditLogInput{Action: "model.lease.release", TargetType: "model_lease", RequestID: "req-1", Outcome: "success", StatusCode: 200})
	if err != nil {
		t.Fatalf("RecordAuditWithOutbox() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRecordAuditWithOutboxMarksDeliveryFailureRetryable(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 18, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).WithArgs(sqlmock.AnyArg(), controlplane.ProductAutoLive, "audit-request:req-2", sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, payload, product\n\t\tFROM audit_outbox")).WithArgs(now, now.Add(-auditOutboxProcessingGrace), 1).
		WillReturnRows(sqlmock.NewRows([]string{"id", "payload", "product"}).AddRow("audit_outbox_2", []byte(`{"Action":"test","TargetType":"test","RequestID":"req-2","Outcome":"success","StatusCode":200}`), nil))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE audit_outbox SET status = 'processing'")).WithArgs("audit_outbox_2", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_logs (")).WithArgs("audit_2", controlplane.ProductAutoLive, "", "", "test", "test", "", "req-2", "success", 200, "", now).WillReturnError(errors.New("audit table unavailable"))
	mock.ExpectRollback()
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("UPDATE audit_outbox\n\t\tSET status = 'pending'")).WithArgs("audit_outbox_2", now.Add(time.Minute), sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	err = repository.RecordAuditWithOutbox(context.Background(), controlplane.AuditLogInput{Action: "test", TargetType: "test", RequestID: "req-2", Outcome: "success", StatusCode: 200})
	if err == nil || !strings.Contains(err.Error(), "audit table unavailable") {
		t.Fatalf("RecordAuditWithOutbox() error = %v, want delivery error", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresEnqueueAuditOutboxValidatesModelAccountProduct(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 22, 18, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND product = $2")).
		WithArgs("account_douyin", controlplane.ProductDouyinDesktop).
		WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).
		WithArgs(sqlmock.AnyArg(), controlplane.ProductDouyinDesktop, "audit-request:model-account", sqlmock.AnyArg(), now).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatalf("BeginTx() error = %v", err)
	}
	err = repository.enqueueAuditOutboxTx(context.Background(), tx, controlplane.AuditLogInput{
		Product: controlplane.ProductDouyinDesktop, Action: "PATCH /api/v1/admin/model-pool/account_douyin",
		TargetType: "model_account", TargetID: "account_douyin", RequestID: "model-account", Outcome: "success", StatusCode: 200,
	}, now)
	if err != nil {
		t.Fatalf("enqueueAuditOutboxTx() error = %v", err)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("Commit() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRecordAuditWithOutboxRejectsMissingSuccessTargetButAllowsFailureTarget(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	input := controlplane.AuditLogInput{
		Product: controlplane.ProductDouyinDesktop, Action: "POST /api/v1/admin/model-pool",
		TargetType: "model_account", TargetID: "account-missing", StatusCode: 201,
	}

	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND product = $2")).
		WithArgs("account-missing", controlplane.ProductDouyinDesktop).
		WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND (product IS NULL OR product <> $2)")).
		WithArgs("account-missing", controlplane.ProductDouyinDesktop).
		WillReturnError(sql.ErrNoRows)
	mock.ExpectRollback()
	if err := repository.RecordAuditWithOutbox(context.Background(), input); !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("RecordAuditWithOutbox(success) error = %v, want forbidden", err)
	}

	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND product = $2")).
		WithArgs("account-missing", controlplane.ProductDouyinDesktop).
		WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND (product IS NULL OR product <> $2)")).
		WithArgs("account-missing", controlplane.ProductDouyinDesktop).
		WillReturnError(sql.ErrNoRows)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_outbox (")).
		WithArgs(sqlmock.AnyArg(), controlplane.ProductDouyinDesktop, sqlmock.AnyArg(), sqlmock.AnyArg(), sqlmock.AnyArg()).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, payload, product\n\t\tFROM audit_outbox")).
		WithArgs(sqlmock.AnyArg(), sqlmock.AnyArg(), 1).
		WillReturnRows(sqlmock.NewRows([]string{"id", "payload", "product"}))
	mock.ExpectCommit()
	if err := repository.RecordAuditWithOutbox(context.Background(), controlplane.AuditLogInput{
		Product: controlplane.ProductDouyinDesktop, Action: input.Action, TargetType: input.TargetType,
		TargetID: input.TargetID, Outcome: "failure", StatusCode: 404,
	}); err != nil {
		t.Fatalf("RecordAuditWithOutbox(failure) error = %v, want nil", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRecordAuditRejectsModelAccountProductConflicts(t *testing.T) {
	cases := []struct {
		name string
		id   string
	}{
		{name: "cross product", id: "account-other-product"},
		{name: "null product", id: "account-null-product"},
		{name: "invalid product", id: "account-invalid-product"},
	}
	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			database, mock, err := sqlmock.New()
			if err != nil {
				t.Fatalf("sqlmock.New() error = %v", err)
			}
			defer database.Close()
			repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
			if err != nil {
				t.Fatalf("constructor error = %v", err)
			}
			mock.ExpectBegin()
			mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND product = $2")).
				WithArgs(tt.id, controlplane.ProductDouyinDesktop).
				WillReturnError(sql.ErrNoRows)
			mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM model_accounts WHERE id = $1 AND (product IS NULL OR product <> $2)")).
				WithArgs(tt.id, controlplane.ProductDouyinDesktop).
				WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(1))
			mock.ExpectRollback()

			err = repository.RecordAudit(context.Background(), controlplane.AuditLogInput{
				Product: controlplane.ProductDouyinDesktop, Action: "PATCH /api/v1/admin/model-pool", TargetType: "model_account", TargetID: tt.id, Outcome: "success", StatusCode: 200,
			})
			if !errors.Is(err, controlplane.ErrForbidden) {
				t.Fatalf("RecordAudit() error = %v, want forbidden", err)
			}
			if err := mock.ExpectationsWereMet(); err != nil {
				t.Fatalf("sql expectations: %v", err)
			}
		})
	}
}

func TestPostgresRepositoryDispatchAuditOutboxRejectsInvalidBatchAndCancellation(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	if _, err := repository.DispatchAuditOutbox(context.Background(), 0); err == nil {
		t.Fatal("DispatchAuditOutbox(batch=0) error = nil")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := repository.DispatchAuditOutbox(ctx, 1); !errors.Is(err, context.Canceled) {
		t.Fatalf("DispatchAuditOutbox(cancelled) error = %v, want context canceled", err)
	}
}

func expectNormalizedAuditTargetProduct(mock sqlmock.Sqlmock, table string, id any, product controlplane.ProductCode) {
	mock.ExpectQuery(regexp.QuoteMeta("SELECT 1 FROM "+table+" WHERE id = $1 AND product = $2")).
		WithArgs(id, product).
		WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(1))
}
