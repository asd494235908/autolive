package store

import (
	"context"
	"slices"
	"sync"
	"time"

	"autoLive/backend/internal/controlplane"
)

// MemoryStore 是 PostgreSQL 接入前的进程内测试实现；进程退出后所有状态都会丢失。
type MemoryStore struct {
	mu                       sync.Mutex
	now                      func() time.Time
	state                    *State
	idempotencyRecordCreated map[string]time.Time
}

type State struct {
	Products                  map[string]controlplane.ProductSummary
	UserProducts              map[string]controlplane.UserProductMembership
	Users                     map[string]controlplane.UserSummary
	UserAuthorizationPolicies map[string]controlplane.UserAuthorizationPolicy
	UserCredentialHashes      map[string][]byte
	Devices                   map[string]controlplane.DeviceSummary
	ActivationCodes           map[string]ActivationCodeRecord
	ActivationCodeIndex       map[string]string
	ModelPoolAccounts         map[string]controlplane.ModelPoolAccountSummary
	ModelLeases               map[string]controlplane.ModelLease
	ModelUsageRecords         map[string]controlplane.ModelUsageRecord
	ModelPoolTestResults      map[string]controlplane.ModelPoolConnectivityTestResult
	IdempotencyRecords        map[string]IdempotencyRecord
	AuditLogs                 map[string]controlplane.AuditLog
	PendingSecretCleanup      map[string]time.Time
	SequenceCounters          map[string]int
}

type ActivationCodeRecord struct {
	ActivationCode controlplane.ActivationCode
	PlainCode      string
	CodePrefix     string
	UsedByUserID   string
	UsedByDeviceID string
	UsedAt         string
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
		now:                      now,
		state:                    NewState(),
		idempotencyRecordCreated: map[string]time.Time{},
	}
}

func NewState() *State {
	return &State{
		Products:                  map[string]controlplane.ProductSummary{},
		UserProducts:              map[string]controlplane.UserProductMembership{},
		Users:                     map[string]controlplane.UserSummary{},
		UserAuthorizationPolicies: map[string]controlplane.UserAuthorizationPolicy{},
		UserCredentialHashes:      map[string][]byte{},
		Devices:                   map[string]controlplane.DeviceSummary{},
		ActivationCodes:           map[string]ActivationCodeRecord{},
		ActivationCodeIndex:       map[string]string{},
		ModelPoolAccounts:         map[string]controlplane.ModelPoolAccountSummary{},
		ModelLeases:               map[string]controlplane.ModelLease{},
		ModelUsageRecords:         map[string]controlplane.ModelUsageRecord{},
		ModelPoolTestResults:      map[string]controlplane.ModelPoolConnectivityTestResult{},
		IdempotencyRecords:        map[string]IdempotencyRecord{},
		AuditLogs:                 map[string]controlplane.AuditLog{},
		PendingSecretCleanup:      map[string]time.Time{},
		SequenceCounters:          map[string]int{},
	}
}

func ensureStateMaps(state *State) *State {
	if state == nil {
		return NewState()
	}
	defaults := NewState()
	if state.Products == nil {
		state.Products = defaults.Products
	}
	if state.UserProducts == nil {
		state.UserProducts = defaults.UserProducts
	}
	if state.Users == nil {
		state.Users = defaults.Users
	}
	if state.UserAuthorizationPolicies == nil {
		state.UserAuthorizationPolicies = defaults.UserAuthorizationPolicies
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
	if state.PendingSecretCleanup == nil {
		state.PendingSecretCleanup = defaults.PendingSecretCleanup
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
	err := fn(s.state)
	s.syncIdempotencyRecordCreated()
	return err
}

func (s *MemoryStore) syncIdempotencyRecordCreated() {
	now := s.Now()
	for key := range s.state.IdempotencyRecords {
		if _, ok := s.idempotencyRecordCreated[key]; !ok {
			s.idempotencyRecordCreated[key] = now
		}
	}
	for key := range s.idempotencyRecordCreated {
		if _, ok := s.state.IdempotencyRecords[key]; !ok {
			delete(s.idempotencyRecordCreated, key)
		}
	}
}

func (s *MemoryStore) ListUsersPage(ctx context.Context, offset, limit int) (UserPage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return UserPage{}, err
	}
	var page UserPage
	err := s.Run(ctx, func(state *State) error {
		items := make([]controlplane.UserSummary, 0, len(state.Users))
		for _, item := range state.Users {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.UserSummary) int {
			if a.ID < b.ID {
				return -1
			}
			if a.ID > b.ID {
				return 1
			}
			return 0
		})
		page.Total = len(items)
		start, end := pageWindow(page.Total, offset, limit)
		page.Items = append([]controlplane.UserSummary(nil), items[start:end]...)
		return nil
	})
	return page, err
}

