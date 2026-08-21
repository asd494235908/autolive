//go:build postgres_integration

package store

import (
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedModelPoolTestPersistsResultAndReplay(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Second)
	fixture := newNormalizedModelLeaseIntegrationFixture(t, database, ctx, func() time.Time { return now })
	accountID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusActive, 1, 1, 1000, nil)
	key := fmt.Sprintf("model-test-%d", time.Now().UTC().UnixNano())
	fingerprint := "model-test-fingerprint"
	record := ModelPoolTestPrepareRecord{Scope: fixture.scope, IdempotencyKey: key, Fingerprint: fingerprint, AccountID: accountID}
	preparation, err := fixture.repository.PrepareModelPoolAccountTest(ctx, record)
	if err != nil {
		t.Fatalf("prepare normalized model test: %v", err)
	}
	if preparation.Cached != nil || preparation.Account.SecretRef == "" {
		t.Fatalf("initial preparation = %+v", preparation)
	}
	result := controlplane.ModelPoolConnectivityTestResult{AccountID: accountID, Provider: "integration", Model: "lease-edge-model", Status: "failed", TestedAt: now.Format(time.RFC3339), ErrorCode: "PROVIDER_HTTP_ERROR"}
	stored, err := fixture.repository.RecordModelPoolAccountTest(ctx, ModelPoolTestRecord{Scope: fixture.scope, IdempotencyKey: key, Fingerprint: fingerprint, AccountID: accountID, Result: result})
	if err != nil {
		t.Fatalf("record normalized model test: %v", err)
	}
	if stored != result {
		t.Fatalf("stored result = %+v, want %+v", stored, result)
	}
	var status string
	var cooldown time.Time
	if err := database.QueryRowContext(ctx, `SELECT status, cooldown_until FROM model_accounts WHERE id = $1`, accountID).Scan(&status, &cooldown); err != nil {
		t.Fatalf("read normalized model account state: %v", err)
	}
	if status != controlplane.ModelAccountStatusCooldown || !cooldown.Equal(now.Add(normalizedModelAccountCooldownDuration)) {
		t.Fatalf("model account after failed test = status:%q cooldown:%s", status, cooldown)
	}
	replay, err := fixture.repository.PrepareModelPoolAccountTest(ctx, record)
	if err != nil {
		t.Fatalf("prepare idempotent normalized model test: %v", err)
	}
	if replay.Cached == nil || *replay.Cached != result {
		t.Fatalf("replayed result = %+v, want %+v", replay.Cached, result)
	}
}

func TestPostgresNormalizedModelPoolHealthReaderFiltersBeforeLimit(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Second)
	fixture := newNormalizedModelLeaseIntegrationFixture(t, database, ctx, func() time.Time { return now })
	fixture.addAccount(t, ctx, controlplane.ModelAccountStatusDisabled, 1, 1, 0, nil)
	futureCooldown := now.Add(10 * time.Minute)
	fixture.addAccount(t, ctx, controlplane.ModelAccountStatusCooldown, 1, 1, 0, &futureCooldown)
	readyID := fixture.addAccount(t, ctx, controlplane.ModelAccountStatusActive, 1, 1, 0, nil)

	items, err := fixture.repository.ListModelPoolHealthAccounts(ctx, 1)
	if err != nil {
		t.Fatalf("ListModelPoolHealthAccounts() error = %v", err)
	}
	if len(items) != 1 || items[0].ID != readyID {
		t.Fatalf("health accounts = %+v, want only eligible account %q", items, readyID)
	}
}
