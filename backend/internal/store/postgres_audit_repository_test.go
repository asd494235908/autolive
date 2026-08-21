package store

import (
	"context"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryRecordAuditUsesNormalizedAppend(t *testing.T) {
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
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO audit_logs (")).WithArgs(sqlmock.AnyArg(), "douyin_desktop", "usr_1", "dev_1", "model.lease.release", "model_lease", "lease_1", "req-1", "success", 200, "", now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	err = repository.RecordAudit(context.Background(), controlplane.AuditLogInput{ActorUserID: "usr_1", Product: controlplane.ProductDouyinDesktop, DeviceID: "dev_1", Action: "model.lease.release", TargetType: "model_lease", TargetID: "lease_1", RequestID: "req-1", Outcome: "success", StatusCode: 200})
	if err != nil {
		t.Fatalf("RecordAudit() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryRecordAuditHonorsCancellation(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	err = repository.RecordAudit(ctx, controlplane.AuditLogInput{Action: "test", TargetType: "test", Outcome: "success", StatusCode: 200})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("RecordAudit() error = %v, want context.Canceled", err)
	}
}
