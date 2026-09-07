//go:build postgres_integration

package store

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresClientSyncRoundTripIdempotencyCASAndIsolation(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("sync_user_%d", suffix)
	deviceID := fmt.Sprintf("sync_device_%d", suffix)
	activationID := fmt.Sprintf("sync_activation_%d", suffix)
	username := fmt.Sprintf("sync-user-%d", suffix)
	product := controlplane.ProductDouyinDesktop

	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, "$2a$10$integration-hash", controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed sync user: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
		VALUES ($1, $2, 'active', 0, $3, $3)
	`, userID, product, now); err != nil {
		t.Fatalf("seed sync membership: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, product, device_key, client_version, status, device_name, platform)
		VALUES ($1, $2, $3, $4, 'integration', 'active', 'sync device', 'test')
	`, deviceID, userID, product, "sync-key-"+deviceID); err != nil {
		t.Fatalf("seed sync device: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO activation_codes (id, product, bound_user_id, code_hash, code_prefix, status, created_at, expires_at, max_devices, bound_devices)
		VALUES ($1, $2, $3, $4, 'sync_code', 'used', $5, $6, 1, 1)
	`, activationID, product, userID, "sync-hash-"+activationID, now, now.Add(time.Hour)); err != nil {
		t.Fatalf("seed sync activation: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO activation_device_bindings (activation_code_id, device_id, product, user_id, bound_at)
		VALUES ($1, $2, $3, $4, $5)
	`, activationID, deviceID, product, userID, now); err != nil {
		t.Fatalf("seed sync device binding: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM client_sync_mutations WHERE product = $1 AND user_id = $2`, product, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM client_sync_items WHERE product = $1 AND user_id = $2`, product, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM client_sync_workspaces WHERE product = $1 AND user_id = $2`, product, userID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_device_bindings WHERE activation_code_id = $1`, activationID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_codes WHERE id = $1`, activationID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, deviceID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM user_products WHERE user_id = $1 AND product = $2`, userID, product)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("sync repository constructor: %v", err)
	}
	scope := ClientSyncScope{Product: product, UserID: userID, DeviceID: deviceID}
	mutation := controlplane.ClientSyncMutation{
		MutationID: "mutation-1", Kind: controlplane.ClientSyncKindPersonaVersion, ItemID: "global-persona-v1",
		Payload: json.RawMessage(`{"version":1,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`),
	}
	created, err := repository.WriteClientSyncItems(ctx, scope, []controlplane.ClientSyncMutation{mutation})
	if err != nil || created.ServerRevision != 1 || created.Items[0].Revision != 1 {
		t.Fatalf("create sync item = (%+v, %v)", created, err)
	}

	restarted, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("restarted sync repository constructor: %v", err)
	}
	replayed, err := restarted.WriteClientSyncItems(ctx, scope, []controlplane.ClientSyncMutation{mutation})
	if err != nil || len(replayed.Items) != 1 || replayed.Items[0] != created.Items[0] {
		t.Fatalf("replay sync item = (%+v, %v), want %+v", replayed, err, created)
	}
	reusedID := mutation
	reusedID.Payload = json.RawMessage(`{"version":1,"content":{"tone":"different"},"created_at":"2026-09-05T01:02:03Z"}`)
	if _, err := restarted.WriteClientSyncItems(ctx, scope, []controlplane.ClientSyncMutation{reusedID}); !errors.Is(err, controlplane.ErrClientSyncMutationConflict) {
		t.Fatalf("reused mutation ID error = %v, want mutation conflict", err)
	}

	conflicting := mutation
	conflicting.MutationID = "mutation-2"
	if _, err := restarted.WriteClientSyncItems(ctx, scope, []controlplane.ClientSyncMutation{conflicting}); !errors.Is(err, controlplane.ErrClientSyncConflict) {
		t.Fatalf("stale create error = %v, want sync conflict", err)
	}
	if _, err := restarted.ListClientSyncItems(ctx, ClientSyncScope{Product: product, UserID: "other-user", DeviceID: deviceID}, 0, 10); !errors.Is(err, controlplane.ErrDeviceBindingRequired) {
		t.Fatalf("cross-user list error = %v, want device binding required", err)
	}
	if _, err := restarted.ListClientSyncItems(ctx, ClientSyncScope{Product: product, UserID: userID, DeviceID: "unbound-device"}, 0, 10); !errors.Is(err, controlplane.ErrDeviceBindingRequired) {
		t.Fatalf("cross-device list error = %v, want device binding required", err)
	}

	tombstone := controlplane.ClientSyncMutation{MutationID: "mutation-3", Kind: mutation.Kind, ItemID: mutation.ItemID, BaseRevision: 1, Deleted: true}
	deleted, err := restarted.WriteClientSyncItems(ctx, scope, []controlplane.ClientSyncMutation{tombstone})
	if err != nil || deleted.ServerRevision != 2 || !deleted.Items[0].Deleted {
		t.Fatalf("delete sync item = (%+v, %v)", deleted, err)
	}
	page, err := restarted.ListClientSyncItems(ctx, scope, 1, 10)
	if err != nil || len(page.Items) != 1 || !page.Items[0].Deleted || page.NextCursor != 2 || page.ServerRevision != 2 {
		t.Fatalf("tombstone page = (%+v, %v)", page, err)
	}
	if _, err := database.ExecContext(ctx, `UPDATE activation_codes SET expires_at = $2 WHERE id = $1`, activationID, now); err != nil {
		t.Fatalf("expire sync activation: %v", err)
	}
	if _, err := restarted.ListClientSyncItems(ctx, scope, 0, 10); !errors.Is(err, controlplane.ErrDeviceBindingRequired) {
		t.Fatalf("expired authorization error = %v, want device binding required", err)
	}
}
