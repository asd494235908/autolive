//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"sync"
	"testing"
	"time"
)

func TestPostgresLoginThrottleDoesNotLoseCrossInstanceFailures(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := fmt.Sprintf("%064x", now.UnixNano())
	buckets := []LoginThrottleBucket{
		{Type: LoginThrottleBucketAccount, Hash: suffix},
		{Type: LoginThrottleBucketAddress, Hash: fmt.Sprintf("%064x", now.UnixNano()+1)},
	}
	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_login_throttles WHERE bucket_hash IN ($1, $2)`, buckets[0].Hash, buckets[1].Hash)
	})
	first, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	second, err := NewPostgresRepository(database, time.Now)
	if err != nil {
		t.Fatal(err)
	}
	const attempts = 10
	errs := make(chan error, attempts)
	var group sync.WaitGroup
	for attempt := 0; attempt < attempts; attempt++ {
		group.Add(1)
		go func(repository *PostgresRepository) {
			defer group.Done()
			errs <- repository.RecordLoginFailure(ctx, buckets, now)
		}([]*PostgresRepository{first, second}[attempt%2])
	}
	group.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatalf("RecordLoginFailure() error = %v", err)
		}
	}
	for _, bucket := range buckets {
		var failureCount int
		if err := database.QueryRowContext(ctx, `SELECT failure_count FROM auth_login_throttles WHERE bucket_type = $1 AND bucket_hash = $2`, bucket.Type, bucket.Hash).Scan(&failureCount); err != nil {
			t.Fatal(err)
		}
		if failureCount != attempts {
			t.Fatalf("%s failure count = %d, want %d", bucket.Type, failureCount, attempts)
		}
	}
}
