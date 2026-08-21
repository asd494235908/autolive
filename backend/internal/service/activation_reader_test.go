package service

import (
	"context"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedActivationPageRepository struct {
	*store.MemoryStore
	called bool
}

func (r *normalizedActivationPageRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedActivationPageRepository) ListActivationCodesPage(context.Context, int, int) (store.ActivationCodePage, error) {
	r.called = true
	return store.ActivationCodePage{
		Total: 1,
		Items: []controlplane.ActivationCode{{ID: "ac_1", Status: controlplane.ActivationCodeStatusActive, MaxDevices: 1}},
	}, nil
}

func TestListActivationCodesPageUsesNormalizedReader(t *testing.T) {
	repository := &normalizedActivationPageRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	items, total, err := NewControlPlaneWithRepository(repository).ListActivationCodesPage(context.Background(), 1, 20)
	if err != nil {
		t.Fatalf("ListActivationCodesPage() error = %v", err)
	}
	if !repository.called || total != 1 || len(items) != 1 || items[0].ID != "ac_1" {
		t.Fatalf("page called:%t total:%d items:%+v", repository.called, total, items)
	}
}
