//go:build postgres_integration

package store

import (
	"context"
	"fmt"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedActivationPageReadsRedactedRows(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Second)
	suffix := now.UnixNano()
	expiredID := fmt.Sprintf("activation_reader_expired_%d", suffix)
	activeID := fmt.Sprintf("activation_reader_active_%d", suffix)
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM activation_codes WHERE id IN ($1, $2)`, expiredID, activeID)
	})
	if _, err := database.ExecContext(ctx, `
		INSERT INTO activation_codes (id, code_hash, code_prefix, status, created_at, expires_at)
		VALUES ($1, $2, $3, $4, $5, $6), ($7, $8, $9, $10, $11, $12)
	`, expiredID, "hash/"+expiredID, "code_expired", controlplane.ActivationCodeStatusActive, now.Add(-2*time.Hour), now.Add(-time.Hour),
		activeID, "hash/"+activeID, "code_active", controlplane.ActivationCodeStatusActive, now, now.Add(time.Hour)); err != nil {
		t.Fatalf("seed activation codes: %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor error = %v", err)
	}
	page, err := repository.ListActivationCodesPage(ctx, 0, 20)
	if err != nil {
		t.Fatalf("ListActivationCodesPage() error = %v", err)
	}
	var expired, active *controlplane.ActivationCode
	for index := range page.Items {
		item := &page.Items[index]
		switch item.ID {
		case expiredID:
			expired = item
		case activeID:
			active = item
		}
	}
	if page.Total < 2 || expired == nil || active == nil || expired.Status != controlplane.ActivationCodeStatusExpired || active.Status != controlplane.ActivationCodeStatusActive {
		t.Fatalf("activation page total:%d expired:%+v active:%+v", page.Total, expired, active)
	}
	for _, item := range page.Items {
		if item.ID == expiredID || item.ID == activeID {
			if item.PlainCode != nil || item.MaxDevices != 1 {
				t.Fatalf("activation row leaked or has wrong max devices: %+v", item)
			}
		}
	}
}
