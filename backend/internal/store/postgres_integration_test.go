//go:build postgres_integration

package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/migrationscheck"

	_ "github.com/lib/pq"
)

func TestPostgresNormalizedRoundTripPersistsDomainState(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)

	now := time.Date(2026, 8, 20, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	userID := fmt.Sprintf("int_%d", now.UnixNano())
	idempotencyKey := "integration:" + userID
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = 'control-plane-state' AND idempotency_key = $1`, idempotencyKey)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	if err := repository.Run(ctx, func(state *State) error {
		state.Users[userID] = controlplane.UserSummary{
			ID: userID, Username: "integration-user", Role: controlplane.RoleUser,
			Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339),
		}
		state.UserCredentialHashes[userID] = []byte("$2a$10$integration-hash")
		state.IdempotencyRecords[idempotencyKey] = IdempotencyRecord{Fingerprint: "integration-fingerprint", ResourceID: userID}
		return nil
	}); err != nil {
		t.Fatalf("normalized write error = %v", err)
	}

	restartedRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted repository constructor error = %v", err)
	}
	var restored *State
	if err := restartedRepository.Run(ctx, func(state *State) error {
		restored = state
		return nil
	}); err != nil {
		t.Fatalf("normalized read error = %v", err)
	}
	if restored.Users[userID].Username != "integration-user" {
		t.Fatalf("restored user = %+v", restored.Users[userID])
	}
	if restored.IdempotencyRecords[idempotencyKey].ResourceID != userID {
		t.Fatalf("restored idempotency record = %+v", restored.IdempotencyRecords[idempotencyKey])
	}
}

func TestPostgresNormalizedTransactionRollsBackOnConstraintFailure(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Date(2026, 8, 20, 13, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	suffix := time.Now().UTC().UnixNano()
	baseUserID := fmt.Sprintf("rollback_base_%d", suffix)
	missingPolicyUserID := fmt.Sprintf("rollback_missing_%d", suffix)
	username := fmt.Sprintf("rollback-user-%d", suffix)
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, baseUserID)
	})

	if err := repository.Run(ctx, func(state *State) error {
		state.Users[baseUserID] = controlplane.UserSummary{
			ID: baseUserID, Username: username, Role: controlplane.RoleUser,
			Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339),
		}
		state.UserCredentialHashes[baseUserID] = []byte("$2a$10$integration-hash")
		return nil
	}); err != nil {
		t.Fatalf("seed normalized user: %v", err)
	}

	err = repository.Run(ctx, func(state *State) error {
		base := state.Users[baseUserID]
		base.Status = controlplane.UserStatusDisabled
		state.Users[baseUserID] = base
		state.UserAuthorizationPolicies[missingPolicyUserID] = controlplane.UserAuthorizationPolicy{
			UserID: missingPolicyUserID, AllowedModels: []string{"openai/model"}, UpdatedAt: now.Format(time.RFC3339),
		}
		return nil
	})
	if err == nil {
		t.Fatal("invalid normalized write error = nil, want foreign-key failure")
	}

	restartedRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted repository constructor error = %v", err)
	}
	var restored *State
	if err := restartedRepository.Run(ctx, func(state *State) error {
		restored = state
		return nil
	}); err != nil {
		t.Fatalf("read after rollback: %v", err)
	}
	if restored.Users[baseUserID].Status != controlplane.UserStatusActive {
		t.Fatalf("base user status after rollback = %q", restored.Users[baseUserID].Status)
	}
	if _, exists := restored.UserAuthorizationPolicies[missingPolicyUserID]; exists {
		t.Fatal("invalid authorization policy persisted after rollback")
	}
}

func TestPostgresNormalizedConcurrentIdempotencySeesCommittedWinner(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	idempotencyKey := fmt.Sprintf("concurrent-idempotency:%d", suffix)
	userIDs := []string{
		fmt.Sprintf("concurrent_user_a_%d", suffix),
		fmt.Sprintf("concurrent_user_b_%d", suffix),
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = 'control-plane-state' AND idempotency_key = $1`, idempotencyKey)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id IN ($1, $2)`, userIDs[0], userIDs[1])
	})

	repositories := make([]*PostgresRepository, 2)
	for index := range repositories {
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("repository %d constructor: %v", index, err)
		}
		repositories[index] = repository
	}
	type runResult struct {
		created    bool
		resourceID string
		err        error
	}
	start := make(chan struct{})
	results := make(chan runResult, len(repositories))
	var waitGroup sync.WaitGroup
	for index, repository := range repositories {
		waitGroup.Add(1)
		go func(index int, repository *PostgresRepository) {
			defer waitGroup.Done()
			<-start
			result := runResult{}
			result.err = repository.Run(ctx, func(state *State) error {
				if existing, exists := state.IdempotencyRecords[idempotencyKey]; exists {
					result.resourceID = existing.ResourceID
					return nil
				}
				result.created = true
				result.resourceID = userIDs[index]
				state.Users[result.resourceID] = controlplane.UserSummary{
					ID: result.resourceID, Username: fmt.Sprintf("concurrent-user-%d-%d", suffix, index),
					Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339),
				}
				state.UserCredentialHashes[result.resourceID] = []byte("$2a$10$integration-hash")
				state.IdempotencyRecords[idempotencyKey] = IdempotencyRecord{Fingerprint: "integration-fingerprint", ResourceID: result.resourceID}
				return nil
			})
			results <- result
		}(index, repository)
	}
	close(start)
	waitGroup.Wait()
	close(results)

	createdCount := 0
	creatorID := ""
	duplicateResourceID := ""
	for result := range results {
		if result.err != nil {
			t.Fatalf("concurrent repository run: %v", result.err)
		}
		if result.created {
			createdCount++
			creatorID = result.resourceID
		} else {
			duplicateResourceID = result.resourceID
		}
	}
	if createdCount != 1 || creatorID == "" || duplicateResourceID != creatorID {
		t.Fatalf("concurrent results: created=%d creator=%q duplicate observed=%q", createdCount, creatorID, duplicateResourceID)
	}

	verifier, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("verifier constructor: %v", err)
	}
	var restored *State
	if err := verifier.Run(ctx, func(state *State) error {
		restored = state
		return nil
	}); err != nil {
		t.Fatalf("verify concurrent state: %v", err)
	}
	if restored.IdempotencyRecords[idempotencyKey].ResourceID != creatorID {
		t.Fatalf("stored idempotency record = %+v", restored.IdempotencyRecords[idempotencyKey])
	}
	persistedCandidates := 0
	for _, userID := range userIDs {
		if _, exists := restored.Users[userID]; exists {
			persistedCandidates++
		}
	}
	if persistedCandidates != 1 {
		t.Fatalf("persisted candidate users = %d, want 1", persistedCandidates)
	}
}

func TestPostgresNormalizedActivationRedeemIsSingleWinnerAcrossRepositories(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("activation_race_user_%d", suffix)
	activationID := fmt.Sprintf("activation_race_code_%d", suffix)
	activationHash := fmt.Sprintf("activation-race-hash-%d", suffix)
	deviceIDs := []string{
		fmt.Sprintf("activation_race_device_a_%d", suffix),
		fmt.Sprintf("activation_race_device_b_%d", suffix),
	}
	accessTokens := []string{
		fmt.Sprintf("activation_race_access_a_%d", suffix),
		fmt.Sprintf("activation_race_access_b_%d", suffix),
	}
	username := fmt.Sprintf("activation-race-user-%d", suffix)
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed activation race user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO activation_codes (id, bound_user_id, code_hash, code_prefix, status, created_at, expires_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
	`, activationID, userID, activationHash, "race_", controlplane.ActivationCodeStatusActive, now, now.Add(time.Hour)); err != nil {
		t.Fatalf("seed activation race code: %v", err)
	}
	for index := range accessTokens {
		if _, err := database.ExecContext(ctx, `
			INSERT INTO auth_sessions (
				id, user_id, access_token_hash, refresh_token_hash,
				access_expires_at, refresh_expires_at, created_at
			) VALUES ($1, $2, $3, $4, $5, $6, $7)
		`, fmt.Sprintf("activation_race_session_%d_%d", suffix, index), userID, accessTokens[index], fmt.Sprintf("activation_race_refresh_%d_%d", suffix, index), now.Add(time.Hour), now.Add(24*time.Hour), now); err != nil {
			t.Fatalf("seed activation race session %d: %v", index, err)
		}
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_device_bindings WHERE activation_code_id = $1`, activationID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_codes WHERE id = $1`, activationID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key LIKE $2`, "control-plane-state", fmt.Sprintf("activate-device:%d:%%", suffix))
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id IN ($1, $2)`, deviceIDs[0], deviceIDs[1])
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	repositories := make([]*PostgresRepository, 2)
	for index := range repositories {
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("repository %d constructor: %v", index, err)
		}
		repositories[index] = repository
	}
	start := make(chan struct{})
	type activationResult struct {
		index  int
		device controlplane.DeviceSummary
		err    error
	}
	results := make(chan activationResult, len(repositories))
	var waitGroup sync.WaitGroup
	for index, repository := range repositories {
		waitGroup.Add(1)
		go func(index int, repository *PostgresRepository) {
			defer waitGroup.Done()
			<-start
			device, err := repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
				Scope:           "control-plane-state",
				IdempotencyKey:  fmt.Sprintf("activate-device:%d:%d", suffix, index),
				Fingerprint:     fmt.Sprintf("activation-race-fingerprint-%d", index),
				AccessTokenHash: accessTokens[index],
				UserID:          userID,
				Product:         controlplane.ProductAutoLive,
				Device: controlplane.DeviceRegistration{
					Product:  controlplane.ProductAutoLive,
					DeviceID: deviceIDs[index], DeviceName: fmt.Sprintf("race-device-%d", index),
					Platform: "integration", AppVersion: "test",
				},
			})
			results <- activationResult{index: index, device: device, err: err}
		}(index, repository)
	}
	close(start)
	waitGroup.Wait()
	close(results)

	winners := 0
	losers := 0
	winnerIndex := -1
	winnerDeviceID := ""
	for result := range results {
		if result.err == nil {
			winners++
			winnerIndex = result.index
			winnerDeviceID = result.device.ID
			continue
		}
		if !errors.Is(result.err, controlplane.ErrDeviceLimitExceeded) {
			t.Fatalf("concurrent activation error = %v, want device-limit loser", result.err)
		}
		losers++
	}
	if winners != 1 || losers != 1 || winnerDeviceID == "" {
		t.Fatalf("concurrent activation outcomes = winners %d losers %d winner %q", winners, losers, winnerDeviceID)
	}
	replayRepository := repositories[(winnerIndex+1)%len(repositories)]
	replayedDevice, err := replayRepository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:           "control-plane-state",
		IdempotencyKey:  fmt.Sprintf("activate-device:%d:%d", suffix, winnerIndex),
		Fingerprint:     fmt.Sprintf("activation-race-fingerprint-%d", winnerIndex),
		AccessTokenHash: accessTokens[winnerIndex],
		UserID:          userID,
		Product:         controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{
			Product:  controlplane.ProductAutoLive,
			DeviceID: deviceIDs[winnerIndex], DeviceName: fmt.Sprintf("race-device-%d", winnerIndex),
			Platform: "integration", AppVersion: "test",
		},
	})
	if err != nil || replayedDevice.ID != winnerDeviceID {
		t.Fatalf("cross-repository activation replay = (%+v, %v), want winner %q", replayedDevice, err, winnerDeviceID)
	}

	var status, usedDeviceID string
	if err := database.QueryRowContext(ctx, `SELECT status, used_by_device_id FROM activation_codes WHERE id = $1`, activationID).Scan(&status, &usedDeviceID); err != nil {
		t.Fatalf("read redeemed activation code: %v", err)
	}
	if status != controlplane.ActivationCodeStatusUsed || usedDeviceID != winnerDeviceID {
		t.Fatalf("redeemed activation code = status %q device %q, want used/%q", status, usedDeviceID, winnerDeviceID)
	}
	var deviceCount, boundSessionCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE id IN ($1, $2)`, deviceIDs[0], deviceIDs[1]).Scan(&deviceCount); err != nil {
		t.Fatalf("count activated devices: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND device_id IS NOT NULL`, userID).Scan(&boundSessionCount); err != nil {
		t.Fatalf("count bound activation sessions: %v", err)
	}
	if deviceCount != 1 || boundSessionCount != 1 {
		t.Fatalf("activation race persisted devices=%d bound_sessions=%d, want 1/1", deviceCount, boundSessionCount)
	}
}

