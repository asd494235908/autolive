package store

import (
	"context"
	"slices"
	"time"
)

type retentionCandidate struct {
	key       string
	createdAt time.Time
}

func (s *MemoryStore) CleanupIdempotencyRecords(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	// The legacy in-memory State has no created_at field for idempotency
	// records; MemoryStore tracks first-observed time separately for bounded
	// test cleanup. PostgreSQL remains the authoritative retention source.
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	return s.cleanupMemoryRecords(ctx, request, func(state *State) ([]retentionCandidate, error) {
		items := make([]retentionCandidate, 0, len(state.IdempotencyRecords))
		for key := range state.IdempotencyRecords {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			if createdAt, ok := s.idempotencyRecordCreated[key]; ok && createdAt.Before(request.Cutoff) {
				items = append(items, retentionCandidate{key: key, createdAt: createdAt})
			}
		}
		return items, nil
	}, func(state *State, key string) {
		delete(state.IdempotencyRecords, key)
	})
}

func (s *MemoryStore) CleanupModelPoolTestResults(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	return s.cleanupMemoryRecords(ctx, request, func(state *State) ([]retentionCandidate, error) {
		items := make([]retentionCandidate, 0, len(state.ModelPoolTestResults))
		for key, result := range state.ModelPoolTestResults {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			if testedAt, ok := parseRetentionTimestamp(result.TestedAt); ok && testedAt.Before(request.Cutoff) {
				items = append(items, retentionCandidate{key: key, createdAt: testedAt})
			}
		}
		return items, nil
	}, func(state *State, key string) {
		delete(state.ModelPoolTestResults, key)
	})
}

func (s *MemoryStore) CleanupAuditLogs(ctx context.Context, request RetentionCleanupRequest) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	return s.cleanupMemoryRecords(ctx, request, func(state *State) ([]retentionCandidate, error) {
		items := make([]retentionCandidate, 0, len(state.AuditLogs))
		for key, auditLog := range state.AuditLogs {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			if createdAt, ok := parseRetentionTimestamp(auditLog.CreatedAt); ok && createdAt.Before(request.Cutoff) {
				items = append(items, retentionCandidate{key: key, createdAt: createdAt})
			}
		}
		return items, nil
	}, func(state *State, key string) {
		delete(state.AuditLogs, key)
	})
}

func (s *MemoryStore) cleanupMemoryRecords(
	ctx context.Context,
	request RetentionCleanupRequest,
	collect func(*State) ([]retentionCandidate, error),
	remove func(*State, string),
) (int64, error) {
	var deleted int64
	err := s.Run(ctx, func(state *State) error {
		if err := ctx.Err(); err != nil {
			return err
		}
		items, err := collect(state)
		if err != nil {
			return err
		}
		slices.SortFunc(items, func(left, right retentionCandidate) int {
			if order := left.createdAt.Compare(right.createdAt); order != 0 {
				return order
			}
			if left.key < right.key {
				return -1
			}
			if left.key > right.key {
				return 1
			}
			return 0
		})
		if len(items) > request.BatchSize {
			items = items[:request.BatchSize]
		}
		for _, item := range items {
			if err := ctx.Err(); err != nil {
				return err
			}
			remove(state, item.key)
			deleted++
		}
		return nil
	})
	return deleted, err
}

func parseRetentionTimestamp(value string) (time.Time, bool) {
	parsed, err := time.Parse(time.RFC3339, value)
	return parsed, err == nil
}
