package store

import (
	"context"
	"errors"
	"slices"
	"strings"
	"time"
)

const MaxRetentionCleanupBatchSize = 1000

var ErrNormalizedRetentionCleanupRequired = errors.New("retention cleanup requires normalized postgres storage")

// RetentionCleanupRequest bounds one retention pass for one data set.
type RetentionCleanupRequest struct {
	Cutoff    time.Time
	BatchSize int
}

type ControlPlaneRetentionCleaner interface {
	CleanupIdempotencyRecords(ctx context.Context, request RetentionCleanupRequest) (int64, error)
	CleanupModelPoolTestResults(ctx context.Context, request RetentionCleanupRequest) (int64, error)
	CleanupAuditLogs(ctx context.Context, request RetentionCleanupRequest) (int64, error)
}

type AuthSessionRetentionCleaner interface {
	CleanupAuthSessions(ctx context.Context, request RetentionCleanupRequest) (int64, error)
}

// AuthSessionBindingCleaner reconciles durable session-to-device bindings
// left behind when the follow-up business transaction cannot complete. It is
// deliberately optional so in-memory stores do not pretend to have a durable
// cross-store recovery mechanism.
type AuthSessionBindingCleaner interface {
	CleanupOrphanedDeviceBindings(ctx context.Context, request RetentionCleanupRequest) (int64, error)
}

// StagedSecretCleaner removes only expired, uniquely-prefixed rotation
// candidates that are not currently referenced by any model account. The
// protected set is supplied by the business store so the secret store never
// decides which reference is active on its own.
type StagedSecretCleaner interface {
	CleanupStagedSecrets(ctx context.Context, request RetentionCleanupRequest, protectedReferences []string) (int64, error)
}

// NormalizedStagedSecretCleaner reconciles staged ciphertext against the
// normalized model_accounts table in one database transaction. It must not
// derive active references from a materialized State or a legacy snapshot.
type NormalizedStagedSecretCleaner interface {
	CleanupUnreferencedStagedSecrets(ctx context.Context, request RetentionCleanupRequest) (int64, error)
}

// SecretReferenceCleaner removes a bounded, explicitly identified set of
// retired references. The protected set prevents a retry from deleting a
// reference that became active again while compensation was pending.
type SecretReferenceCleaner interface {
	CleanupSecretReferences(ctx context.Context, request RetentionCleanupRequest, references, protectedReferences []string) ([]string, error)
}

var (
	_ ControlPlaneRetentionCleaner  = (*MemoryStore)(nil)
	_ ControlPlaneRetentionCleaner  = (*PostgresRepository)(nil)
	_ NormalizedStagedSecretCleaner = (*PostgresRepository)(nil)
	_ AuthSessionRetentionCleaner   = (*SQLSessionStore)(nil)
	_ AuthSessionBindingCleaner     = (*SQLSessionStore)(nil)
	_ StagedSecretCleaner           = (*EncryptedSQLSecretStore)(nil)
	_ SecretReferenceCleaner        = (*EncryptedSQLSecretStore)(nil)
	_ SecretReferenceCleaner        = (*MemorySecretStore)(nil)
)

func validateRetentionCleanupRequest(request RetentionCleanupRequest) error {
	return request.Validate()
}

func (request RetentionCleanupRequest) Validate() error {
	if request.Cutoff.IsZero() {
		return errors.New("retention cleanup cutoff is required")
	}
	if request.BatchSize < 1 || request.BatchSize > MaxRetentionCleanupBatchSize {
		return errors.New("retention cleanup batch size is out of range")
	}
	return nil
}

func normalizeSecretReferences(references []string, limit int) []string {
	seen := make(map[string]struct{}, len(references))
	capacity := len(references)
	if capacity > limit {
		capacity = limit
	}
	result := make([]string, 0, capacity)
	for _, reference := range references {
		reference = strings.TrimSpace(reference)
		if reference == "" {
			continue
		}
		if _, exists := seen[reference]; exists {
			continue
		}
		seen[reference] = struct{}{}
		result = append(result, reference)
	}
	slices.Sort(result)
	if len(result) > limit {
		return result[:limit]
	}
	return result
}