func TestPostgresNormalizedActivationRejectsExpiredOrRevokedCodes(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	cases := []struct {
		name      string
		status    string
		expiresAt time.Time
		wantError error
	}{
		{
			name:      "expired",
			status:    controlplane.ActivationCodeStatusActive,
			expiresAt: now.Add(-time.Minute),
			wantError: controlplane.ErrAccountActivationExpired,
		},
		{
			name:      "revoked",
			status:    controlplane.ActivationCodeStatusRevoked,
			expiresAt: now.Add(time.Hour),
			wantError: controlplane.ErrAccountActivationRequired,
		},
	}
	for _, testCase := range cases {
		t.Run(testCase.name, func(t *testing.T) {
			fixture := seedPostgresActivationFixture(t, database, ctx, now, testCase.status, testCase.expiresAt, 1)
			repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
			if err != nil {
				t.Fatalf("repository constructor: %v", err)
			}

			_, err = repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
				Scope:           "control-plane-state",
				IdempotencyKey:  fixture.IdempotencyPrefix + "redeem",
				Fingerprint:     fixture.IdempotencyPrefix + "fingerprint",
				AccessTokenHash: fixture.AccessTokenHashes[0],
				UserID:          fixture.UserID,
				Product:         controlplane.ProductAutoLive,
				Device: controlplane.DeviceRegistration{
					Product:  controlplane.ProductAutoLive,
					DeviceID: fixture.DeviceIDs[0], DeviceName: "activation-edge-device",
					Platform: "integration", AppVersion: "test",
				},
			})
			if !errors.Is(err, testCase.wantError) {
				t.Fatalf("activation error = %v, want %v", err, testCase.wantError)
			}

			var status string
			var usedByDeviceID sql.NullString
			if err := database.QueryRowContext(ctx, `
				SELECT status, used_by_device_id
				FROM activation_codes
				WHERE id = $1
			`, fixture.CodeID).Scan(&status, &usedByDeviceID); err != nil {
				t.Fatalf("read rejected activation code: %v", err)
			}
			if status != testCase.status || usedByDeviceID.Valid {
				t.Fatalf("rejected activation code = status %q used_by_device_id %v, want unchanged %q and NULL", status, usedByDeviceID, testCase.status)
			}

			var boundDeviceID sql.NullString
			if err := database.QueryRowContext(ctx, `
				SELECT device_id
				FROM auth_sessions
				WHERE access_token_hash = $1
			`, fixture.AccessTokenHashes[0]).Scan(&boundDeviceID); err != nil {
				t.Fatalf("read rejected activation session: %v", err)
			}
			if boundDeviceID.Valid {
				t.Fatalf("rejected activation bound session to %q", boundDeviceID.String)
			}

			var deviceCount, idempotencyCount int
			if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE id = $1`, fixture.DeviceIDs[0]).Scan(&deviceCount); err != nil {
				t.Fatalf("count rejected activation device: %v", err)
			}
			if err := database.QueryRowContext(ctx, `
				SELECT COUNT(*)
				FROM idempotency_records
				WHERE scope = $1 AND idempotency_key = $2
			`, "control-plane-state", fixture.IdempotencyPrefix+"redeem").Scan(&idempotencyCount); err != nil {
				t.Fatalf("count rejected activation idempotency record: %v", err)
			}
			if deviceCount != 0 || idempotencyCount != 0 {
				t.Fatalf("rejected activation residue = devices %d idempotency %d, want 0/0", deviceCount, idempotencyCount)
			}
		})
	}
}

func TestPostgresNormalizedActivationRejectsCommittedRedeemAndIdempotencyConflict(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 2)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}

	winnerKey := fixture.IdempotencyPrefix + "winner"
	winnerFingerprint := fixture.IdempotencyPrefix + "winner-fingerprint"
	winner, err := repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:           "control-plane-state",
		IdempotencyKey:  winnerKey,
		Fingerprint:     winnerFingerprint,
		AccessTokenHash: fixture.AccessTokenHashes[0],
		UserID:          fixture.UserID,
		Product:         controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{
			Product:  controlplane.ProductAutoLive,
			DeviceID: fixture.DeviceIDs[0], DeviceName: "activation-winner",
			Platform: "integration", AppVersion: "test",
		},
	})
	if err != nil {
		t.Fatalf("first activation: %v", err)
	}
	if winner.ID != fixture.DeviceIDs[0] {
		t.Fatalf("first activation device = %q, want %q", winner.ID, fixture.DeviceIDs[0])
	}

	_, err = repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:           "control-plane-state",
		IdempotencyKey:  fixture.IdempotencyPrefix + "duplicate",
		Fingerprint:     fixture.IdempotencyPrefix + "duplicate-fingerprint",
		AccessTokenHash: fixture.AccessTokenHashes[1],
		UserID:          fixture.UserID,
		Product:         controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{
			Product:  controlplane.ProductAutoLive,
			DeviceID: fixture.DeviceIDs[1], DeviceName: "activation-duplicate",
			Platform: "integration", AppVersion: "test",
		},
	})
	if !errors.Is(err, controlplane.ErrDeviceLimitExceeded) {
		t.Fatalf("second activation error = %v, want device limit exceeded", err)
	}

	restartedRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted repository constructor: %v", err)
	}
	_, err = restartedRepository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:           "control-plane-state",
		IdempotencyKey:  winnerKey,
		Fingerprint:     fixture.IdempotencyPrefix + "different-fingerprint",
		AccessTokenHash: fixture.AccessTokenHashes[0],
		UserID:          fixture.UserID,
		Product:         controlplane.ProductAutoLive,
		Device: controlplane.DeviceRegistration{
			Product:  controlplane.ProductAutoLive,
			DeviceID: fixture.DeviceIDs[0], DeviceName: "activation-winner",
			Platform: "integration", AppVersion: "test",
		},
	})
	if !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("conflicting replay error = %v, want idempotency conflict", err)
	}

	var status, usedByDeviceID string
	if err := database.QueryRowContext(ctx, `
		SELECT status, used_by_device_id
		FROM activation_codes
		WHERE id = $1
	`, fixture.CodeID).Scan(&status, &usedByDeviceID); err != nil {
		t.Fatalf("read committed activation: %v", err)
	}
	if status != controlplane.ActivationCodeStatusUsed || usedByDeviceID != fixture.DeviceIDs[0] {
		t.Fatalf("committed activation = status %q used_by_device_id %q, want used/%q", status, usedByDeviceID, fixture.DeviceIDs[0])
	}
	var deviceCount, boundSessionCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE id IN ($1, $2)`, fixture.DeviceIDs[0], fixture.DeviceIDs[1]).Scan(&deviceCount); err != nil {
		t.Fatalf("count activation devices: %v", err)
	}
	if err := database.QueryRowContext(ctx, `
		SELECT COUNT(*)
		FROM auth_sessions
		WHERE user_id = $1 AND device_id IS NOT NULL
	`, fixture.UserID).Scan(&boundSessionCount); err != nil {
		t.Fatalf("count activation sessions: %v", err)
	}
	if deviceCount != 1 || boundSessionCount != 1 {
		t.Fatalf("activation residue = devices %d bound_sessions %d, want 1/1", deviceCount, boundSessionCount)
	}
}

