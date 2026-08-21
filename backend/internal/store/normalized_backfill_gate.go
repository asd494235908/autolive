package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
)

// lockNormalizedControlPlaneMutation serializes normalized compatibility
// writes and lets migration 0020 reject them until the explicit backfill has
// completed. Keeping this boundary in its own file prevents user-specific
// repositories from becoming the owner of a cross-domain runtime gate.
func lockNormalizedControlPlaneMutation(ctx context.Context, tx *sql.Tx) error {
	if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0)), autolive_require_normalized_backfill_completed()`); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("lock normalized control-plane mutation: %w", err))
	}
	return nil
}

func ensureNormalizedBackfillReady(ctx context.Context, tx *sql.Tx) error {
	var status string
	if err := tx.QueryRowContext(ctx, `SELECT status FROM normalized_backfill_state WHERE id = TRUE FOR SHARE`).Scan(&status); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return errors.New("normalized backfill state is missing; run database migrations before enabling normalized reads")
		}
		return fmt.Errorf("read normalized backfill state: %w", err)
	}
	if status != "completed" {
		return fmt.Errorf("normalized backfill is not complete (status %q); run autolive-backfill-normalized before enabling normalized reads", status)
	}
	return nil
}
