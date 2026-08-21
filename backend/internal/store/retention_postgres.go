package store

import (
	"context"
	"database/sql"
)

const cleanupAuthSessionsSQL = `
	WITH candidates AS (
		SELECT id
		FROM auth_sessions
		WHERE refresh_expires_at < $1 OR revoked_at < $1
		ORDER BY LEAST(refresh_expires_at, COALESCE(revoked_at, refresh_expires_at)), id
		LIMIT $2
		FOR UPDATE SKIP LOCKED
	)
	DELETE FROM auth_sessions AS target
	USING candidates
	WHERE target.id = candidates.id
`

// The binding timestamp is written by UpdateDeviceID before activation or
// heartbeat commits. A short caller-selected grace window lets an in-flight
// request finish while making process-crash compensation recoverable.
const cleanupOrphanedDeviceBindingsSQL = `
	WITH candidates AS (
		SELECT s.id
		FROM auth_sessions AS s
		LEFT JOIN devices AS d
		  ON d.id = s.device_id
		 AND d.user_id = s.user_id
		 AND d.status = 'active'
		WHERE s.revoked_at IS NULL
		  AND s.device_id IS NOT NULL
		  AND s.device_bound_at < $1
		  AND d.id IS NULL
		ORDER BY s.device_bound_at, s.id
		LIMIT $2
		FOR UPDATE OF s SKIP LOCKED
	)
	UPDATE auth_sessions AS target
	SET device_id = NULL, device_bound_at = NULL
	FROM candidates
	WHERE target.id = candidates.id
`

const cleanupIdempotencyRecordsSQL = `
	WITH candidates AS (
		SELECT scope, idempotency_key
		FROM idempotency_records
		WHERE scope = 'control-plane-state' AND created_at < $1
		ORDER BY created_at, idempotency_key
		LIMIT $2
		FOR UPDATE SKIP LOCKED
	)
	DELETE FROM idempotency_records AS target
	USING candidates
	WHERE target.scope = candidates.scope
	  AND target.idempotency_key = candidates.idempotency_key
	RETURNING target.idempotency_key
`

const cleanupModelPoolTestResultsSQL = `
	WITH candidates AS (
		SELECT id
		FROM model_pool_test_results
		WHERE created_at < $1
		ORDER BY created_at, id
		LIMIT $2
		FOR UPDATE SKIP LOCKED
	)
	DELETE FROM model_pool_test_results AS target
	USING candidates
	WHERE target.id = candidates.id
	RETURNING target.id
`

const cleanupAuditLogsSQL = `
	WITH candidates AS (
		SELECT id
		FROM audit_logs
		WHERE created_at < $1
		ORDER BY created_at, id
		LIMIT $2
		FOR UPDATE SKIP LOCKED
	)
	DELETE FROM audit_logs AS target
	USING candidates
	WHERE target.id = candidates.id
	RETURNING target.id
`

func (s *SQLSessionStore) CleanupAuthSessions(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	return cleanupPostgresRows(ctx, s.db, s.operationContext, cleanupAuthSessionsSQL, request)
}

func (s *SQLSessionStore) CleanupOrphanedDeviceBindings(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	return cleanupPostgresRows(ctx, s.db, s.operationContext, cleanupOrphanedDeviceBindingsSQL, request)
}

func (s *PostgresRepository) CleanupIdempotencyRecords(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	return s.cleanupNormalizedRetention(ctx, request, cleanupIdempotencyRecordsSQL)
}

func (s *PostgresRepository) CleanupModelPoolTestResults(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	return s.cleanupNormalizedRetention(ctx, request, cleanupModelPoolTestResultsSQL)
}

func (s *PostgresRepository) CleanupAuditLogs(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	return s.cleanupNormalizedRetention(ctx, request, cleanupAuditLogsSQL)
}

func (s *PostgresRepository) cleanupNormalizedRetention(
	ctx context.Context,
	request RetentionCleanupRequest,
	query string,
) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	if s.modelReadSource != ModelReadSourceNormalized {
		return 0, ErrNormalizedRetentionCleanupRequired
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(operationCtx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()`); err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	rows, err := tx.QueryContext(operationCtx, query, request.Cutoff.UTC(), request.BatchSize)
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	var deleted int64
	for rows.Next() {
		var key string
		if err := rows.Scan(&key); err != nil {
			_ = rows.Close()
			return 0, postgresOperationError(operationCtx, err)
		}
		deleted++
	}
	if err := rows.Err(); err != nil {
		_ = rows.Close()
		return 0, postgresOperationError(operationCtx, err)
	}
	if err := rows.Close(); err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	if err := tx.Commit(); err != nil {
		return 0, postgresCommitError(operationCtx, "commit postgres retention cleanup", err)
	}
	return deleted, nil
}

func cleanupPostgresRows(
	ctx context.Context,
	db *sql.DB,
	operationContext func(context.Context) (context.Context, context.CancelFunc),
	query string,
	request RetentionCleanupRequest,
) (int64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	operationCtx, cancel := operationContext(ctx)
	defer cancel()
	result, err := db.ExecContext(operationCtx, query, request.Cutoff.UTC(), request.BatchSize)
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	deleted, err := result.RowsAffected()
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	return deleted, nil
}