func TestPostgresNormalizedActivationBindsConfiguredDeviceCount(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 2, 2)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	for index := range fixture.DeviceIDs {
		device, activateErr := repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
			Scope:           "control-plane-state",
			IdempotencyKey:  fixture.IdempotencyPrefix + fmt.Sprintf("redeem-%d", index),
			Fingerprint:     fixture.IdempotencyPrefix + fmt.Sprintf("fingerprint-%d", index),
			AccessTokenHash: fixture.AccessTokenHashes[index],
			UserID:          fixture.UserID,
			Product:         controlplane.ProductAutoLive,
			Device: controlplane.DeviceRegistration{
				Product:  controlplane.ProductAutoLive,
				DeviceID: fixture.DeviceIDs[index], DeviceName: "multi-device",
				Platform: "integration", AppVersion: "test",
			},
		})
		if activateErr != nil || device.ID != fixture.DeviceIDs[index] {
			t.Fatalf("activation %d = device %+v, error %v", index+1, device, activateErr)
		}
	}

	var status string
	var maxDevices, boundDevices int
	if err := database.QueryRowContext(ctx, `SELECT status, max_devices, bound_devices FROM activation_codes WHERE id = $1`, fixture.CodeID).Scan(&status, &maxDevices, &boundDevices); err != nil {
		t.Fatalf("read multi-device activation: %v", err)
	}
	if status != controlplane.ActivationCodeStatusUsed || maxDevices != 2 || boundDevices != 2 {
		t.Fatalf("multi-device activation = status %q max %d bound %d, want used/2/2", status, maxDevices, boundDevices)
	}
}

type postgresActivationFixture struct {
	UserID            string
	CodeID            string
	CodeHash          string
	DeviceIDs         []string
	AccessTokenHashes []string
	IdempotencyPrefix string
}

