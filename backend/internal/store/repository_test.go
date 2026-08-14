package store

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestMemoryStoreRunHonorsContextAndPersistsState(t *testing.T) {
	repository := NewMemoryStore(func() time.Time { return time.Date(2026, 8, 13, 0, 0, 0, 0, time.UTC) })

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := repository.Run(ctx, func(state *State) error {
		state.Users["usr_cancelled"] = controlplane.UserSummary{ID: "usr_cancelled"}
		return nil
	}); !errors.Is(err, context.Canceled) {
		t.Fatalf("Run() cancelled error = %v, want context.Canceled", err)
	}

	if err := repository.Run(context.Background(), func(state *State) error {
		state.Users["usr_committed"] = controlplane.UserSummary{ID: "usr_committed"}
		return nil
	}); err != nil {
		t.Fatalf("Run() error = %v", err)
	}

	_, err := WithState(repository, func(state *State) (struct{}, error) {
		if _, ok := state.Users["usr_cancelled"]; ok {
			t.Fatal("cancelled transaction changed state")
		}
		if _, ok := state.Users["usr_committed"]; !ok {
			t.Fatal("committed transaction did not change state")
		}
		return struct{}{}, nil
	})
	if err != nil {
		t.Fatalf("WithState() error = %v", err)
	}
}
