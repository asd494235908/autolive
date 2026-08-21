package store

import (
	"errors"
	"slices"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

const (
	ModelUsageSortCreatedDesc = "created_at_desc"
	ModelUsageSortCreatedAsc  = "created_at_asc"
)

// ModelUsagePageOptions is the bounded filter surface for the administrator
// usage list. Filter values are always bound parameters in PostgreSQL.
type ModelUsagePageOptions struct {
	Offset        int
	Limit         int
	Product       controlplane.ProductCode
	Provider      string
	Model         string
	UserID        string
	DeviceID      string
	RequestID     string
	CreatedAfter  *time.Time
	CreatedBefore *time.Time
	Sort          string
}

func NormalizeModelUsagePageOptions(options ModelUsagePageOptions) (ModelUsagePageOptions, error) {
	options.Product = controlplane.ProductCode(strings.TrimSpace(string(options.Product)))
	if options.Product != "" && !options.Product.Valid() {
		return ModelUsagePageOptions{}, errors.New("model usage product filter is not supported")
	}
	options.Provider = strings.TrimSpace(options.Provider)
	options.Model = strings.TrimSpace(options.Model)
	options.UserID = strings.TrimSpace(options.UserID)
	options.DeviceID = strings.TrimSpace(options.DeviceID)
	options.RequestID = strings.TrimSpace(options.RequestID)
	options.Sort = strings.TrimSpace(options.Sort)
	if options.Sort == "" {
		options.Sort = ModelUsageSortCreatedDesc
	}
	if err := validatePageWindow(options.Offset, options.Limit); err != nil {
		return ModelUsagePageOptions{}, err
	}
	if len(options.Provider) > 64 || len(options.Model) > 128 || len(options.UserID) > 64 || len(options.DeviceID) > 64 || len(options.RequestID) > 128 {
		return ModelUsagePageOptions{}, errors.New("model usage filter is too long")
	}
	if options.CreatedAfter != nil {
		value := options.CreatedAfter.UTC()
		options.CreatedAfter = &value
	}
	if options.CreatedBefore != nil {
		value := options.CreatedBefore.UTC()
		options.CreatedBefore = &value
	}
	if options.CreatedAfter != nil && options.CreatedBefore != nil && options.CreatedAfter.After(*options.CreatedBefore) {
		return ModelUsagePageOptions{}, errors.New("model usage time range is invalid")
	}
	switch options.Sort {
	case ModelUsageSortCreatedDesc, ModelUsageSortCreatedAsc:
		return options, nil
	default:
		return ModelUsagePageOptions{}, errors.New("model usage sort is not supported")
	}
}

func modelUsageMatchesPageOptions(item controlplane.ModelUsageRecord, options ModelUsagePageOptions, userID, deviceID string) bool {
	if options.Product != "" && item.Product != options.Product {
		return false
	}
	if options.Provider != "" && item.Provider != options.Provider {
		return false
	}
	if options.Model != "" && item.Model != options.Model {
		return false
	}
	if options.UserID != "" && userID != options.UserID {
		return false
	}
	if options.DeviceID != "" && deviceID != options.DeviceID {
		return false
	}
	if options.RequestID != "" && item.RequestID != options.RequestID {
		return false
	}
	if options.CreatedAfter == nil && options.CreatedBefore == nil {
		return true
	}
	createdAt, err := time.Parse(time.RFC3339, item.CreatedAt)
	if err != nil {
		return false
	}
	if options.CreatedAfter != nil && createdAt.Before(*options.CreatedAfter) {
		return false
	}
	return options.CreatedBefore == nil || !createdAt.After(*options.CreatedBefore)
}

func compareModelUsageCreatedAt(a, b string) int {
	parsedA, errA := time.Parse(time.RFC3339, a)
	parsedB, errB := time.Parse(time.RFC3339, b)
	if errA == nil && errB == nil {
		if parsedA.Before(parsedB) {
			return -1
		}
		if parsedA.After(parsedB) {
			return 1
		}
		return 0
	}
	return strings.Compare(a, b)
}

func sortModelUsageRecords(items []controlplane.ModelUsageRecord, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.ModelUsageRecord) int {
		if a.CreatedAt != b.CreatedAt {
			createdAtOrder := compareModelUsageCreatedAt(a.CreatedAt, b.CreatedAt)
			if sortKey == ModelUsageSortCreatedAsc {
				return createdAtOrder
			}
			return -createdAtOrder
		}
		if sortKey == ModelUsageSortCreatedAsc {
			return strings.Compare(a.ID, b.ID)
		}
		return strings.Compare(b.ID, a.ID)
	})
}
