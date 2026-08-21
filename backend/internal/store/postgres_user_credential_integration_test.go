//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"testing"
	"time"

	"golang.org/x/crypto/bcrypt"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedAuthenticationReadsUserCredentialWithoutSnapshot(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Second)
	suffix := now.UnixNano()
	userID := fmt.Sprintf("auth_user_%d", suffix)
	username := fmt.Sprintf("auth-user-%d", suffix)
	password := "normalized-password"
	hash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.MinCost)
	if err != nil {
		t.Fatalf("bcrypt.GenerateFromPassword() error = %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM users WHERE id = $1`, userID)
	})
	if _, err := database.ExecContext(ctx, `
		INSERT INTO users (id, username, password_hash, role, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6)
	`, userID, username, hash, controlplane.RoleUser, controlplane.UserStatusActive, now); err != nil {
		t.Fatalf("seed normalized auth user: %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	user, storedHash, err := repository.GetUserCredential(ctx, username)
	if err != nil {
		t.Fatalf("GetUserCredential() error = %v", err)
	}
	if user.ID != userID || string(storedHash) != string(hash) {
		t.Fatalf("credential row = user:%+v hash_match:%t", user, string(storedHash) == string(hash))
	}
	byID, err := repository.GetUserByID(ctx, userID)
	if err != nil || byID.ID != userID {
		t.Fatalf("GetUserByID() = user:%+v err:%v", byID, err)
	}
}
