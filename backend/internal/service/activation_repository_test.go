package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedActivationWriterRepository struct {
	*store.MemoryStore
	called       bool
	record       store.ActivationCodeCreateRecord
	revokeCalled bool
	revokeCodeID string
}

func (r *normalizedActivationWriterRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedActivationWriterRepository) CreateActivationCode(_ context.Context, _, _, _ string, record store.ActivationCodeCreateRecord) (controlplane.ActivationCode, error) {
	r.called = true
	r.record = record
	plainCode := record.PlainCode
	return controlplane.ActivationCode{ID: "ac_direct", Status: controlplane.ActivationCodeStatusActive, ExpiresAt: record.ExpiresAt.UTC().Format(time.RFC3339), MaxDevices: record.MaxDevices, CodePrefix: record.CodePrefix, PlainCode: &plainCode}, nil
}

func (r *normalizedActivationWriterRepository) RevokeActivationCode(_ context.Context, _, _, _, codeID string) (controlplane.ActivationCode, error) {
	r.revokeCalled = true
	r.revokeCodeID = codeID
	return controlplane.ActivationCode{ID: codeID, Status: controlplane.ActivationCodeStatusRevoked, MaxDevices: 1}, nil
}

func TestCreateActivationCodeUsesNormalizedRepositoryWriter(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &normalizedActivationWriterRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	code, err := svc.CreateActivationCode(context.Background(), "activation-key", controlplane.CreateActivationCodeInput{ExpiresAt: now.Add(time.Hour), MaxDevices: 1})
	if err != nil {
		t.Fatalf("CreateActivationCode() error = %v", err)
	}
	if !repository.called || code.ID != "ac_direct" || repository.record.CodeHash == "" || repository.record.PlainCode == "" || repository.record.CodePrefix == "" {
		t.Fatalf("normalized activation writer = called %t code %+v record %+v", repository.called, code, repository.record)
	}
}

func TestRevokeActivationCodeUsesNormalizedRepositoryWriter(t *testing.T) {
	repository := &normalizedActivationWriterRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	code, err := svc.RevokeActivationCode(context.Background(), "revoke-key", "ac_1")
	if err != nil {
		t.Fatalf("RevokeActivationCode() error = %v", err)
	}
	if !repository.revokeCalled || repository.revokeCodeID != "ac_1" || code.ID != "ac_1" || code.Status != controlplane.ActivationCodeStatusRevoked {
		t.Fatalf("normalized revoke writer = called %t code %+v id %q", repository.revokeCalled, code, repository.revokeCodeID)
	}
}
