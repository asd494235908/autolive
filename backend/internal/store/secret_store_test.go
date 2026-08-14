package store

import (
	"context"
	"errors"
	"testing"
)

func TestMemorySecretStoreRoundTripAndContextBoundary(t *testing.T) {
	secretStore := NewMemorySecretStore()
	ctx := context.Background()
	if err := secretStore.Put(ctx, "model-account/mpa_1", "sk-test-secret"); err != nil {
		t.Fatalf("Put() error = %v", err)
	}
	value, err := secretStore.Get(ctx, "model-account/mpa_1")
	if err != nil {
		t.Fatalf("Get() error = %v", err)
	}
	if value != "sk-test-secret" {
		t.Fatalf("Get() value = %q, want secret", value)
	}

	if err := secretStore.Delete(ctx, "model-account/mpa_1"); err != nil {
		t.Fatalf("Delete() error = %v", err)
	}
	if _, err := secretStore.Get(ctx, "model-account/mpa_1"); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("Get() after Delete() error = %v, want ErrSecretNotFound", err)
	}

	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	if err := secretStore.Put(cancelled, "model-account/mpa_2", "sk-test-secret"); !errors.Is(err, context.Canceled) {
		t.Fatalf("Put() cancelled error = %v, want context.Canceled", err)
	}
}
