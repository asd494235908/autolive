package service

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"regexp"
	"slices"
	"strings"
	"time"

	"golang.org/x/crypto/bcrypt"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

var (
	idPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{7,63}$`)
)

type ControlPlane struct {
	repository  store.Repository
	secretStore store.SecretStore
	httpClient  *http.Client
}

func NewControlPlane(memory *store.MemoryStore) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(memory, &http.Client{Timeout: 30 * time.Second}, store.NewMemorySecretStore())
}

func NewControlPlaneWithHTTPClient(memory *store.MemoryStore, client *http.Client) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(memory, client, store.NewMemorySecretStore())
}

func NewControlPlaneWithRepository(repository store.Repository) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(repository, &http.Client{Timeout: 30 * time.Second}, store.NewMemorySecretStore())
}

func NewControlPlaneWithRepositoryAndHTTPClient(repository store.Repository, client *http.Client) *ControlPlane {
	return NewControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore())
}

func NewControlPlaneWithRepositoryAndSecretStore(repository store.Repository, client *http.Client, secretStore store.SecretStore) *ControlPlane {
	if repository == nil {
		repository = store.NewMemoryStore(time.Now)
	}
	if secretStore == nil {
		secretStore = store.NewMemorySecretStore()
	}
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}
	return &ControlPlane{repository: repository, secretStore: secretStore, httpClient: client}
}

func (s *ControlPlane) EnsureLocalAdmin(username string) {
	if strings.TrimSpace(username) == "" {
		username = "admin"
	}

	_ = s.repository.Run(context.Background(), func(state *store.State) error {
		if _, exists := state.Users["usr_local_admin"]; exists {
			return nil
		}

		now := s.repository.Now().Format(time.RFC3339)
		state.Users["usr_local_admin"] = controlplane.UserSummary{
			ID:        "usr_local_admin",
			Username:  username,
			Role:      controlplane.RoleAdmin,
			Status:    controlplane.UserStatusActive,
			CreatedAt: now,
		}
		return nil
	})
}

func withState[T any](ctx context.Context, repository store.Repository, fn func(state *store.State) (T, error)) (T, error) {
	var result T
	err := repository.Run(ctx, func(state *store.State) error {
		var err error
		result, err = fn(state)
		return err
	})
	return result, err
}

func (s *ControlPlane) RecordAudit(ctx context.Context, input controlplane.AuditLogInput) error {
	if err := checkContext(ctx); err != nil {
		return err
	}
	input.ActorUserID = strings.TrimSpace(input.ActorUserID)
	input.DeviceID = strings.TrimSpace(input.DeviceID)
	input.Action = strings.TrimSpace(input.Action)
	input.TargetType = strings.TrimSpace(input.TargetType)
	input.TargetID = strings.TrimSpace(input.TargetID)
	input.RequestID = strings.TrimSpace(input.RequestID)
	if input.Action == "" || input.TargetType == "" || len(input.Action) > 512 || len(input.TargetType) > 128 || len(input.TargetID) > 128 || len(input.RequestID) > 128 {
		return controlplane.ErrInvalidRequest
	}
	return s.repository.Run(ctx, func(state *store.State) error {
		id := nextID(state, "audit")
		state.AuditLogs[id] = controlplane.AuditLog{
			ID:          id,
			ActorUserID: input.ActorUserID,
			DeviceID:    input.DeviceID,
			Action:      input.Action,
			TargetType:  input.TargetType,
			TargetID:    input.TargetID,
			RequestID:   input.RequestID,
			CreatedAt:   s.repository.Now().Format(time.RFC3339),
		}
		return nil
	})
}

func (s *ControlPlane) ListAuditLogs(ctx context.Context) ([]controlplane.AuditLog, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.AuditLog, error) {
		items := make([]controlplane.AuditLog, 0, len(state.AuditLogs))
		for _, item := range state.AuditLogs {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.AuditLog) int {
			if a.CreatedAt != b.CreatedAt {
				return strings.Compare(b.CreatedAt, a.CreatedAt)
			}
			return strings.Compare(b.ID, a.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) ListUsers(ctx context.Context) ([]controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.UserSummary, error) {
		items := make([]controlplane.UserSummary, 0, len(state.Users))
		for _, item := range state.Users {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.UserSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) CreateUser(ctx context.Context, idempotencyKey string, input controlplane.CreateUserInput) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	input.Username = strings.TrimSpace(input.Username)
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateUserInput(input); err != nil {
		return controlplane.UserSummary{}, err
	}
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(input.Password), bcrypt.DefaultCost)
	if err != nil {
		return controlplane.UserSummary{}, fmt.Errorf("hash user password: %w", err)
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		fingerprint, err := fingerprintValue(input)
		if err != nil {
			return controlplane.UserSummary{}, err
		}

		if existing, ok := state.IdempotencyRecords["create-user:"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Users[existing.ResourceID], nil
		}

		for _, user := range state.Users {
			if user.Username == input.Username {
				return controlplane.UserSummary{}, controlplane.ErrUsernameAlreadyExists
			}
		}

		user := controlplane.UserSummary{
			ID:        nextID(state, "usr"),
			Username:  strings.TrimSpace(input.Username),
			Role:      input.Role,
			Status:    controlplane.UserStatusActive,
			CreatedAt: s.repository.Now().Format(time.RFC3339),
		}
		state.Users[user.ID] = user
		state.UserCredentialHashes[user.ID] = append([]byte(nil), passwordHash...)
		state.IdempotencyRecords["create-user:"+idempotencyKey] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  user.ID,
		}
		return user, nil
	})
}

func (s *ControlPlane) AuthenticateUser(ctx context.Context, username, password string) (controlplane.Actor, controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.Actor{}, controlplane.UserSummary{}, err
	}
	username = strings.TrimSpace(username)
	if username == "" || password == "" {
		return controlplane.Actor{}, controlplane.UserSummary{}, controlplane.ErrUnauthenticated
	}

	result, err := withState(ctx, s.repository, func(state *store.State) (struct {
		Actor controlplane.Actor
		User  controlplane.UserSummary
	}, error) {
		for _, user := range state.Users {
			if user.Username != username {
				continue
			}
			if user.Status != controlplane.UserStatusActive {
				return struct {
					Actor controlplane.Actor
					User  controlplane.UserSummary
				}{}, controlplane.ErrUserDisabled
			}
			hash := state.UserCredentialHashes[user.ID]
			if bcrypt.CompareHashAndPassword(hash, []byte(password)) != nil {
				return struct {
					Actor controlplane.Actor
					User  controlplane.UserSummary
				}{}, controlplane.ErrUnauthenticated
			}
			return struct {
				Actor controlplane.Actor
				User  controlplane.UserSummary
			}{
				Actor: controlplane.Actor{UserID: user.ID, Role: user.Role},
				User:  user,
			}, nil
		}
		return struct {
			Actor controlplane.Actor
			User  controlplane.UserSummary
		}{}, controlplane.ErrUnauthenticated
	})
	if err != nil {
		return controlplane.Actor{}, controlplane.UserSummary{}, err
	}
	return result.Actor, result.User, nil
}

func (s *ControlPlane) DisableUser(ctx context.Context, idempotencyKey, userID string) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.UserSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		if user.ID == "usr_local_admin" {
			return controlplane.UserSummary{}, controlplane.ErrCannotDisableLocalAdmin
		}
		fingerprint, err := fingerprintValue(struct {
			UserID string `json:"user_id"`
		}{UserID: userID})
		if err != nil {
			return controlplane.UserSummary{}, err
		}
		scope := "disable-user:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.UserSummary{}, controlplane.ErrIdempotencyConflict
			}
			return user, nil
		}
		if user.Status == controlplane.UserStatusDisabled {
			state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
			return user, nil
		}
		user.Status = controlplane.UserStatusDisabled
		state.Users[user.ID] = user
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: userID}
		return user, nil
	})
}

func (s *ControlPlane) GetUser(ctx context.Context, userID string) (controlplane.UserSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.UserSummary{}, err
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.UserSummary, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.UserSummary{}, controlplane.ErrUserNotFound
		}
		return user, nil
	})
}

func (s *ControlPlane) ListDevices(ctx context.Context) ([]controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.DeviceSummary, error) {
		items := make([]controlplane.DeviceSummary, 0, len(state.Devices))
		for _, item := range state.Devices {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.DeviceSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) GetClientProfile(ctx context.Context, userID, deviceID string) (controlplane.ClientProfile, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ClientProfile{}, err
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.ClientProfile, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ClientProfile{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrUserDisabled
		}
		device, err := resolveOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ClientProfile{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ClientProfile{}, controlplane.ErrDeviceDisabled
		}
		return controlplane.ClientProfile{
			User:        user,
			Device:      device,
			Permissions: permissionsForRole(user.Role),
		}, nil
	})
}

func (s *ControlPlane) RecordHeartbeat(ctx context.Context, idempotencyKey, userID string, input controlplane.HeartbeatInput) (controlplane.HeartbeatResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.HeartbeatResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateHeartbeatInput(input); err != nil {
		return controlplane.HeartbeatResult{}, err
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.HeartbeatResult, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.HeartbeatResult{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.HeartbeatResult{}, controlplane.ErrUserDisabled
		}
		device, err := resolveOwnedDevice(state, userID, input.DeviceID)
		if err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.HeartbeatResult{}, controlplane.ErrDeviceDisabled
		}

		fingerprint, err := fingerprintValue(struct {
			UserID string                      `json:"user_id"`
			Input  controlplane.HeartbeatInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.HeartbeatResult{}, err
		}
		scope := "heartbeat:" + userID + ":" + input.DeviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.HeartbeatResult{}, controlplane.ErrIdempotencyConflict
			}
			device = state.Devices[existing.ResourceID]
			return controlplane.HeartbeatResult{
				AcceptedAt:   device.LastSeenAt,
				DeviceStatus: device.Status,
			}, nil
		}

		acceptedAt := s.repository.Now().Format(time.RFC3339)
		device.DiskFreeBytes = input.Status.DiskFreeBytes
		device.MemoryTotalBytes = input.Status.MemoryTotalBytes
		device.MemoryAvailableBytes = input.Status.MemoryAvailableBytes
		device.CPULogicalCores = input.Status.CPULogicalCores
		device.RuntimeOSName = input.Status.OSName
		device.RuntimeOSVersion = input.Status.OSVersion
		device.KernelVersion = input.Status.KernelVersion
		device.LastSeenAt = acceptedAt
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  device.ID,
		}
		return controlplane.HeartbeatResult{
			AcceptedAt:   acceptedAt,
			DeviceStatus: device.Status,
		}, nil
	})
}

func (s *ControlPlane) ListModelPoolAccounts(ctx context.Context) ([]controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelPoolAccountSummary, error) {
		now := s.repository.Now()
		sweepExpiredModelLeases(state, now)
		items := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
		for _, account := range state.ModelPoolAccounts {
			items = append(items, decorateModelPoolAccount(state, account, now))
		}
		slices.SortFunc(items, func(a, b controlplane.ModelPoolAccountSummary) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) CreateModelPoolAccount(ctx context.Context, idempotencyKey string, input controlplane.CreateModelPoolAccountInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateModelPoolAccountInput(&input); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		fingerprint, err := fingerprintValue(input)
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		scope := "create-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		account := controlplane.ModelPoolAccountSummary{
			ID:               nextID(state, "mpa"),
			Provider:         input.Provider,
			Model:            input.Model,
			BaseURL:          input.BaseURL,
			Status:           input.Status,
			Priority:         input.Priority,
			DailyLimit:       input.DailyLimit,
			ConcurrencyLimit: input.ConcurrencyLimit,
			SecretConfigured: true,
		}
		account.SecretRef = "model-account/" + account.ID
		state.ModelPoolAccounts[account.ID] = account
		if err := s.secretStore.Put(ctx, account.SecretRef, input.APIKey); err != nil {
			delete(state.ModelPoolAccounts, account.ID)
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrSecretStoreUnavailable
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  account.ID,
		}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) DisableModelPoolAccount(ctx context.Context, idempotencyKey, accountID string) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		fingerprint, err := fingerprintValue(struct {
			AccountID string `json:"account_id"`
		}{AccountID: accountID})
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		scope := "disable-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		sweepExpiredModelLeases(state, s.repository.Now())
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
		}
		if activeLeaseCountForAccount(state, account.ID) > 0 {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountInUse
		}
		account.Status = controlplane.ModelAccountStatusDisabled
		state.ModelPoolAccounts[account.ID] = account
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  account.ID,
		}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) UpdateModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.UpdateModelPoolAccountInput) (controlplane.ModelPoolAccountSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolAccountSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
	}
	if input.BaseURL == nil && input.Priority == nil && input.DailyLimit == nil && input.ConcurrencyLimit == nil && input.Status == nil {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.BaseURL != nil {
		value := strings.TrimRight(strings.TrimSpace(*input.BaseURL), "/")
		input.BaseURL = &value
		if value != "" && !validModelBaseURL(value) {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
		}
	}
	if input.Priority != nil && *input.Priority < 0 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.DailyLimit != nil && *input.DailyLimit < 0 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.ConcurrencyLimit != nil && *input.ConcurrencyLimit < 1 {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}
	if input.Status != nil && !validModelAccountStatus(*input.Status) {
		return controlplane.ModelPoolAccountSummary{}, controlplane.ErrInvalidRequest
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelPoolAccountSummary, error) {
		fingerprint, err := fingerprintValue(struct {
			AccountID string                                   `json:"account_id"`
			Input     controlplane.UpdateModelPoolAccountInput `json:"input"`
		}{AccountID: accountID, Input: input})
		if err != nil {
			return controlplane.ModelPoolAccountSummary{}, err
		}
		scope := "update-model-account:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrIdempotencyConflict
			}
			account, exists := state.ModelPoolAccounts[existing.ResourceID]
			if !exists {
				return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
			}
			return decorateModelPoolAccount(state, account, s.repository.Now()), nil
		}

		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolAccountNotFound
		}
		sweepExpiredModelLeases(state, s.repository.Now())
		if input.ConcurrencyLimit != nil && activeLeaseCountForAccount(state, account.ID) > *input.ConcurrencyLimit {
			return controlplane.ModelPoolAccountSummary{}, controlplane.ErrModelPoolConcurrencyConflict
		}
		if input.BaseURL != nil {
			account.BaseURL = *input.BaseURL
		}
		if input.Priority != nil {
			account.Priority = *input.Priority
		}
		if input.DailyLimit != nil {
			account.DailyLimit = *input.DailyLimit
		}
		if input.ConcurrencyLimit != nil {
			account.ConcurrencyLimit = *input.ConcurrencyLimit
		}
		if input.Status != nil {
			account.Status = *input.Status
		}
		state.ModelPoolAccounts[account.ID] = account
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: account.ID}
		return decorateModelPoolAccount(state, account, s.repository.Now()), nil
	})
}

func (s *ControlPlane) CreateModelLease(ctx context.Context, idempotencyKey, userID, deviceID string, input controlplane.CreateModelLeaseInput) (controlplane.ModelLease, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelLease{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelLease{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateCreateModelLeaseInput(&input); err != nil {
		return controlplane.ModelLease{}, err
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelLease, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		fingerprint, err := fingerprintValue(struct {
			UserID   string                             `json:"user_id"`
			DeviceID string                             `json:"device_id"`
			Input    controlplane.CreateModelLeaseInput `json:"input"`
		}{
			UserID:   userID,
			DeviceID: device.ID,
			Input:    input,
		})
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		scope := "create-model-lease:" + userID + ":" + device.ID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
			}
			lease, exists := state.ModelLeases[existing.ResourceID]
			if !exists {
				return controlplane.ModelLease{}, controlplane.ErrModelLeaseNotFound
			}
			return lease, nil
		}

		account, ok := selectModelPoolAccount(state, input.Provider, input.Model, s.repository.Now())
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}
		if _, err := s.secretStore.Get(ctx, account.SecretRef); err != nil {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}

		lease := controlplane.ModelLease{
			ID:               nextID(state, "lease"),
			UserID:           userID,
			DeviceID:         device.ID,
			AccountID:        account.ID,
			Purpose:          input.Purpose,
			Provider:         account.Provider,
			Model:            account.Model,
			Status:           controlplane.ModelLeaseStatusActive,
			ExpiresAt:        s.repository.Now().Add(time.Duration(input.MaxDurationSeconds) * time.Second).UTC().Format(time.RFC3339),
			ProxyMode:        controlplane.ModelLeaseProxyModeDirectLease,
			DirectBaseURL:    account.BaseURL,
			ConcurrencyLimit: account.ConcurrencyLimit,
		}
		state.ModelLeases[lease.ID] = lease
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  lease.ID,
		}
		return lease, nil
	})
}

func (s *ControlPlane) RenewModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.RenewModelLeaseInput) (controlplane.ModelLease, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelLease{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelLease{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateRenewModelLeaseInput(&input); err != nil {
		return controlplane.ModelLease{}, err
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelLease, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ModelLease{}, controlplane.ErrForbidden
		}

		fingerprint, err := fingerprintValue(struct {
			UserID       string                            `json:"user_id"`
			DeviceID     string                            `json:"device_id"`
			LeaseID      string                            `json:"lease_id"`
			RenewRequest controlplane.RenewModelLeaseInput `json:"renew_request"`
		}{
			UserID:       userID,
			DeviceID:     device.ID,
			LeaseID:      leaseID,
			RenewRequest: input,
		})
		if err != nil {
			return controlplane.ModelLease{}, err
		}
		scope := "renew-model-lease:" + leaseID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelLease{}, controlplane.ErrIdempotencyConflict
			}
			return state.ModelLeases[leaseID], nil
		}

		if lease.Status != controlplane.ModelLeaseStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
		}
		account, ok := state.ModelPoolAccounts[lease.AccountID]
		if !ok || account.Status != controlplane.ModelAccountStatusActive {
			return controlplane.ModelLease{}, controlplane.ErrModelPoolUnavailable
		}
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			return controlplane.ModelLease{}, controlplane.ErrModelLeaseStateConflict
		}
		lease.ExpiresAt = expiresAt.Add(time.Duration(input.ExtendSeconds) * time.Second).UTC().Format(time.RFC3339)
		state.ModelLeases[lease.ID] = lease
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  lease.ID,
		}
		return lease, nil
	})
}

func (s *ControlPlane) ReleaseModelLease(ctx context.Context, idempotencyKey, userID, deviceID, leaseID string, input controlplane.ReleaseModelLeaseInput) (controlplane.ReleaseModelLeaseResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ReleaseModelLeaseResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	if len(strings.TrimSpace(input.Reason)) > 255 {
		return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrInvalidRequest
	}
	input.Reason = strings.TrimSpace(input.Reason)

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ReleaseModelLeaseResult, error) {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrUserDisabled
		}
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		if device.Status != controlplane.DeviceStatusActive {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrDeviceDisabled
		}

		sweepExpiredModelLeases(state, s.repository.Now())

		lease, ok := state.ModelLeases[leaseID]
		if !ok {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrForbidden
		}

		fingerprint, err := fingerprintValue(struct {
			UserID       string                              `json:"user_id"`
			DeviceID     string                              `json:"device_id"`
			LeaseID      string                              `json:"lease_id"`
			ReleaseInput controlplane.ReleaseModelLeaseInput `json:"release_input"`
		}{
			UserID:       userID,
			DeviceID:     device.ID,
			LeaseID:      leaseID,
			ReleaseInput: input,
		})
		if err != nil {
			return controlplane.ReleaseModelLeaseResult{}, err
		}
		scope := "release-model-lease:" + leaseID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ReleaseModelLeaseResult{}, controlplane.ErrIdempotencyConflict
			}
			return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
		}
		if lease.Status == controlplane.ModelLeaseStatusActive {
			lease.Status = controlplane.ModelLeaseStatusReleased
			state.ModelLeases[lease.ID] = lease
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  leaseID,
		}
		return controlplane.ReleaseModelLeaseResult{LeaseID: leaseID, Released: true}, nil
	})
}

func (s *ControlPlane) TestModelPoolAccount(ctx context.Context, idempotencyKey, accountID string, input controlplane.TestModelPoolAccountInput) (controlplane.ModelPoolConnectivityTestResult, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrIdempotencyKeyRequired
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" || validateTestModelPoolAccountInput(&input) != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	type modelPoolTestContext struct {
		account controlplane.ModelPoolAccountSummary
		cached  *controlplane.ModelPoolConnectivityTestResult
	}
	testContext, err := withState(ctx, s.repository, func(state *store.State) (modelPoolTestContext, error) {
		account, ok := state.ModelPoolAccounts[accountID]
		if !ok {
			return modelPoolTestContext{}, controlplane.ErrModelPoolAccountNotFound
		}
		fingerprint, fingerprintErr := fingerprintValue(struct {
			AccountID string                                 `json:"account_id"`
			Input     controlplane.TestModelPoolAccountInput `json:"input"`
		}{AccountID: accountID, Input: input})
		if fingerprintErr != nil {
			return modelPoolTestContext{}, fingerprintErr
		}
		if existing, ok := state.IdempotencyRecords["test-model-account:"+accountID+":"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return modelPoolTestContext{}, controlplane.ErrIdempotencyConflict
			}
			cached, exists := state.ModelPoolTestResults[existing.ResourceID]
			if !exists {
				return modelPoolTestContext{}, controlplane.ErrModelPoolAccountNotFound
			}
			return modelPoolTestContext{account: account, cached: &cached}, nil
		}
		return modelPoolTestContext{account: account}, nil
	})
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	if testContext.cached != nil {
		return *testContext.cached, nil
	}
	account := testContext.account
	secret, err := s.secretStore.Get(ctx, account.SecretRef)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrSecretStoreUnavailable
	}
	baseURL := strings.TrimRight(account.BaseURL, "/")
	if baseURL == "" {
		baseURL = "https://api.openai.com/v1"
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, baseURL+"/models", nil)
	if err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, controlplane.ErrInvalidRequest
	}
	request.Header.Set("Accept", "application/json")
	request.Header.Set("Authorization", "Bearer "+secret)
	client := *s.httpClient
	client.Timeout = time.Duration(input.TimeoutSeconds) * time.Second
	startedAt := time.Now()
	response, requestErr := client.Do(request)
	result := controlplane.ModelPoolConnectivityTestResult{
		AccountID: account.ID,
		Provider:  account.Provider,
		Model:     account.Model,
		TestedAt:  s.repository.Now().Format(time.RFC3339),
		LatencyMS: time.Since(startedAt).Milliseconds(),
	}
	if requestErr != nil {
		result.Status = "failed"
		result.ErrorCode = "MODEL_POOL_CONNECTIVITY_FAILED"
		if ctx.Err() != nil || (func() bool {
			timeoutError, ok := requestErr.(net.Error)
			return ok && timeoutError.Timeout()
		})() {
			result.Status = "timeout"
			result.ErrorCode = "MODEL_POOL_CONNECTIVITY_TIMEOUT"
		}
	} else {
		defer response.Body.Close()
		_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 4096))
		result.HTTPStatus = response.StatusCode
		result.ResponseSummary = fmt.Sprintf("HTTP %d", response.StatusCode)
		if response.StatusCode >= http.StatusOK && response.StatusCode < http.StatusMultipleChoices {
			result.Status = "succeeded"
		} else {
			result.Status = "failed"
			result.ErrorCode = "MODEL_POOL_PROVIDER_HTTP_ERROR"
		}
	}
	if err := s.repository.Run(ctx, func(state *store.State) error {
		fingerprint, fingerprintErr := fingerprintValue(struct {
			AccountID string                                 `json:"account_id"`
			Input     controlplane.TestModelPoolAccountInput `json:"input"`
		}{AccountID: accountID, Input: input})
		if fingerprintErr != nil {
			return fingerprintErr
		}
		scope := "test-model-account:" + accountID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ErrIdempotencyConflict
			}
			if cached, exists := state.ModelPoolTestResults[existing.ResourceID]; exists {
				result = cached
			}
			return nil
		}
		resultID := nextID(state, "model_test")
		state.ModelPoolTestResults[resultID] = result
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: resultID}
		return nil
	}); err != nil {
		return controlplane.ModelPoolConnectivityTestResult{}, err
	}
	return result, nil
}

func (s *ControlPlane) RecordDirectLLMCall(ctx context.Context, idempotencyKey, userID, deviceID, requestID string, input controlplane.CreateDirectLLMCallRecordInput) (controlplane.ModelUsageRecord, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyKeyRequired
	}
	if err := validateDirectLLMCallRecordInput(&input); err != nil {
		return controlplane.ModelUsageRecord{}, err
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.ModelUsageRecord, error) {
		device, err := resolveBoundOwnedDevice(state, userID, deviceID)
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		lease, ok := state.ModelLeases[input.LeaseID]
		if !ok {
			return controlplane.ModelUsageRecord{}, controlplane.ErrModelLeaseNotFound
		}
		if lease.UserID != userID || lease.DeviceID != device.ID {
			return controlplane.ModelUsageRecord{}, controlplane.ErrForbidden
		}
		if lease.Provider != input.Provider || lease.Model != input.Model {
			return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
		}
		if input.TotalTokens == 0 {
			input.TotalTokens = input.InputTokens + input.OutputTokens
		}
		if input.TotalTokens < input.InputTokens+input.OutputTokens {
			return controlplane.ModelUsageRecord{}, controlplane.ErrInvalidRequest
		}
		fingerprint, err := fingerprintValue(struct {
			UserID   string                                      `json:"user_id"`
			DeviceID string                                      `json:"device_id"`
			Input    controlplane.CreateDirectLLMCallRecordInput `json:"input"`
		}{UserID: userID, DeviceID: device.ID, Input: input})
		if err != nil {
			return controlplane.ModelUsageRecord{}, err
		}
		scope := "record-direct-llm-call:" + userID + ":" + device.ID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
			}
			return state.ModelUsageRecords[existing.ResourceID], nil
		}
		for _, existing := range state.ModelUsageRecords {
			if existing.ClientCallID == input.ClientCallID && existing.LeaseID == input.LeaseID {
				if existing.Provider != input.Provider || existing.Model != input.Model || existing.InputTokens != input.InputTokens || existing.OutputTokens != input.OutputTokens || existing.TotalTokens != input.TotalTokens || existing.LatencyMS != input.LatencyMS || existing.Status != input.Status || existing.UsageSource != input.UsageSource || existing.ErrorCode != input.ErrorCode {
					return controlplane.ModelUsageRecord{}, controlplane.ErrIdempotencyConflict
				}
				state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: existing.ID}
				return existing, nil
			}
		}
		record := controlplane.ModelUsageRecord{
			ID:           nextID(state, "usage"),
			LeaseID:      input.LeaseID,
			ClientCallID: input.ClientCallID,
			RequestID:    strings.TrimSpace(requestID),
			Provider:     input.Provider,
			Model:        input.Model,
			InputTokens:  input.InputTokens,
			OutputTokens: input.OutputTokens,
			TotalTokens:  input.TotalTokens,
			LatencyMS:    input.LatencyMS,
			Status:       input.Status,
			UsageSource:  input.UsageSource,
			ErrorCode:    input.ErrorCode,
			CreatedAt:    s.repository.Now().Format(time.RFC3339),
		}
		state.ModelUsageRecords[record.ID] = record
		account, accountExists := state.ModelPoolAccounts[lease.AccountID]
		if accountExists && account.DailyLimit > 0 && dailyUsedTokensForAccount(state, account.ID, s.repository.Now()) >= account.DailyLimit {
			account.Status = controlplane.ModelAccountStatusExhausted
			state.ModelPoolAccounts[account.ID] = account
		}
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: record.ID}
		return record, nil
	})
}

func (s *ControlPlane) ListModelUsage(ctx context.Context) ([]controlplane.ModelUsageRecord, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ModelUsageRecord, error) {
		items := make([]controlplane.ModelUsageRecord, 0, len(state.ModelUsageRecords))
		for _, item := range state.ModelUsageRecords {
			items = append(items, item)
		}
		slices.SortFunc(items, func(a, b controlplane.ModelUsageRecord) int {
			if a.CreatedAt != b.CreatedAt {
				return strings.Compare(b.CreatedAt, a.CreatedAt)
			}
			return strings.Compare(b.ID, a.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) CreateActivationCode(ctx context.Context, idempotencyKey string, input controlplane.CreateActivationCodeInput) (controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ActivationCode{}, controlplane.ErrIdempotencyKeyRequired
	}
	if input.MaxDevices == 0 {
		input.MaxDevices = 1
	}
	if input.MaxDevices != 1 || input.ExpiresAt.IsZero() || !input.ExpiresAt.After(s.repository.Now()) {
		return controlplane.ActivationCode{}, controlplane.ErrInvalidRequest
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ActivationCode, error) {
		fingerprint, err := fingerprintValue(struct {
			ExpiresAt  string `json:"expires_at"`
			MaxDevices int    `json:"max_devices"`
		}{
			ExpiresAt:  input.ExpiresAt.UTC().Format(time.RFC3339),
			MaxDevices: input.MaxDevices,
		})
		if err != nil {
			return controlplane.ActivationCode{}, err
		}

		if existing, ok := state.IdempotencyRecords["create-activation-code:"+idempotencyKey]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
			}
			record := state.ActivationCodes[existing.ResourceID]
			if record.PlainCode == "" {
				// PostgreSQL 快照不会恢复明文激活码；重复请求只能返回
				// 已持久化的元数据，不能伪造一个空字符串指针。
				record.ActivationCode.PlainCode = nil
			} else {
				plainCode := record.PlainCode
				record.ActivationCode.PlainCode = &plainCode
			}
			return record.ActivationCode, nil
		}

		id := nextID(state, "ac")
		plainCode, err := randomToken("code_", 18)
		if err != nil {
			return controlplane.ActivationCode{}, controlplane.NewError(http.StatusInternalServerError, "RANDOM_GENERATION_FAILED", "无法生成激活码")
		}
		code := controlplane.ActivationCode{
			ID:         id,
			Status:     controlplane.ActivationCodeStatusActive,
			ExpiresAt:  input.ExpiresAt.UTC().Format(time.RFC3339),
			MaxDevices: input.MaxDevices,
			PlainCode:  &plainCode,
		}
		state.ActivationCodes[id] = store.ActivationCodeRecord{
			ActivationCode: code,
			PlainCode:      plainCode,
			CodePrefix:     plainCode[:12],
		}
		state.ActivationCodeIndex[secretDigest(plainCode)] = id
		state.IdempotencyRecords["create-activation-code:"+idempotencyKey] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  id,
		}
		return code, nil
	})
}

func (s *ControlPlane) ListActivationCodes(ctx context.Context) ([]controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	return withState(ctx, s.repository, func(state *store.State) ([]controlplane.ActivationCode, error) {
		items := make([]controlplane.ActivationCode, 0, len(state.ActivationCodes))
		now := s.repository.Now()
		for id, record := range state.ActivationCodes {
			code := record.ActivationCode
			if code.Status == controlplane.ActivationCodeStatusActive {
				expiresAt, _ := time.Parse(time.RFC3339, code.ExpiresAt)
				if !now.Before(expiresAt) {
					code.Status = controlplane.ActivationCodeStatusExpired
					record.ActivationCode.Status = code.Status
					state.ActivationCodes[id] = record
				}
			}
			code.PlainCode = nil
			items = append(items, code)
		}
		slices.SortFunc(items, func(a, b controlplane.ActivationCode) int {
			return strings.Compare(a.ID, b.ID)
		})
		return items, nil
	})
}

func (s *ControlPlane) RevokeActivationCode(ctx context.Context, idempotencyKey, codeID string) (controlplane.ActivationCode, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.ActivationCode{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ActivationCode{}, controlplane.ErrIdempotencyKeyRequired
	}
	codeID = strings.TrimSpace(codeID)
	if codeID == "" {
		return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.ActivationCode, error) {
		fingerprint, err := fingerprintValue(struct {
			CodeID string `json:"code_id"`
		}{CodeID: codeID})
		if err != nil {
			return controlplane.ActivationCode{}, err
		}
		scope := "revoke-activation-code:" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.ActivationCode{}, controlplane.ErrIdempotencyConflict
			}
			record, exists := state.ActivationCodes[existing.ResourceID]
			if !exists {
				return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
			}
			record.ActivationCode.PlainCode = nil
			return record.ActivationCode, nil
		}

		record, ok := state.ActivationCodes[codeID]
		if !ok {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeNotFound
		}
		if record.ActivationCode.Status != controlplane.ActivationCodeStatusActive && record.ActivationCode.Status != controlplane.ActivationCodeStatusRevoked {
			return controlplane.ActivationCode{}, controlplane.ErrActivationCodeStateConflict
		}
		record.ActivationCode.Status = controlplane.ActivationCodeStatusRevoked
		record.ActivationCode.PlainCode = nil
		state.ActivationCodes[codeID] = record
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{
			Fingerprint: fingerprint,
			ResourceID:  codeID,
		}
		return record.ActivationCode, nil
	})
}

func (s *ControlPlane) ActivateDevice(ctx context.Context, idempotencyKey, userID string, input controlplane.ActivateDeviceInput) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	if len(input.ActivationCode) < 8 || len(input.ActivationCode) > 128 || !idPattern.MatchString(input.Device.DeviceID) {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	}
	if strings.TrimSpace(input.Device.DeviceName) == "" || len(input.Device.DeviceName) > 128 || strings.TrimSpace(input.Device.Platform) == "" || len(input.Device.Platform) > 64 || strings.TrimSpace(input.Device.AppVersion) == "" || len(input.Device.AppVersion) > 64 || len(input.Device.OSVersion) > 128 {
		return controlplane.DeviceSummary{}, controlplane.ErrInvalidRequest
	}

	return withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		fingerprint, err := fingerprintValue(struct {
			UserID string                           `json:"user_id"`
			Input  controlplane.ActivateDeviceInput `json:"input"`
		}{UserID: userID, Input: input})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "activate-device:" + userID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			device, exists := state.Devices[existing.ResourceID]
			if !exists {
				return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
			}
			return device, nil
		}
		user, exists := state.Users[userID]
		if !exists {
			return controlplane.DeviceSummary{}, controlplane.ErrUserNotFound
		}
		if user.Status != controlplane.UserStatusActive {
			return controlplane.DeviceSummary{}, controlplane.ErrUserDisabled
		}
		if existing, exists := state.Devices[input.Device.DeviceID]; exists {
			if existing.Status == controlplane.DeviceStatusPendingActivation && existing.UserID == "" {
				// 允许管理员解除绑定后的同一设备使用新激活码重新绑定。
			} else {
				if existing.UserID != userID || existing.Status == controlplane.DeviceStatusDisabled {
					return controlplane.DeviceSummary{}, controlplane.ErrForbidden
				}
				return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingConflict
			}
		}
		codeID, ok := state.ActivationCodeIndex[secretDigest(input.ActivationCode)]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeNotFound
		}
		record := state.ActivationCodes[codeID]
		expiresAt, _ := time.Parse(time.RFC3339, record.ActivationCode.ExpiresAt)
		if !s.repository.Now().Before(expiresAt) {
			record.ActivationCode.Status = controlplane.ActivationCodeStatusExpired
			state.ActivationCodes[codeID] = record
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeExpired
		}
		if record.ActivationCode.Status == controlplane.ActivationCodeStatusRevoked {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeRevoked
		}
		if record.ActivationCode.Status == controlplane.ActivationCodeStatusUsed {
			return controlplane.DeviceSummary{}, controlplane.ErrActivationCodeAlreadyUsed
		}

		now := s.repository.Now().Format(time.RFC3339)
		device := controlplane.DeviceSummary{
			ID:         input.Device.DeviceID,
			UserID:     userID,
			DeviceName: strings.TrimSpace(input.Device.DeviceName),
			Platform:   strings.TrimSpace(input.Device.Platform),
			AppVersion: strings.TrimSpace(input.Device.AppVersion),
			Status:     controlplane.DeviceStatusActive,
			LastSeenAt: now,
		}
		state.Devices[device.ID] = device
		record.ActivationCode.Status = controlplane.ActivationCodeStatusUsed
		record.ActivationCode.PlainCode = nil
		record.UsedByDeviceID = device.ID
		state.ActivationCodes[codeID] = record
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: device.ID}
		return device, nil
	})
}

func (s *ControlPlane) DisableDevice(ctx context.Context, idempotencyKey, deviceID string) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "disable-device:" + deviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			return device, nil
		}
		if device.Status == controlplane.DeviceStatusDisabled {
			state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: deviceID}
			return device, nil
		}
		device.Status = controlplane.DeviceStatusDisabled
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: deviceID}
		return device, nil
	})
}

func (s *ControlPlane) UnbindDevice(ctx context.Context, idempotencyKey, deviceID string) (controlplane.DeviceSummary, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.DeviceSummary{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyKeyRequired
	}
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return withState(ctx, s.repository, func(state *store.State) (controlplane.DeviceSummary, error) {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		fingerprint, err := fingerprintValue(struct {
			DeviceID string `json:"device_id"`
		}{DeviceID: deviceID})
		if err != nil {
			return controlplane.DeviceSummary{}, err
		}
		scope := "unbind-device:" + deviceID + ":" + idempotencyKey
		if existing, ok := state.IdempotencyRecords[scope]; ok {
			if existing.Fingerprint != fingerprint {
				return controlplane.DeviceSummary{}, controlplane.ErrIdempotencyConflict
			}
			return state.Devices[existing.ResourceID], nil
		}
		device.UserID = ""
		device.Status = controlplane.DeviceStatusPendingActivation
		state.Devices[device.ID] = device
		state.IdempotencyRecords[scope] = store.IdempotencyRecord{Fingerprint: fingerprint, ResourceID: device.ID}
		return device, nil
	})
}

func validateCreateUserInput(input controlplane.CreateUserInput) error {
	username := strings.TrimSpace(input.Username)
	if len(username) < 3 || len(username) > 64 || len(input.Password) < 8 || len(input.Password) > 256 {
		return controlplane.ErrInvalidRequest
	}
	if input.Role != controlplane.RoleAdmin && input.Role != controlplane.RoleUser {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateCreateModelPoolAccountInput(input *controlplane.CreateModelPoolAccountInput) error {
	input.Provider = strings.TrimSpace(input.Provider)
	input.Model = strings.TrimSpace(input.Model)
	input.BaseURL = strings.TrimRight(strings.TrimSpace(input.BaseURL), "/")
	input.APIKey = strings.TrimSpace(input.APIKey)
	if input.Status == "" {
		input.Status = controlplane.ModelAccountStatusActive
	}
	if len(input.Provider) == 0 || len(input.Provider) > 64 || len(input.Model) == 0 || len(input.Model) > 128 || len(input.APIKey) < 8 || len(input.APIKey) > 4096 {
		return controlplane.ErrInvalidRequest
	}
	if input.BaseURL != "" && !validModelBaseURL(input.BaseURL) {
		return controlplane.ErrInvalidRequest
	}
	if input.DailyLimit < 0 || input.ConcurrencyLimit < 1 {
		return controlplane.ErrInvalidRequest
	}
	if !validModelAccountStatus(input.Status) {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validModelAccountStatus(status string) bool {
	switch status {
	case controlplane.ModelAccountStatusActive, controlplane.ModelAccountStatusCooldown, controlplane.ModelAccountStatusExhausted, controlplane.ModelAccountStatusDisabled:
		return true
	default:
		return false
	}
}

func validateCreateModelLeaseInput(input *controlplane.CreateModelLeaseInput) error {
	input.Provider = strings.TrimSpace(input.Provider)
	input.Model = strings.TrimSpace(input.Model)
	input.Purpose = strings.TrimSpace(input.Purpose)
	if input.MaxDurationSeconds == 0 {
		input.MaxDurationSeconds = 300
	}
	if len(input.Provider) == 0 || len(input.Provider) > 64 || len(input.Model) == 0 || len(input.Model) > 128 {
		return controlplane.ErrInvalidRequest
	}
	switch input.Purpose {
	case "realtime_script", "chat", "validation":
	default:
		return controlplane.ErrInvalidRequest
	}
	if input.MaxDurationSeconds < 30 || input.MaxDurationSeconds > 3600 {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateRenewModelLeaseInput(input *controlplane.RenewModelLeaseInput) error {
	if input.ExtendSeconds == 0 {
		input.ExtendSeconds = 300
	}
	if input.ExtendSeconds < 30 || input.ExtendSeconds > 3600 {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func permissionsForRole(role string) []string {
	if role == controlplane.RoleAdmin {
		return []string{"admin", "client"}
	}
	return []string{"client"}
}

func resolveOwnedDevice(state *store.State, userID, deviceID string) (controlplane.DeviceSummary, error) {
	if deviceID != "" {
		device, ok := state.Devices[deviceID]
		if !ok {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		if device.UserID != userID {
			return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
		}
		return device, nil
	}

	var selected *controlplane.DeviceSummary
	for _, device := range state.Devices {
		if device.UserID != userID {
			continue
		}
		candidate := device
		if selected == nil || candidate.LastSeenAt > selected.LastSeenAt {
			selected = &candidate
		}
	}
	if selected == nil {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceNotFound
	}
	return *selected, nil
}

func resolveBoundOwnedDevice(state *store.State, userID, deviceID string) (controlplane.DeviceSummary, error) {
	if strings.TrimSpace(deviceID) == "" {
		return controlplane.DeviceSummary{}, controlplane.ErrDeviceBindingRequired
	}
	return resolveOwnedDevice(state, userID, deviceID)
}

func sweepExpiredModelLeases(state *store.State, now time.Time) {
	for id, lease := range state.ModelLeases {
		if lease.Status != controlplane.ModelLeaseStatusActive {
			continue
		}
		expiresAt, err := time.Parse(time.RFC3339, lease.ExpiresAt)
		if err != nil {
			continue
		}
		if !now.Before(expiresAt) {
			lease.Status = controlplane.ModelLeaseStatusExpired
			state.ModelLeases[id] = lease
		}
	}
}

func selectModelPoolAccount(state *store.State, provider, model string, now time.Time) (controlplane.ModelPoolAccountSummary, bool) {
	candidates := make([]controlplane.ModelPoolAccountSummary, 0, len(state.ModelPoolAccounts))
	for _, account := range state.ModelPoolAccounts {
		if account.Provider != provider || account.Model != model || account.Status != controlplane.ModelAccountStatusActive {
			continue
		}
		if !account.SecretConfigured || strings.TrimSpace(account.SecretRef) == "" {
			continue
		}
		if account.DailyLimit > 0 && dailyUsedTokensForAccount(state, account.ID, now) >= account.DailyLimit {
			continue
		}
		candidates = append(candidates, account)
	}
	slices.SortFunc(candidates, func(a, b controlplane.ModelPoolAccountSummary) int {
		if a.Priority != b.Priority {
			if a.Priority > b.Priority {
				return -1
			}
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
	for _, account := range candidates {
		if activeLeaseCountForAccount(state, account.ID) < account.ConcurrencyLimit {
			return account, true
		}
	}
	return controlplane.ModelPoolAccountSummary{}, false
}

func activeLeaseCountForAccount(state *store.State, accountID string) int {
	count := 0
	for _, lease := range state.ModelLeases {
		if lease.AccountID == accountID && lease.Status == controlplane.ModelLeaseStatusActive {
			count++
		}
	}
	return count
}

func dailyUsedTokensForAccount(state *store.State, accountID string, now time.Time) int {
	day := now.UTC().Format("2006-01-02")
	leaseAccountByID := make(map[string]string, len(state.ModelLeases))
	for id, lease := range state.ModelLeases {
		leaseAccountByID[id] = lease.AccountID
	}
	used := 0
	for _, usage := range state.ModelUsageRecords {
		if leaseAccountByID[usage.LeaseID] != accountID {
			continue
		}
		createdAt, err := time.Parse(time.RFC3339, usage.CreatedAt)
		if err != nil || createdAt.UTC().Format("2006-01-02") != day {
			continue
		}
		used += usage.TotalTokens
	}
	return used
}

func decorateModelPoolAccount(state *store.State, account controlplane.ModelPoolAccountSummary, now time.Time) controlplane.ModelPoolAccountSummary {
	account.ActiveLeases = activeLeaseCountForAccount(state, account.ID)
	account.DailyUsedTokens = dailyUsedTokensForAccount(state, account.ID, now)
	var latest controlplane.ModelPoolConnectivityTestResult
	for _, result := range state.ModelPoolTestResults {
		if result.AccountID != account.ID || result.TestedAt == "" || result.TestedAt <= latest.TestedAt {
			continue
		}
		latest = result
	}
	if latest.TestedAt != "" {
		account.LastTestStatus = latest.Status
		account.LastTestedAt = latest.TestedAt
	}
	return account
}
