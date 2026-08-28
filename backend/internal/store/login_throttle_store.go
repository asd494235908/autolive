package store

import (
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/lib/pq"
)

type LoginThrottleStore interface {
	CheckLoginThrottle(ctx context.Context, buckets []LoginThrottleBucket, now time.Time) (time.Duration, error)
	RecordLoginFailure(ctx context.Context, buckets []LoginThrottleBucket, now time.Time) error
	ResetLoginFailures(ctx context.Context, buckets []LoginThrottleBucket) error
}

type LoginThrottleRetentionCleaner interface {
	CleanupLoginThrottleBuckets(ctx context.Context, request RetentionCleanupRequest) (int64, error)
}

const (
	LoginThrottleBucketAccount = "account"
	LoginThrottleBucketAddress = "address"

	loginAccountFailureThreshold = 5
	loginAddressFailureThreshold = 50
)

type LoginThrottleBucket struct {
	Type string
	Hash string
}

var _ LoginThrottleStore = (*PostgresRepository)(nil)
var _ LoginThrottleRetentionCleaner = (*PostgresRepository)(nil)

func LoginFailureBackoff(bucketType string, failureCount int) time.Duration {
	threshold := loginAccountFailureThreshold
	if bucketType == LoginThrottleBucketAddress {
		threshold = loginAddressFailureThreshold
	}
	if failureCount < threshold {
		return 0
	}
	delay := 30 * time.Second
	for count := threshold; count < failureCount && delay < 15*time.Minute; count++ {
		delay *= 2
	}
	if delay > 15*time.Minute {
		return 15 * time.Minute
	}
	return delay
}

func (s *PostgresRepository) CheckLoginThrottle(ctx context.Context, buckets []LoginThrottleBucket, now time.Time) (time.Duration, error) {
	buckets, err := validLoginThrottleBuckets(buckets)
	if err != nil {
		return 0, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	var blockedUntil sql.NullTime
	err = s.db.QueryRowContext(operationCtx, `
		SELECT MAX(blocked_until)
		FROM auth_login_throttles
		WHERE (bucket_type = $1 AND bucket_hash = $2)
		   OR (bucket_type = $3 AND bucket_hash = $4)
	`, buckets[0].Type, buckets[0].Hash, buckets[len(buckets)-1].Type, buckets[len(buckets)-1].Hash).Scan(&blockedUntil)
	if err != nil {
		return 0, postgresOperationError(operationCtx, fmt.Errorf("check login throttle: %w", err))
	}
	if !blockedUntil.Valid || !blockedUntil.Time.After(now) {
		return 0, nil
	}
	return blockedUntil.Time.Sub(now), nil
}

func (s *PostgresRepository) RecordLoginFailure(ctx context.Context, buckets []LoginThrottleBucket, now time.Time) error {
	buckets, err := validLoginThrottleBuckets(buckets)
	if err != nil {
		return err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("begin login throttle update: %w", err))
	}
	defer func() { _ = tx.Rollback() }()
	for _, bucket := range buckets {
		var failureCount int
		if err := tx.QueryRowContext(operationCtx, `
			INSERT INTO auth_login_throttles (bucket_type, bucket_hash, failure_count, blocked_until, last_failed_at, updated_at)
			VALUES ($1, $2, 1, $3, $3, $3)
			ON CONFLICT (bucket_type, bucket_hash) DO UPDATE SET
				failure_count = CASE
					WHEN auth_login_throttles.last_failed_at < $3 - INTERVAL '15 minutes' THEN 1
					ELSE auth_login_throttles.failure_count + 1
				END,
				last_failed_at = $3,
				updated_at = $3
			RETURNING failure_count
		`, bucket.Type, bucket.Hash, now).Scan(&failureCount); err != nil {
			return postgresOperationError(operationCtx, fmt.Errorf("increment login throttle: %w", err))
		}
		blockedUntil := now.Add(LoginFailureBackoff(bucket.Type, failureCount))
		if _, err := tx.ExecContext(operationCtx, `UPDATE auth_login_throttles SET blocked_until = $3 WHERE bucket_type = $1 AND bucket_hash = $2`, bucket.Type, bucket.Hash, blockedUntil); err != nil {
			return postgresOperationError(operationCtx, fmt.Errorf("persist login throttle: %w", err))
		}
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit login throttle update", err)
	}
	return nil
}

func (s *PostgresRepository) CleanupLoginThrottleBuckets(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := request.Validate(); err != nil {
		return 0, err
	}
	return cleanupPostgresRows(ctx, s.db, s.operationContext, `
		WITH candidates AS (
			SELECT bucket_type, bucket_hash
			FROM auth_login_throttles
			WHERE updated_at < $1
			ORDER BY updated_at, bucket_type, bucket_hash
			LIMIT $2
			FOR UPDATE SKIP LOCKED
		)
		DELETE FROM auth_login_throttles AS target
		USING candidates
		WHERE target.bucket_type = candidates.bucket_type
		  AND target.bucket_hash = candidates.bucket_hash
	`, request)
}

func (s *PostgresRepository) ResetLoginFailures(ctx context.Context, buckets []LoginThrottleBucket) error {
	buckets, err := validLoginThrottleBuckets(buckets)
	if err != nil {
		return err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	types := make([]string, len(buckets))
	hashes := make([]string, len(buckets))
	for index, bucket := range buckets {
		types[index], hashes[index] = bucket.Type, bucket.Hash
	}
	if _, err := s.db.ExecContext(operationCtx, `DELETE FROM auth_login_throttles WHERE (bucket_type, bucket_hash) IN (SELECT * FROM unnest($1::text[], $2::text[]))`, pq.Array(types), pq.Array(hashes)); err != nil {
		return postgresOperationError(operationCtx, fmt.Errorf("reset login throttle: %w", err))
	}
	return nil
}

func validLoginThrottleBuckets(buckets []LoginThrottleBucket) ([]LoginThrottleBucket, error) {
	if len(buckets) == 0 || len(buckets) > 2 {
		return nil, errors.New("one or two login throttle buckets are required")
	}
	result := make([]LoginThrottleBucket, 0, len(buckets))
	seen := make(map[string]struct{}, len(buckets))
	for _, bucket := range buckets {
		bucket.Type = strings.TrimSpace(bucket.Type)
		bucket.Hash = strings.TrimSpace(bucket.Hash)
		_, hashErr := hex.DecodeString(bucket.Hash)
		if (bucket.Type != LoginThrottleBucketAccount && bucket.Type != LoginThrottleBucketAddress) || len(bucket.Hash) != 64 || hashErr != nil {
			return nil, errors.New("login throttle digest must be a SHA-256 hex value")
		}
		key := bucket.Type + ":" + bucket.Hash
		if _, exists := seen[key]; exists {
			continue
		}
		seen[key] = struct{}{}
		result = append(result, bucket)
	}
	sort.Slice(result, func(i, j int) bool {
		if result[i].Type == result[j].Type {
			return result[i].Hash < result[j].Hash
		}
		return result[i].Type < result[j].Type
	})
	return result, nil
}