func seedPostgresActivationFixture(t *testing.T, database *sql.DB, ctx context.Context, now time.Time, status string, expiresAt time.Time, sessionCount int, configuredMaxDevices ...int) postgresActivationFixture {
	t.Helper()
	if sessionCount < 1 {
		t.Fatalf("session count = %d, want at least one", sessionCount)
	}
	suffix := fmt.Sprintf("%d", time.Now().UTC().UnixNano())
	fixture := postgresActivationFixture{
		UserID:            "activation_edge_user_" + suffix,
		CodeID:            "activation_edge_code_" + suffix,
		CodeHash:          "activation-edge-hash-" + suffix,
		IdempotencyPrefix: "activation-edge:" + suffix + ":",
		DeviceIDs:         make([]string, sessionCount),
		AccessTokenHashes: make([]string, sessionCount),
	}
	username := "activation-edge-user-" + suffix
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, fixture.UserID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed activation edge user: %v", err)
	}
	maxDevices := 1
	if len(configuredMaxDevices) > 0 {
		maxDevices = configuredMaxDevices[0]
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO activation_codes (id, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, max_devices, bound_devices)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0)
	`, fixture.CodeID, fixture.UserID, fixture.CodeHash, "edge_", status, now, expiresAt, maxDevices); err != nil {
		t.Fatalf("seed activation edge code: %v", err)
	}
	for index := range fixture.AccessTokenHashes {
		fixture.DeviceIDs[index] = fmt.Sprintf("activation_edge_device_%s_%d", suffix, index)
		fixture.AccessTokenHashes[index] = fmt.Sprintf("activation_edge_access_%s_%d", suffix, index)
		if _, err := database.ExecContext(ctx, `
			INSERT INTO auth_sessions (
				id, user_id, access_token_hash, refresh_token_hash,
				access_expires_at, refresh_expires_at, created_at
			) VALUES ($1, $2, $3, $4, $5, $6, $7)
		`, fmt.Sprintf("activation_edge_session_%s_%d", suffix, index), fixture.UserID, fixture.AccessTokenHashes[index], fmt.Sprintf("activation_edge_refresh_%s_%d", suffix, index), now.Add(time.Hour), now.Add(24*time.Hour), now); err != nil {
			t.Fatalf("seed activation edge session %d: %v", index, err)
		}
	}
	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE user_id = $1`, fixture.UserID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key LIKE $2`, "control-plane-state", fixture.IdempotencyPrefix+"%")
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM audit_outbox WHERE payload->>'RequestID' LIKE $1`, fixture.IdempotencyPrefix+"%")
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_device_bindings WHERE activation_code_id = $1`, fixture.CodeID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_codes WHERE id = $1`, fixture.CodeID)
		for _, deviceID := range fixture.DeviceIDs {
			_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		}
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, fixture.UserID)
	})
	return fixture
}

func TestPostgresNormalizedModelLeaseConcurrencyHasSingleWinnerAcrossRepositories(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("lease_race_user_%d", suffix)
	accountID := fmt.Sprintf("lease_race_account_%d", suffix)
	secretRef := "model-account/" + accountID
	deviceIDs := []string{
		fmt.Sprintf("lease_race_device_a_%d", suffix),
		fmt.Sprintf("lease_race_device_b_%d", suffix),
	}
	username := fmt.Sprintf("lease-race-user-%d", suffix)
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed lease race user: %v", err)
	}
	for index, deviceID := range deviceIDs {
		if _, err := database.ExecContext(ctx, `
			INSERT INTO devices (id, user_id, device_key, client_version, status, device_name, platform)
			VALUES ($1, $2, $3, $4, $5, $6, $7)
		`, deviceID, userID, fmt.Sprintf("lease-race-key-%d-%d", suffix, index), "integration", controlplane.DeviceStatusActive, fmt.Sprintf("race-device-%d", index), "test"); err != nil {
			t.Fatalf("seed lease race device %d: %v", index, err)
		}
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "lease-model", "https://provider.example.test", secretRef,
		controlplane.ModelAccountStatusActive, 1, 1, 0, now); err != nil {
		t.Fatalf("seed lease race account: %v", err)
	}
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("lease race secret store constructor: %v", err)
	}
	if err := secretStore.Put(ctx, secretRef, "integration-lease-secret"); err != nil {
		t.Fatalf("seed lease race secret: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_usage_records WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_leases WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key LIKE $2`, "integration-model-lease", fmt.Sprintf("lease-race:%d:%%", suffix))
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, secretRef)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id IN ($1, $2)`, deviceIDs[0], deviceIDs[1])
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	repositories := make([]*PostgresRepository, 2)
	for index := range repositories {
		repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretStore, ModelReadSourceNormalized)
		if err != nil {
			t.Fatalf("repository %d constructor: %v", index, err)
		}
		repositories[index] = repository
	}
	type leaseResult struct {
		lease controlplane.ModelLease
		err   error
	}
	start := make(chan struct{})
	results := make(chan leaseResult, len(repositories))
	var waitGroup sync.WaitGroup
	for index, repository := range repositories {
		waitGroup.Add(1)
		go func(index int, repository *PostgresRepository) {
			defer waitGroup.Done()
			<-start
			lease, err := repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
				Scope:              "integration-model-lease",
				IdempotencyKey:     fmt.Sprintf("lease-race:%d:%d", suffix, index),
				Fingerprint:        fmt.Sprintf("lease-race-fingerprint:%d", index),
				UserID:             userID,
				DeviceID:           deviceIDs[index],
				Provider:           "integration",
				Model:              "lease-model",
				Purpose:            "integration",
				MaxDurationSeconds: 60,
			})
			results <- leaseResult{lease: lease, err: err}
		}(index, repository)
	}
	close(start)
	waitGroup.Wait()
	close(results)

	var winner controlplane.ModelLease
	successes := 0
	for result := range results {
		if result.err == nil {
			successes++
			winner = result.lease
			continue
		}
		if !controlplane.IsErrorCode(result.err, controlplane.ErrModelPoolUnavailable.Code) {
			t.Fatalf("concurrent lease error = %v, want MODEL_POOL_UNAVAILABLE", result.err)
		}
	}
	if successes != 1 || winner.ID == "" {
		t.Fatalf("concurrent lease successes = %d, winner = %+v; want exactly one", successes, winner)
	}
	var activeCount int
	if err := database.QueryRowContext(ctx, `
		SELECT COUNT(*) FROM model_leases
		WHERE account_id = $1 AND status = $2 AND expires_at > $3
	`, accountID, controlplane.ModelLeaseStatusActive, now).Scan(&activeCount); err != nil {
		t.Fatalf("count active lease race rows: %v", err)
	}
	if activeCount != 1 {
		t.Fatalf("active lease rows = %d, want 1", activeCount)
	}
}

func TestPostgresNormalizedModelDailyQuotaRecoversAfterUTCDayRollover(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	start := time.Date(2026, 8, 21, 23, 59, 59, 0, time.UTC)
	now := start
	suffix := start.UnixNano()
	userID := fmt.Sprintf("quota_rollover_user_%d", suffix)
	accountID := fmt.Sprintf("quota_rollover_account_%d", suffix)
	deviceID := fmt.Sprintf("quota_rollover_device_%d", suffix)
	secretRef := "model-account/" + accountID
	username := fmt.Sprintf("quota-rollover-user-%d", suffix)
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, start); err != nil {
		t.Fatalf("seed quota rollover user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, client_version, status, device_name, platform)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
	`, deviceID, userID, "quota-rollover-device-key", "integration", controlplane.DeviceStatusActive, "quota-rollover-device", "test"); err != nil {
		t.Fatalf("seed quota rollover device: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "quota-model", "https://provider.example.test", secretRef,
		controlplane.ModelAccountStatusActive, 1, 2, 10, start); err != nil {
		t.Fatalf("seed quota rollover account: %v", err)
	}
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("quota rollover secret store constructor: %v", err)
	}
	if err := secretStore.Put(ctx, secretRef, "integration-quota-secret"); err != nil {
		t.Fatalf("seed quota rollover secret: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_usage_records WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_leases WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key LIKE $2`, "integration-quota-rollover", fmt.Sprintf("quota-rollover:%d:%%", suffix))
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, secretRef)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("quota rollover repository constructor: %v", err)
	}
	firstLease, err := repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: "integration-quota-rollover", IdempotencyKey: fmt.Sprintf("quota-rollover:%d:first-lease", suffix), Fingerprint: "quota-rollover-first-lease",
		UserID: userID, DeviceID: deviceID, Provider: "integration", Model: "quota-model", Purpose: "integration", MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("create first quota rollover lease: %v", err)
	}
	if _, err := repository.RecordDirectLLMCall(ctx, ModelUsageWriteRecord{
		Scope: "integration-quota-rollover", IdempotencyKey: fmt.Sprintf("quota-rollover:%d:first-usage", suffix), Fingerprint: "quota-rollover-first-usage",
		UserID: userID, DeviceID: deviceID, RequestID: "quota-rollover-request-1",
		Input: controlplane.CreateDirectLLMCallRecordInput{
			ClientCallID: "quota-rollover-call-1", LeaseID: firstLease.ID, Provider: "integration", Model: "quota-model",
			InputTokens: 6, OutputTokens: 4, TotalTokens: 10, Status: "succeeded", UsageSource: "client_reported",
		},
	}); err != nil {
		t.Fatalf("record quota rollover usage: %v", err)
	}
	var status string
	if err := database.QueryRowContext(ctx, `SELECT status FROM model_accounts WHERE id = $1`, accountID).Scan(&status); err != nil {
		t.Fatalf("read exhausted quota rollover status: %v", err)
	}
	if status != controlplane.ModelAccountStatusExhausted {
		t.Fatalf("quota rollover status before UTC rollover = %q, want exhausted", status)
	}
	if _, err := repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: "integration-quota-rollover", IdempotencyKey: fmt.Sprintf("quota-rollover:%d:blocked-lease", suffix), Fingerprint: "quota-rollover-blocked-lease",
		UserID: userID, DeviceID: deviceID, Provider: "integration", Model: "quota-model", Purpose: "integration", MaxDurationSeconds: 300,
	}); !controlplane.IsErrorCode(err, controlplane.ErrModelPoolUnavailable.Code) {
		t.Fatalf("create quota rollover lease before UTC rollover = %v, want MODEL_POOL_UNAVAILABLE", err)
	}

	now = start.Add(time.Second)
	recoveredLease, err := repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: "integration-quota-rollover", IdempotencyKey: fmt.Sprintf("quota-rollover:%d:recovered-lease", suffix), Fingerprint: "quota-rollover-recovered-lease",
		UserID: userID, DeviceID: deviceID, Provider: "integration", Model: "quota-model", Purpose: "integration", MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("create quota rollover lease after UTC rollover: %v", err)
	}
	if recoveredLease.Status != controlplane.ModelLeaseStatusActive || recoveredLease.AccountID != accountID {
		t.Fatalf("recovered quota rollover lease = %+v", recoveredLease)
	}
	if err := database.QueryRowContext(ctx, `SELECT status FROM model_accounts WHERE id = $1`, accountID).Scan(&status); err != nil {
		t.Fatalf("read recovered quota rollover status: %v", err)
	}
	if status != controlplane.ModelAccountStatusActive {
		t.Fatalf("quota rollover status after UTC rollover = %q, want active", status)
	}
	var dailyUsed int
	dayStart := time.Date(now.UTC().Year(), now.UTC().Month(), now.UTC().Day(), 0, 0, 0, 0, time.UTC)
	if err := database.QueryRowContext(ctx, `
		SELECT COALESCE(SUM(total_tokens), 0)
		FROM model_usage_records
		WHERE account_id = $1 AND created_at >= $2 AND created_at < $3
	`, accountID, dayStart, dayStart.Add(24*time.Hour)).Scan(&dailyUsed); err != nil {
		t.Fatalf("read recovered UTC-day usage: %v", err)
	}
	if dailyUsed != 0 {
		t.Fatalf("daily usage after UTC rollover = %d, want 0", dailyUsed)
	}
}

