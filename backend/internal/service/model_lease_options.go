package service

import (
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// ModelLeaseListOptions is the bounded, public filter surface for the admin
// lease list. Values are converted to the storage whitelist at the service
// boundary; callers cannot provide SQL identifiers or expressions.
type ModelLeaseListOptions struct {
	Status    string
	Provider  string
	Model     string
	UserID    string
	DeviceID  string
	AccountID string
	Sort      string
}

func normalizeModelLeaseListOptions(options ModelLeaseListOptions) (store.ModelLeasePageOptions, error) {
	result := store.ModelLeasePageOptions{
		Status:    strings.TrimSpace(options.Status),
		Provider:  strings.TrimSpace(options.Provider),
		Model:     strings.TrimSpace(options.Model),
		UserID:    strings.TrimSpace(options.UserID),
		DeviceID:  strings.TrimSpace(options.DeviceID),
		AccountID: strings.TrimSpace(options.AccountID),
		Sort:      strings.TrimSpace(options.Sort),
	}
	if result.Status != "" {
		switch result.Status {
		case controlplane.ModelLeaseStatusActive, controlplane.ModelLeaseStatusReleased, controlplane.ModelLeaseStatusExpired:
		default:
			return store.ModelLeasePageOptions{}, controlplane.ErrInvalidRequest
		}
	}
	if result.Sort != "" {
		switch result.Sort {
		case store.ModelLeaseSortExpiresDesc, store.ModelLeaseSortExpiresAsc, store.ModelLeaseSortStatus, store.ModelLeaseSortProviderModel:
		default:
			return store.ModelLeasePageOptions{}, controlplane.ErrInvalidRequest
		}
	}
	if len(result.Provider) > 64 || len(result.Model) > 128 || len(result.UserID) > 64 || len(result.DeviceID) > 64 || len(result.AccountID) > 64 {
		return store.ModelLeasePageOptions{}, controlplane.ErrInvalidRequest
	}
	return result, nil
}
