package store

import (
	"context"
	"fmt"
)

// The candidate set and the active-reference check are evaluated inside the
// same transaction that deletes the rows. The normalized advisory lock is
// shared with secret rotation, so a rotation cannot switch an active reference
// between the NOT EXISTS check and the delete.
const cleanupUnreferencedStagedSecretsSQL = `
WITH candidates AS (
	SELECT s.secret_ref
	FROM model_account_secrets AS s
	WHERE s.secret_ref LIKE 'model-account/%/rotation_%'
	  AND s.updated_at < $1
	  AND NOT EXISTS (
		SELECT 1
		FROM model_accounts AS a
		WHERE a.secret_ref = s.secret_ref
	  )
	ORDER BY s.updated_at, s.secret_ref
	LIMIT $2
	FOR UPDATE OF s SKIP LOCKED
)
DELETE FROM model_account_secrets AS target
USING candidates
WHERE target.secret_ref = candidates.secret_ref
`

// CleanupUnreferencedStagedSecrets is the normalized recovery path for
// process-crash and commit-unknown candidates. It intentionally bypasses the
// compatibility StateOperation, whose full-table materialization and sync
// writes are not appropriate for a retention pass.
func (s *PostgresRepository) CleanupUnreferencedStagedSecrets(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := request.Validate(); err != nil {
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
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return 0, err
	}
	result, err := tx.ExecContext(operationCtx, cleanupUnreferencedStagedSecretsSQL, request.Cutoff.UTC(), request.BatchSize)
	if err != nil {
		return 0, postgresOperationError(operationCtx, fmt.Errorf("cleanup unreferenced staged secrets: %w", err))
	}
	deleted, err := result.RowsAffected()
	if err != nil {
		return 0, postgresOperationError(operationCtx, fmt.Errorf("count unreferenced staged secrets: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return 0, postgresCommitError(operationCtx, "commit unreferenced staged secret cleanup", err)
	}
	return deleted, nil
}