func TestPostgresAdvisoryLockCoordinatesIndependentRepositoryInstances(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC()
	first, err := NewPostgresRepository(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("first repository constructor: %v", err)
	}
	second, err := NewPostgresRepository(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("second repository constructor: %v", err)
	}
	key := now.UnixNano()
	firstRelease, acquired, err := first.TryAdvisoryLock(ctx, key)
	if err != nil || !acquired || firstRelease == nil {
		t.Fatalf("first TryAdvisoryLock() = acquired %t, release %v, error %v", acquired, firstRelease != nil, err)
	}
	secondRelease, acquired, err := second.TryAdvisoryLock(ctx, key)
	if err != nil {
		t.Fatalf("second TryAdvisoryLock() error = %v", err)
	}
	if acquired || secondRelease != nil {
		t.Fatalf("second TryAdvisoryLock() = acquired %t, release %v; want busy", acquired, secondRelease != nil)
	}
	if err := firstRelease(ctx); err != nil {
		t.Fatalf("first advisory lock release: %v", err)
	}
	thirdRelease, acquired, err := second.TryAdvisoryLock(ctx, key)
	if err != nil || !acquired || thirdRelease == nil {
		t.Fatalf("second acquire after release = acquired %t, release %v, error %v", acquired, thirdRelease != nil, err)
	}
	if err := thirdRelease(ctx); err != nil {
		t.Fatalf("second advisory lock release: %v", err)
	}
}

func TestSQLSessionStoreRestartsAndConsumesRefreshTokenOnce(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("session_user_%d", suffix)
	username := fmt.Sprintf("session-user-%d", suffix)
	first := AuthSession{
		ID:               fmt.Sprintf("session_first_%d", suffix),
		UserID:           userID,
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  fmt.Sprintf("access_first_%d", suffix),
		RefreshTokenHash: fmt.Sprintf("refresh_first_%d", suffix),
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	second := AuthSession{
		ID:               fmt.Sprintf("session_second_%d", suffix),
		UserID:           userID,
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  fmt.Sprintf("access_second_%d", suffix),
		RefreshTokenHash: fmt.Sprintf("refresh_second_%d", suffix),
		AccessExpiresAt:  now.Add(2 * time.Hour),
		RefreshExpiresAt: first.RefreshExpiresAt,
		CreatedAt:        now.Add(time.Minute),
	}
	replayCandidate := AuthSession{
		ID:               fmt.Sprintf("session_replay_%d", suffix),
		UserID:           userID,
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  fmt.Sprintf("access_replay_%d", suffix),
		RefreshTokenHash: fmt.Sprintf("refresh_replay_%d", suffix),
		AccessExpiresAt:  now.Add(3 * time.Hour),
		RefreshExpiresAt: first.RefreshExpiresAt,
		CreatedAt:        now.Add(2 * time.Minute),
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed session user: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	initialStore, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("initial session store constructor: %v", err)
	}
	if err := initialStore.Create(ctx, first); err != nil {
		t.Fatalf("create first session: %v", err)
	}

	restartedStore, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("restarted session store constructor: %v", err)
	}
	restored, found, err := restartedStore.GetByRefreshTokenHash(ctx, first.RefreshTokenHash)
	if err != nil || !found || restored.ID != first.ID {
		t.Fatalf("restored session = (%+v, %t, %v)", restored, found, err)
	}
	rotated, found, err := restartedStore.Rotate(ctx, first.RefreshTokenHash, second)
	if err != nil || !found || rotated.ID != first.ID {
		t.Fatalf("first refresh rotation = (%+v, %t, %v)", rotated, found, err)
	}
	if _, found, err := restartedStore.Rotate(ctx, first.RefreshTokenHash, replayCandidate); err != nil || found {
		t.Fatalf("replayed refresh rotation = (found %t, error %v), want single consumption", found, err)
	}

	afterRestart, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("post-rotation session store constructor: %v", err)
	}
	if restored, found, err := afterRestart.GetByRefreshTokenHash(ctx, second.RefreshTokenHash); err != nil || !found || restored.ID != second.ID {
		t.Fatalf("rotated session after restart = (%+v, %t, %v)", restored, found, err)
	}
	if _, found, err := afterRestart.GetByRefreshTokenHash(ctx, first.RefreshTokenHash); err != nil || found {
		t.Fatalf("revoked refresh after restart = (found %t, error %v)", found, err)
	}
	if _, found, err := afterRestart.GetByRefreshTokenHash(ctx, replayCandidate.RefreshTokenHash); err != nil || found {
		t.Fatalf("replay candidate persisted = (found %t, error %v)", found, err)
	}
}