func (s *MemoryStore) ListDevicesPage(ctx context.Context, offset, limit int) (DevicePage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	var page DevicePage
	err := s.Run(ctx, func(state *State) error {
		items := make([]controlplane.DeviceSummary, 0, len(state.Devices))
		for _, item := range state.Devices {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int {
			if a.ID < b.ID {
				return -1
			}
			if a.ID > b.ID {
				return 1
			}
			return 0
		})
		page.Total = len(items)
		start, end := pageWindow(page.Total, offset, limit)
		page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
		return nil
	})
	return page, err
}

func (s *MemoryStore) ListDevicesForUserPage(ctx context.Context, userID string, offset, limit int) (DevicePage, error) {
	if err := validatePageWindow(offset, limit); err != nil {
		return DevicePage{}, err
	}
	var page DevicePage
	err := s.Run(ctx, func(state *State) error {
		if _, ok := state.Users[userID]; !ok {
			return controlplane.ErrUserNotFound
		}
		items := make([]controlplane.DeviceSummary, 0)
		for _, item := range state.Devices {
			if item.UserID == userID {
				items = append(items, item)
			}
		}
		slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int {
			if a.ID < b.ID {
				return -1
			}
			if a.ID > b.ID {
				return 1
			}
			return 0
		})
		page.Total = len(items)
		start, end := pageWindow(page.Total, offset, limit)
		page.Items = append([]controlplane.DeviceSummary(nil), items[start:end]...)
		return nil
	})
	return page, err
}

func (s *MemoryStore) ListModelUsagePage(ctx context.Context, offset, limit int) (ModelUsagePage, error) {
	return s.ListModelUsagePageWithOptions(ctx, ModelUsagePageOptions{Offset: offset, Limit: limit})
}

func (s *MemoryStore) ListModelUsagePageWithOptions(ctx context.Context, options ModelUsagePageOptions) (ModelUsagePage, error) {
	options, err := NormalizeModelUsagePageOptions(options)
	if err != nil {
		return ModelUsagePage{}, err
	}
	var page ModelUsagePage
	err = s.Run(ctx, func(state *State) error {
		items := make([]controlplane.ModelUsageRecord, 0, len(state.ModelUsageRecords))
		for _, item := range state.ModelUsageRecords {
			lease := state.ModelLeases[item.LeaseID]
			if !modelUsageMatchesPageOptions(item, options, lease.UserID, lease.DeviceID) {
				continue
			}
			items = append(items, item)
		}
		sortModelUsageRecords(items, options.Sort)
		page.Total = len(items)
		start, end := pageWindow(page.Total, options.Offset, options.Limit)
		page.Items = append([]controlplane.ModelUsageRecord(nil), items[start:end]...)
		return nil
	})
	return page, err
}

func (s *MemoryStore) ListAuditLogsPage(ctx context.Context, offset, limit int) (AuditPage, error) {
	return s.ListAuditLogsPageWithOptions(ctx, AuditLogPageOptions{Offset: offset, Limit: limit})
}

func (s *MemoryStore) ListAuditLogsPageWithOptions(ctx context.Context, options AuditLogPageOptions) (AuditPage, error) {
	options, err := NormalizeAuditLogPageOptions(options)
	if err != nil {
		return AuditPage{}, err
	}
	var page AuditPage
	err = s.Run(ctx, func(state *State) error {
		items := make([]controlplane.AuditLog, 0, len(state.AuditLogs))
		for _, item := range state.AuditLogs {
			if auditLogMatchesPageOptions(item, options) {
				items = append(items, item)
			}
		}
		sortAuditLogs(items, options.Sort)
		page.Total = len(items)
		start, end := pageWindow(page.Total, options.Offset, options.Limit)
		page.Items = append([]controlplane.AuditLog(nil), items[start:end]...)
		return nil
	})
	return page, err
}

func (s *MemoryStore) ListModelLeasesPage(ctx context.Context, offset, limit int) (ModelLeasePage, error) {
	return s.ListModelLeasesPageWithOptions(ctx, ModelLeasePageOptions{Offset: offset, Limit: limit})
}

func (s *MemoryStore) ListModelLeasesPageWithOptions(ctx context.Context, options ModelLeasePageOptions) (ModelLeasePage, error) {
	options, err := NormalizeModelLeasePageOptions(options)
	if err != nil {
		return ModelLeasePage{}, err
	}
	var page ModelLeasePage
	err = s.Run(ctx, func(state *State) error {
		items := make([]controlplane.ModelLeaseAdminSummary, 0, len(state.ModelLeases))
		now := s.Now()
		for _, lease := range state.ModelLeases {
			item := modelLeaseAdminSummary(lease)
			if !modelLeaseMatchesPageOptions(item, options, now) {
				continue
			}
			normalizeModelLeaseSummaryStatus(&item, now)
			items = append(items, item)
		}
		sortModelLeaseSummaries(items, options.Sort)
		page.Total = len(items)
		start, end := pageWindow(page.Total, options.Offset, options.Limit)
		page.Items = append([]controlplane.ModelLeaseAdminSummary(nil), items[start:end]...)
		return nil
	})
	return page, err
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
