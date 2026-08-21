package httpapi

import (
	"context"
	"errors"
	"slices"

	"autoLive/backend/internal/store"
)

var _ store.AuthSessionRetentionCleaner = (*authenticator)(nil)
var _ store.AuthSessionBindingCleaner = (*authenticator)(nil)

func (a *authenticator) CleanupAuthSessions(ctx context.Context, request store.RetentionCleanupRequest) (int64, error) {
	if err := request.Validate(); err != nil {
		return 0, err
	}
	if a.store != nil {
		cleaner, ok := a.store.(store.AuthSessionRetentionCleaner)
		if !ok {
			return 0, errors.New("configured auth session store does not support retention cleanup")
		}
		return cleaner.CleanupAuthSessions(ctx, request)
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return 0, err
	}

	candidates := make([]sessionRecord, 0, len(a.sessions))
	for _, session := range a.sessions {
		if err := ctx.Err(); err != nil {
			return 0, err
		}
		if session.RefreshExpiresAt.Before(request.Cutoff) {
			candidates = append(candidates, session)
		}
	}
	slices.SortFunc(candidates, func(left, right sessionRecord) int {
		if order := left.RefreshExpiresAt.Compare(right.RefreshExpiresAt); order != 0 {
			return order
		}
		if left.AccessTokenHash < right.AccessTokenHash {
			return -1
		}
		if left.AccessTokenHash > right.AccessTokenHash {
			return 1
		}
		return 0
	})
	if len(candidates) > request.BatchSize {
		candidates = candidates[:request.BatchSize]
	}
	for _, session := range candidates {
		if err := ctx.Err(); err != nil {
			return 0, err
		}
		delete(a.sessions, session.AccessTokenHash)
		delete(a.refreshIndex, session.RefreshTokenHash)
	}
	return int64(len(candidates)), nil
}

func (a *authenticator) CleanupOrphanedDeviceBindings(ctx context.Context, request store.RetentionCleanupRequest) (int64, error) {
	if err := request.Validate(); err != nil {
		return 0, err
	}
	if a.store != nil {
		cleaner, ok := a.store.(store.AuthSessionBindingCleaner)
		if !ok {
			return 0, errors.New("configured auth session store does not support orphan binding cleanup")
		}
		return cleaner.CleanupOrphanedDeviceBindings(ctx, request)
	}
	// Memory sessions and control-plane state share one process-owned map; the
	// binding is either compensated synchronously or removed with the session.
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	return 0, nil
}