func TestSQLSessionStoreReconcilesPreDeviceActivationBinding(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("binding_user_%d", suffix)
	session := AuthSession{
		ID:               fmt.Sprintf("binding_session_%d", suffix),
		UserID:           userID,
		Product:          controlplane.ProductAutoLive,
		AccessTokenHash:  fmt.Sprintf("binding_access_%d", suffix),
		RefreshTokenHash: fmt.Sprintf("binding_refresh_%d", suffix),
		AccessExpiresAt:  now.Add(time.Hour),
		RefreshExpiresAt: now.Add(24 * time.Hour),
		CreatedAt:        now,
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, fmt.Sprintf("binding-user-%d", suffix), "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed binding user: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	sessions, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("session store constructor: %v", err)
	}
	if err := sessions.Create(ctx, session); err != nil {
		t.Fatalf("create session: %v", err)
	}
	// Activation binds the session before the device row exists; migration
	// 0018 intentionally makes this provisional state recoverable.
	if err := sessions.UpdateDeviceID(ctx, session.AccessTokenHash, "device_pending_activation"); err != nil {
		t.Fatalf("pre-device binding: %v", err)
	}
	deleted, err := sessions.CleanupOrphanedDeviceBindings(ctx, RetentionCleanupRequest{Cutoff: now.Add(time.Minute), BatchSize: 10})
	if err != nil || deleted != 1 {
		t.Fatalf("orphan binding cleanup = (%d, %v), want (1, nil)", deleted, err)
	}
	restored, found, err := sessions.GetByAccessTokenHash(ctx, session.AccessTokenHash)
	if err != nil || !found || restored.DeviceID != "" {
		t.Fatalf("restored session after reconciliation = (%+v, %t, %v)", restored, found, err)
	}
}

func TestPostgresRepositorySessionBindingRollsBackWithDomainFailure(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("tx_binding_user_%d", suffix)
	session := AuthSession{
		ID: fmt.Sprintf("tx_binding_session_%d", suffix), UserID: userID,
		Product:         controlplane.ProductAutoLive,
		AccessTokenHash: fmt.Sprintf("tx_binding_access_%d", suffix), RefreshTokenHash: fmt.Sprintf("tx_binding_refresh_%d", suffix),
		AccessExpiresAt: now.Add(time.Hour), RefreshExpiresAt: now.Add(24 * time.Hour), CreatedAt: now,
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, fmt.Sprintf("tx-binding-user-%d", suffix), "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed transaction user: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})
	sessions, err := NewSQLSessionStore(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("session store constructor: %v", err)
	}
	if err := sessions.Create(ctx, session); err != nil {
		t.Fatalf("create session: %v", err)
	}
	repository, err := NewPostgresRepository(database, func() time.Time { return now })
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	domainFailure := errors.New("domain operation failed")
	err = repository.RunWithSessionBinding(ctx, session.AccessTokenHash, userID, "device_transactional", func(*State) error {
		return domainFailure
	})
	if !errors.Is(err, domainFailure) {
		t.Fatalf("RunWithSessionBinding() error = %v, want domain failure", err)
	}
	restored, found, err := sessions.GetByAccessTokenHash(ctx, session.AccessTokenHash)
	if err != nil || !found || restored.DeviceID != "" {
		t.Fatalf("session after domain rollback = (%+v, %t, %v), want unbound", restored, found, err)
	}
}

func TestPostgresNormalizedSecretRotationPersistsAcrossRepositoryRestart(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	accountID := fmt.Sprintf("rotation_account_%d", suffix)
	oldSecretRef := "model-account/" + accountID
	idempotencyKey := fmt.Sprintf("rotation-integration:%d", suffix)
	auditRequestID := fmt.Sprintf("rotation-audit:%d", suffix)
	newAPIKey := "integration-new-secret"
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("secret store constructor: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "test-model", "https://provider.example.test", oldSecretRef,
		controlplane.ModelAccountStatusActive, 1, 2, 100, now); err != nil {
		t.Fatalf("seed model account: %v", err)
	}
	if err := secretStore.Put(ctx, oldSecretRef, "integration-old-secret"); err != nil {
		t.Fatalf("seed model account secret: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_pool_test_results WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref LIKE $1`, oldSecretRef+"/%")
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, oldSecretRef)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "model-account-secret-rotation", idempotencyKey)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
	})
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	result, err := repository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
		Scope:             "model-account-secret-rotation",
		IdempotencyKey:    idempotencyKey,
		Fingerprint:       "integration-fingerprint",
		AccountID:         accountID,
		ExpectedSecretRef: oldSecretRef,
		APIKey:            newAPIKey,
		Probe: controlplane.ModelPoolConnectivityTestResult{
			AccountID: accountID, Provider: "integration", Model: "test-model", Status: "succeeded",
			TestedAt: now.Format(time.RFC3339), LatencyMS: 1, HTTPStatus: 200,
		},
		Audit: controlplane.AuditLogInput{
			Action: "model_account.rotate_secret", TargetType: "model_account", TargetID: accountID,
			Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
		},
	})
	if err != nil {
		t.Fatalf("normalized secret rotation: %v", err)
	}
	if result.ID != accountID || !result.SecretConfigured || result.LastTestStatus != "succeeded" {
		t.Fatalf("rotation result = %+v", result)
	}

	var activeSecretRef string
	if err := database.QueryRowContext(ctx, `SELECT secret_ref FROM model_accounts WHERE id = $1`, accountID).Scan(&activeSecretRef); err != nil {
		t.Fatalf("read active secret reference: %v", err)
	}
	if !strings.HasPrefix(activeSecretRef, oldSecretRef+"/rotation_") {
		t.Fatalf("active secret reference = %q, want staged rotation reference", activeSecretRef)
	}
	if stored, err := secretStore.Get(ctx, activeSecretRef); err != nil || stored != newAPIKey {
		t.Fatalf("new secret after rotation = (%q, %v)", stored, err)
	}
	if _, err := secretStore.Get(ctx, oldSecretRef); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("old secret lookup error = %v, want ErrSecretNotFound", err)
	}

	restartedSecretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("restarted secret store constructor: %v", err)
	}
	restartedRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, restartedSecretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted repository constructor: %v", err)
	}
	if replay, err := restartedRepository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
		Scope: "model-account-secret-rotation", IdempotencyKey: idempotencyKey, Fingerprint: "integration-fingerprint",
		AccountID: accountID, ExpectedSecretRef: oldSecretRef, APIKey: "ignored-replay-secret",
		Probe: controlplane.ModelPoolConnectivityTestResult{AccountID: accountID, Status: "succeeded"},
	}); err != nil || replay.ID != accountID {
		t.Fatalf("idempotent rotation replay = (%+v, %v)", replay, err)
	}
	var outboxCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID).Scan(&outboxCount); err != nil {
		t.Fatalf("read audit outbox: %v", err)
	}
	if outboxCount != 1 {
		t.Fatalf("audit outbox rows = %d, want 1", outboxCount)
	}
}

