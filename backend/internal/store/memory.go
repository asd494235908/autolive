package store

import (
	"context"
	"sync"
	"time"

	"autoLive/backend/internal/controlplane"
)

// MemoryStore 是 PostgreSQL 接入前的进程内测试实现；进程退出后所有状态都会丢失。
type MemoryStore struct {
	mu    sync.Mutex
	now   func() time.Time
	state *State
}

type State struct {
	Users                map[string]controlplane.UserSummary
	UserCredentialHashes map[string][]byte
	Devices              map[string]controlplane.DeviceSummary
	ActivationCodes      map[string]ActivationCodeRecord
	ActivationCodeIndex  map[string]string
	ModelPoolAccounts    map[string]controlplane.ModelPoolAccountSummary
	ModelLeases          map[string]controlplane.ModelLease
	ModelUsageRecords    map[string]controlplane.ModelUsageRecord
	ModelPoolTestResults map[string]controlplane.ModelPoolConnectivityTestResult
	IdempotencyRecords   map[string]IdempotencyRecord
	AuditLogs            map[string]controlplane.AuditLog
	SequenceCounters     map[string]int
}

type ActivationCodeRecord struct {
	ActivationCode controlplane.ActivationCode
	PlainCode      string
	CodePrefix     string
	UsedByDeviceID string
}

type IdempotencyRecord struct {
	Fingerprint string
	ResourceID  string
}

func NewMemoryStore(now func() time.Time) *MemoryStore {
	if now == nil {
		now = time.Now
	}

	return &MemoryStore{
		now:   now,
		state: NewState(),
	}
}

func NewState() *State {
	return &State{
		Users:                map[string]controlplane.UserSummary{},
		UserCredentialHashes: map[string][]byte{},
		Devices:              map[string]controlplane.DeviceSummary{},
		ActivationCodes:      map[string]ActivationCodeRecord{},
		ActivationCodeIndex:  map[string]string{},
		ModelPoolAccounts:    map[string]controlplane.ModelPoolAccountSummary{},
		ModelLeases:          map[string]controlplane.ModelLease{},
		ModelUsageRecords:    map[string]controlplane.ModelUsageRecord{},
		ModelPoolTestResults: map[string]controlplane.ModelPoolConnectivityTestResult{},
		IdempotencyRecords:   map[string]IdempotencyRecord{},
		AuditLogs:            map[string]controlplane.AuditLog{},
		SequenceCounters:     map[string]int{},
	}
}

func ensureStateMaps(state *State) *State {
	if state == nil {
		return NewState()
	}
	defaults := NewState()
	if state.Users == nil {
		state.Users = defaults.Users
	}
	if state.UserCredentialHashes == nil {
		state.UserCredentialHashes = defaults.UserCredentialHashes
	}
	if state.Devices == nil {
		state.Devices = defaults.Devices
	}
	if state.ActivationCodes == nil {
		state.ActivationCodes = defaults.ActivationCodes
	}
	if state.ActivationCodeIndex == nil {
		state.ActivationCodeIndex = defaults.ActivationCodeIndex
	}
	if state.ModelPoolAccounts == nil {
		state.ModelPoolAccounts = defaults.ModelPoolAccounts
	}
	if state.ModelLeases == nil {
		state.ModelLeases = defaults.ModelLeases
	}
	if state.ModelUsageRecords == nil {
		state.ModelUsageRecords = defaults.ModelUsageRecords
	}
	if state.ModelPoolTestResults == nil {
		state.ModelPoolTestResults = defaults.ModelPoolTestResults
	}
	if state.IdempotencyRecords == nil {
		state.IdempotencyRecords = defaults.IdempotencyRecords
	}
	if state.AuditLogs == nil {
		state.AuditLogs = defaults.AuditLogs
	}
	if state.SequenceCounters == nil {
		state.SequenceCounters = defaults.SequenceCounters
	}
	return state
}

func (s *MemoryStore) Now() time.Time {
	return s.now().UTC()
}

func (s *MemoryStore) Run(ctx context.Context, fn StateOperation) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	return fn(s.state)
}

func WithState[T any](s *MemoryStore, fn func(state *State) (T, error)) (T, error) {
	var result T
	err := s.Run(context.Background(), func(state *State) error {
		var err error
		result, err = fn(state)
		return err
	})
	return result, err
}
