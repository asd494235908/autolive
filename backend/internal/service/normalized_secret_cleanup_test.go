package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/store"
)

func TestCleanupStagedSecretsNormalizedFailsClosedWithoutDatabaseCleaner(t *testing.T) {
	repository := &normalizedSecretCleanupRepository{}
	service := NewControlPlaneWithRepository(repository)
	_, err := service.CleanupStagedSecrets(context.Background(), store.RetentionCleanupRequest{
		Cutoff: time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC), BatchSize: 10,
	})
	if !errors.Is(err, store.ErrNormalizedStagedSecretCleanerRequired) {
		t.Fatalf("CleanupStagedSecrets() error = %v, want normalized cleaner requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized cleanup used StateOperation %d times", repository.runCalls)
	}
}

type normalizedSecretCleanupRepository struct {
	runCalls int
}

func (*normalizedSecretCleanupRepository) Now() time.Time { return time.Unix(0, 0) }

func (r *normalizedSecretCleanupRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized cleanup must not use StateOperation")
}

func (*normalizedSecretCleanupRepository) UsesNormalizedReadSource() bool { return true }