func TestPostgresNormalizedSecretRotationRollsBackStagedCiphertext(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	accountID := fmt.Sprintf("rotation_rollback_account_%d", suffix)
	oldSecretRef := "model-account/" + accountID
	idempotencyKey := fmt.Sprintf("rotation-rollback:%d", suffix)
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("secret store constructor: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "test-model", "https://provider.example.test", oldSecretRef,
		controlplane.ModelAccountStatusActive, 1, 2, 100, now); err != nil {
		t.Fatalf("seed rollback model account: %v", err)
	}
	if err := secretStore.Put(ctx, oldSecretRef, "integration-old-secret"); err != nil {
		t.Fatalf("seed rollback model account secret: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_pool_test_results WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref LIKE $1`, oldSecretRef+"/%")
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, oldSecretRef)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "model-account-secret-rotation", idempotencyKey)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
	})
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, secretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("rollback repository constructor: %v", err)
	}
	_, err = repository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
		Scope: "model-account-secret-rotation", IdempotencyKey: idempotencyKey, Fingerprint: "rollback-fingerprint",
		AccountID: accountID, ExpectedSecretRef: oldSecretRef, APIKey: "integration-rollback-secret",
		Probe: controlplane.ModelPoolConnectivityTestResult{AccountID: accountID, Status: "succeeded", TestedAt: "not-a-rfc3339-time"},
	})
	if err == nil {
		t.Fatal("rotation with invalid probe timestamp error = nil")
	}
	var activeSecretRef string
	if err := database.QueryRowContext(ctx, `SELECT secret_ref FROM model_accounts WHERE id = $1`, accountID).Scan(&activeSecretRef); err != nil {
		t.Fatalf("read rollback secret reference: %v", err)
	}
	if activeSecretRef != oldSecretRef {
		t.Fatalf("secret reference after rollback = %q, want %q", activeSecretRef, oldSecretRef)
	}
	if _, err := secretStore.Get(ctx, oldSecretRef); err != nil {
		t.Fatalf("old secret after rollback: %v", err)
	}
	var stagedCount, testCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_account_secrets WHERE secret_ref LIKE $1`, oldSecretRef+"/%").Scan(&stagedCount); err != nil {
		t.Fatalf("count staged secrets after rollback: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_pool_test_results WHERE account_id = $1`, accountID).Scan(&testCount); err != nil {
		t.Fatalf("count test results after rollback: %v", err)
	}
	if stagedCount != 0 || testCount != 0 {
		t.Fatalf("rollback residue = staged %d, test results %d", stagedCount, testCount)
	}
}

func TestPostgresNormalizedModelLeaseLazyExpiryReusesConcurrencySlot(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Date(2026, 8, 22, 10, 0, 0, 0, time.UTC)
	fixture := newNormalizedModelLeaseIntegrationFixture(t, database, ctx, func() time.Time { return now })
	accountID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusActive, 1, 1, 0, nil)

	first, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "lazy-expiry:first", Fingerprint: "lazy-expiry:first",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("create first lease: %v", err)
	}
	now = now.Add(31 * time.Second)
	second, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "lazy-expiry:second", Fingerprint: "lazy-expiry:second",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 30,
	})
	if err != nil {
		t.Fatalf("create lease after expiry: %v", err)
	}
	if second.AccountID != accountID || second.Status != controlplane.ModelLeaseStatusActive {
		t.Fatalf("lease after lazy expiry = %+v, want active lease on %q", second, accountID)
	}

	var status string
	var releasedAt sql.NullTime
	if err := database.QueryRowContext(ctx, `SELECT status, released_at FROM model_leases WHERE id = $1`, first.ID).Scan(&status, &releasedAt); err != nil {
		t.Fatalf("read lazily expired lease: %v", err)
	}
	if status != controlplane.ModelLeaseStatusExpired || !releasedAt.Valid || !releasedAt.Time.UTC().Equal(now.UTC()) {
		t.Fatalf("lazily expired lease = status %q released_at %v, want expired at %s", status, releasedAt, now.Format(time.RFC3339))
	}
}

func TestPostgresNormalizedModelLeaseSkipsCooldownAndDisabledAccounts(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Date(2026, 8, 22, 11, 0, 0, 0, time.UTC)
	fixture := newNormalizedModelLeaseIntegrationFixture(t, database, ctx, func() time.Time { return now })
	disabledID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusDisabled, 3, 1, 0, nil)
	cooldownUntil := now.Add(5 * time.Minute)
	cooldownID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusCooldown, 2, 1, 0, &cooldownUntil)
	activeID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusActive, 1, 1, 0, nil)

	first, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "account-state:first", Fingerprint: "account-state:first",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 60,
	})
	if err != nil {
		t.Fatalf("create lease with unavailable higher-priority accounts: %v", err)
	}
	if first.AccountID != activeID {
		t.Fatalf("first selected account = %q, want active fallback %q", first.AccountID, activeID)
	}

	var disabledStatus, cooldownStatus string
	if err := database.QueryRowContext(ctx, `SELECT status FROM model_accounts WHERE id = $1`, disabledID).Scan(&disabledStatus); err != nil {
		t.Fatalf("read disabled account status: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT status FROM model_accounts WHERE id = $1`, cooldownID).Scan(&cooldownStatus); err != nil {
		t.Fatalf("read cooldown account status: %v", err)
	}
	if disabledStatus != controlplane.ModelAccountStatusDisabled || cooldownStatus != controlplane.ModelAccountStatusCooldown {
		t.Fatalf("unavailable account statuses = disabled:%q cooldown:%q", disabledStatus, cooldownStatus)
	}

	now = now.Add(6 * time.Minute)
	second, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "account-state:second", Fingerprint: "account-state:second",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 60,
	})
	if err != nil {
		t.Fatalf("create lease after cooldown: %v", err)
	}
	if second.AccountID != cooldownID {
		t.Fatalf("account selected after cooldown = %q, want %q", second.AccountID, cooldownID)
	}
	var refreshedStatus string
	var refreshedCooldown sql.NullTime
	if err := database.QueryRowContext(ctx, `SELECT status, cooldown_until FROM model_accounts WHERE id = $1`, cooldownID).Scan(&refreshedStatus, &refreshedCooldown); err != nil {
		t.Fatalf("read refreshed cooldown account: %v", err)
	}
	if refreshedStatus != controlplane.ModelAccountStatusActive || refreshedCooldown.Valid {
		t.Fatalf("refreshed cooldown account = status:%q cooldown:%v, want active/NULL", refreshedStatus, refreshedCooldown)
	}
}

func TestPostgresNormalizedModelLeaseReleaseRestoresConcurrencySlot(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	fixture := newNormalizedModelLeaseIntegrationFixture(t, database, ctx, func() time.Time { return now })
	accountID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusActive, 1, 1, 0, nil)

	first, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "release-slot:first", Fingerprint: "release-slot:first",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("create first lease: %v", err)
	}
	if _, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "release-slot:blocked", Fingerprint: "release-slot:blocked",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 300,
	}); !controlplane.IsErrorCode(err, controlplane.ErrModelPoolUnavailable.Code) {
		t.Fatalf("create lease while slot is occupied = %v, want MODEL_POOL_UNAVAILABLE", err)
	}

	result, err := fixture.repository.ReleaseModelLease(ctx, ModelLeaseReleaseRecord{
		Scope: fixture.scope, IdempotencyKey: "release-slot:release", Fingerprint: "release-slot:release",
		UserID: fixture.userID, DeviceID: fixture.deviceID, LeaseID: first.ID, Reason: "integration release",
	})
	if err != nil || !result.Released {
		t.Fatalf("release occupied lease = %+v, %v", result, err)
	}
	second, err := fixture.repository.CreateModelLease(ctx, ModelLeaseCreateRecord{
		Scope: fixture.scope, IdempotencyKey: "release-slot:second", Fingerprint: "release-slot:second",
		UserID: fixture.userID, DeviceID: fixture.deviceID, Provider: "integration", Model: "lease-edge-model", Purpose: "integration", MaxDurationSeconds: 300,
	})
	if err != nil {
		t.Fatalf("create lease after release: %v", err)
	}
	if second.AccountID != accountID || second.ID == first.ID {
		t.Fatalf("lease after release = %+v, want new lease on %q", second, accountID)
	}
	var status string
	if err := database.QueryRowContext(ctx, `SELECT status FROM model_leases WHERE id = $1`, first.ID).Scan(&status); err != nil {
		t.Fatalf("read released lease status: %v", err)
	}
	if status != controlplane.ModelLeaseStatusReleased {
		t.Fatalf("released lease status = %q, want released", status)
	}
}

