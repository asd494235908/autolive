package store

import (
	"context"
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
