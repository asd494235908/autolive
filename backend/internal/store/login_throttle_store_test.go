package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
)

func TestLoginFailureBackoffIsBounded(t *testing.T) {
	if got := LoginFailureBackoff(0); got != 0 {
		t.Fatalf("LoginFailureBackoff(0) = %v", got)
	}
	if got := LoginFailureBackoff(5); got != 30*time.Second {
		t.Fatalf("LoginFailureBackoff(5) = %v, want 30s", got)
	}
	if got := LoginFailureBackoff(100); got != 15*time.Minute {
		t.Fatalf("LoginFailureBackoff(100) = %v, want 15m", got)
	}
}

func TestRecordLoginFailureUsesAtomicUpsertForFirstConcurrentFailure(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 8, 26, 12, 0, 0, 0, time.UTC)
	account := LoginThrottleBucket{Type: LoginThrottleBucketAccount, Hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
	address := LoginThrottleBucket{Type: LoginThrottleBucketAddress, Hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
	atomicIncrement := "(?s)" + regexp.QuoteMeta("ON CONFLICT (bucket_type, bucket_hash) DO UPDATE SET") + ".*" + regexp.QuoteMeta("auth_login_throttles.failure_count + 1")
	mock.ExpectBegin()
	for _, bucket := range []LoginThrottleBucket{account, address} {
		mock.ExpectQuery(atomicIncrement).WithArgs(bucket.Type, bucket.Hash, now).WillReturnRows(sqlmock.NewRows([]string{"failure_count"}).AddRow(1))
		mock.ExpectExec(regexp.QuoteMeta("UPDATE auth_login_throttles SET blocked_until = $3 WHERE bucket_type = $1 AND bucket_hash = $2")).WithArgs(bucket.Type, bucket.Hash, now).WillReturnResult(sqlmock.NewResult(0, 1))
	}
	mock.ExpectCommit()
	if err := repository.RecordLoginFailure(context.Background(), []LoginThrottleBucket{address, account}, now); err != nil {
		t.Fatalf("RecordLoginFailure() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestCleanupLoginThrottleBucketsIsBoundedAndDeterministic(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	repository, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	request := RetentionCleanupRequest{Cutoff: time.Date(2026, 8, 25, 12, 0, 0, 0, time.UTC), BatchSize: 1000}
	mock.ExpectExec("(?s)"+regexp.QuoteMeta("ORDER BY updated_at, bucket_type, bucket_hash")+".*"+regexp.QuoteMeta("LIMIT $2")+".*"+regexp.QuoteMeta("DELETE FROM auth_login_throttles AS target")).WithArgs(request.Cutoff, request.BatchSize).WillReturnResult(sqlmock.NewResult(0, 7))
	deleted, err := repository.CleanupLoginThrottleBuckets(context.Background(), request)
	if err != nil || deleted != 7 {
		t.Fatalf("CleanupLoginThrottleBuckets() = %d, %v", deleted, err)
	}
	if _, err := repository.CleanupLoginThrottleBuckets(context.Background(), RetentionCleanupRequest{Cutoff: request.Cutoff, BatchSize: 1001}); err == nil {
		t.Fatal("oversized cleanup batch accepted")
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