type normalizedModelLeaseIntegrationFixture struct {
	database      *sql.DB
	repository    *PostgresRepository
	secretStore   *EncryptedSQLSecretStore
	now           func() time.Time
	scope         string
	userID        string
	deviceID      string
	accountPrefix string
	accountIndex  int
}

func newNormalizedModelLeaseIntegrationFixture(t *testing.T, database *sql.DB, ctx context.Context, now func() time.Time) *normalizedModelLeaseIntegrationFixture {
	t.Helper()
	suffix := time.Now().UTC().UnixNano()
	fixture := &normalizedModelLeaseIntegrationFixture{
		database: database, now: now,
		scope:  fmt.Sprintf("integration-model-lease-edge-%d", suffix),
		userID: fmt.Sprintf("lease_edge_user_%d", suffix), deviceID: fmt.Sprintf("lease_edge_device_%d", suffix),
		accountPrefix: fmt.Sprintf("lease_edge_account_%d", suffix),
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		pattern := fixture.accountPrefix + "%"
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_usage_records WHERE account_id LIKE $1`, pattern)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_leases WHERE account_id LIKE $1`, pattern)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1`, fixture.scope)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref LIKE $1`, "model-account/"+pattern)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id LIKE $1`, pattern)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, fixture.deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, fixture.userID)
	})
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, fixture.userID, fixture.userID, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now()); err != nil {
		t.Fatalf("seed lease edge user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, client_version, status, device_name, platform)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
	`, fixture.deviceID, fixture.userID, fixture.deviceID+"-key", "integration", controlplane.DeviceStatusActive, "lease-edge-device", "test"); err != nil {
		t.Fatalf("seed lease edge device: %v", err)
	}
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("lease edge secret store constructor: %v", err)
	}
	fixture.secretStore = secretStore
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, now, secretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("lease edge repository constructor: %v", err)
	}
	fixture.repository = repository
	return fixture
}

func (f *normalizedModelLeaseIntegrationFixture) addAccount(t *testing.T, ctx context.Context, status string, priority, concurrencyLimit, dailyLimit int, cooldownUntil *time.Time) string {
	t.Helper()
	accountID := fmt.Sprintf("%s_%d", f.accountPrefix, f.accountIndex)
	f.accountIndex++
	secretRef := "model-account/" + accountID
	var cooldownValue any
	if cooldownUntil != nil {
		cooldownValue = cooldownUntil.UTC()
	}
	if _, err := f.database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, cooldown_until, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, 0, $11, $11)
	`, accountID, "integration", "lease-edge-model", "https://provider.example.test", secretRef, status, priority, concurrencyLimit, dailyLimit, cooldownValue, f.now()); err != nil {
		t.Fatalf("seed lease edge account %q: %v", accountID, err)
	}
	if err := f.secretStore.Put(ctx, secretRef, "integration-lease-edge-secret"); err != nil {
		t.Fatalf("seed lease edge secret %q: %v", secretRef, err)
	}
	return accountID
}

func TestPostgresProductCompatibilityDefaultsLegacyWritesToAutolive(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)

	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	suffix := time.Now().UTC().UnixNano()
	userID := fmt.Sprintf("product_scope_user_%d", suffix)
	deviceID := fmt.Sprintf("product_scope_device_%d", suffix)
	sessionID := fmt.Sprintf("session_product_scope_%d", suffix)
	accessTokenHash := fmt.Sprintf("access_product_scope_%d", suffix)
	refreshTokenHash := fmt.Sprintf("refresh_product_scope_%d", suffix)
	deviceKey := "state-device/" + deviceID

	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM auth_sessions WHERE id = $1`, sessionID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM user_products WHERE user_id = $1`, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, userID, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed product isolation user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, device_name, platform, client_version, status, last_heartbeat_at)
		VALUES ($1, $2, $3, 'AutoLive Device', 'windows', 'integration', $4, $5)
	`, deviceID, userID, deviceKey, controlplane.DeviceStatusActive, now); err != nil {
		t.Fatalf("seed legacy-shaped device: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO auth_sessions (
			id, user_id, device_id, access_token_hash, refresh_token_hash,
			access_expires_at, refresh_expires_at, created_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
	`, sessionID, userID, deviceID, accessTokenHash, refreshTokenHash, now.Add(time.Hour), now.Add(24*time.Hour), now); err != nil {
		t.Fatalf("seed legacy-shaped auth session: %v", err)
	}

	var membershipCount int
	if err := database.QueryRowContext(ctx, `
		SELECT COUNT(*)
		FROM user_products
		WHERE user_id = $1 AND product = $2
	`, userID, controlplane.ProductAutoLive).Scan(&membershipCount); err != nil {
		t.Fatalf("count default autolive membership: %v", err)
	}
	if membershipCount != 1 {
		t.Fatalf("autolive membership count = %d, want 1", membershipCount)
	}

	var storedDeviceProduct, storedSessionProduct, storedDeviceID string
	if err := database.QueryRowContext(ctx, `
		SELECT product
		FROM devices
		WHERE id = $1
	`, deviceID).Scan(&storedDeviceProduct); err != nil {
		t.Fatalf("read defaulted device product: %v", err)
	}
	if err := database.QueryRowContext(ctx, `
		SELECT product, device_id
		FROM auth_sessions
		WHERE id = $1
	`, sessionID).Scan(&storedSessionProduct, &storedDeviceID); err != nil {
		t.Fatalf("read defaulted auth session product: %v", err)
	}
	if storedDeviceProduct != string(controlplane.ProductAutoLive) || storedSessionProduct != string(controlplane.ProductAutoLive) || storedDeviceID != deviceID {
		t.Fatalf("legacy defaulted scope = (%q, %q, %q), want (%q, %q, %q)", storedDeviceProduct, storedSessionProduct, storedDeviceID, controlplane.ProductAutoLive, controlplane.ProductAutoLive, deviceID)
	}
}

func openPostgresIntegrationDatabase(t *testing.T) (*sql.DB, context.Context) {
	t.Helper()
	databaseURL := os.Getenv("TEST_POSTGRES_URL")
	if databaseURL == "" {
		t.Skip("TEST_POSTGRES_URL is not set")
	}
	database, err := sql.Open("postgres", databaseURL)
	if err != nil {
		t.Fatalf("sql.Open() error = %v", err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(func() {
		cancel()
		_ = database.Close()
	})
	if err := database.PingContext(ctx); err != nil {
		t.Fatalf("database ping error = %v", err)
	}
	if err := migrationscheck.ValidateApplied(ctx, database); err != nil {
		t.Fatalf("migration check error = %v", err)
	}
	return database, ctx
}
