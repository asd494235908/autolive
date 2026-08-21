package service

import (
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// ModelUsageListOptions is the bounded public filter surface for the admin
// model usage list. Timestamps are parsed before reaching the repository.
type ModelUsageListOptions struct {
	Product       controlplane.ProductCode
	Provider      string
	Model         string
	UserID        string
	DeviceID      string
	RequestID     string
	CreatedAfter  string
	CreatedBefore string
	Sort          string
}

func normalizeModelUsageListOptions(options ModelUsageListOptions) (store.ModelUsagePageOptions, error) {
	result := store.ModelUsagePageOptions{
		Product:  controlplane.ProductCode(strings.TrimSpace(string(options.Product))),
		Provider: options.Provider, Model: options.Model, UserID: options.UserID,
		DeviceID: options.DeviceID, RequestID: options.RequestID, Sort: options.Sort,
	}
	if result.Product != "" && !result.Product.Valid() {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	result.Provider = strings.TrimSpace(result.Provider)
	result.Model = strings.TrimSpace(result.Model)
	result.UserID = strings.TrimSpace(result.UserID)
	result.DeviceID = strings.TrimSpace(result.DeviceID)
	result.RequestID = strings.TrimSpace(result.RequestID)
	result.Sort = strings.TrimSpace(result.Sort)
	if len(result.Provider) > 64 || len(result.Model) > 128 || len(result.UserID) > 64 || len(result.DeviceID) > 64 || len(result.RequestID) > 128 {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	createdAfter, err := parseAuditTime(options.CreatedAfter)
	if err != nil {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	createdBefore, err := parseAuditTime(options.CreatedBefore)
	if err != nil {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	result.CreatedAfter = createdAfter
	result.CreatedBefore = createdBefore
	if createdAfter != nil && createdBefore != nil && createdAfter.After(*createdBefore) {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	if result.Sort != "" && result.Sort != store.ModelUsageSortCreatedDesc && result.Sort != store.ModelUsageSortCreatedAsc {
		return store.ModelUsagePageOptions{}, controlplane.ErrInvalidRequest
	}
	return result, nil
}
