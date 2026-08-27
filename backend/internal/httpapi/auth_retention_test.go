package httpapi

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

func TestMemoryAuthenticatorRetentionCleanupUsesCutoffAndBatch(t *testing.T) {
	cutoff := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	auth := &authenticator{
		sessions: map[string]sessionRecord{
			"oldest-access": {AccessTokenHash: "oldest-access", RefreshTokenHash: "oldest-refresh", RefreshExpiresAt: cutoff.Add(-2 * time.Hour)},
			"old-access":    {AccessTokenHash: "old-access", RefreshTokenHash: "old-refresh", RefreshExpiresAt: cutoff.Add(-time.Hour)},
			"new-access":    {AccessTokenHash: "new-access", RefreshTokenHash: "new-refresh", RefreshExpiresAt: cutoff.Add(time.Hour)},
		},
		refreshIndex: map[string]string{
			"oldest-refresh": "oldest-access",
			"old-refresh":    "old-access",
			"new-refresh":    "new-access",
		},
	}
	deleted, err := auth.CleanupAuthSessions(context.Background(), store.RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 1})
	if err != nil {
		t.Fatalf("CleanupAuthSessions() error = %v", err)
	}
	if deleted != 1 {
		t.Fatalf("deleted = %d, want 1", deleted)
	}
	if _, ok := auth.sessions["oldest-access"]; ok {
		t.Error("oldest session was retained")
	}
	if _, ok := auth.refreshIndex["oldest-refresh"]; ok {
		t.Error("oldest refresh index was retained")
	}
	if _, ok := auth.sessions["old-access"]; !ok {
		t.Error("batch limit removed a second old session")
	}
	if _, ok := auth.sessions["new-access"]; !ok {
		t.Error("new session was removed")
	}
}

func TestMemoryAuthenticatorRetentionCleanupIsCancellable(t *testing.T) {
	auth := &authenticator{sessions: map[string]sessionRecord{}, refreshIndex: map[string]string{}}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	deleted, err := auth.CleanupAuthSessions(ctx, store.RetentionCleanupRequest{Cutoff: time.Now(), BatchSize: 1})
	if !errors.Is(err, context.Canceled) || deleted != 0 {
		t.Fatalf("CleanupAuthSessions() = (%d, %v), want (0, context canceled)", deleted, err)
	}
}

func TestMemoryAuthenticatorRetentionCleanupCanRunInOwnedScheduler(t *testing.T) {
	cutoff := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	auth := &authenticator{
		sessions: map[string]sessionRecord{
			"expired": {AccessTokenHash: "expired", RefreshTokenHash: "expired-refresh", RefreshExpiresAt: cutoff.Add(-time.Hour)},
		},
		refreshIndex: map[string]string{"expired-refresh": "expired"},
	}
	scheduler, err := service.NewRetentionCleanupScheduler(nil, auth, service.RetentionCleanupSchedulerOptions{
		BatchSize: 1,
		Policy: service.RetentionCleanupPolicy{
			AuthSessionTTL:       time.Hour,
			AuthThrottleTTL:      time.Hour,
			IdempotencyRecordTTL: time.Hour,
			ModelTestResultTTL:   time.Hour,
			AuditLogTTL:          time.Hour,
		},
		Now: func() time.Time { return cutoff.Add(2 * time.Hour) },
	})
	if err != nil {
		t.Fatalf("NewRetentionCleanupScheduler() error = %v", err)
	}
	if _, err := scheduler.RunOnce(context.Background()); err != nil {
		t.Fatalf("RunOnce() error = %v", err)
	}
	if _, exists := auth.sessions["expired"]; exists {
		t.Fatal("scheduler retained expired memory session")
	}
}

func TestAuthenticatorForwardsOrphanBindingCleanupToPersistentStore(t *testing.T) {
	cleaner := &recordingBindingSessionStore{}
	auth := &authenticator{store: cleaner}
	request := store.RetentionCleanupRequest{Cutoff: time.Date(2026, 8, 21, 11, 55, 0, 0, time.UTC), BatchSize: 9}
	deleted, err := auth.CleanupOrphanedDeviceBindings(context.Background(), request)
	if err != nil {
		t.Fatalf("CleanupOrphanedDeviceBindings() error = %v", err)
	}
	if deleted != 4 || cleaner.request != request {
		t.Fatalf("forwarded cleanup = (%d, %+v), want (4, %+v)", deleted, cleaner.request, request)
	}
}

type recordingBindingSessionStore struct {
	testSessionStore
	request store.RetentionCleanupRequest
}

func (s *recordingBindingSessionStore) CleanupOrphanedDeviceBindings(_ context.Context, request store.RetentionCleanupRequest) (int64, error) {
	s.request = request
	return 4, nil
}
