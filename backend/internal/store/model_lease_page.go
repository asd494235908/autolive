package store

import (
	"errors"
	"slices"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

const (
	ModelLeaseSortExpiresDesc   = "expires_at_desc"
	ModelLeaseSortExpiresAsc    = "expires_at_asc"
	ModelLeaseSortStatus        = "status"
	ModelLeaseSortProviderModel = "provider_model"
)

func NormalizeModelLeasePageOptions(options ModelLeasePageOptions) (ModelLeasePageOptions, error) {
	options.Product = controlplane.ProductCode(strings.TrimSpace(string(options.Product)))
	if options.Product != "" && !options.Product.Valid() {
		return ModelLeasePageOptions{}, errors.New("model lease product filter is not supported")
	}
	options.Status = strings.TrimSpace(options.Status)
	options.Provider = strings.TrimSpace(options.Provider)
	options.Model = strings.TrimSpace(options.Model)
	options.UserID = strings.TrimSpace(options.UserID)
	options.DeviceID = strings.TrimSpace(options.DeviceID)
	options.AccountID = strings.TrimSpace(options.AccountID)
	options.Sort = strings.TrimSpace(options.Sort)
	if options.Sort == "" {
		options.Sort = ModelLeaseSortExpiresDesc
	}
	if err := validatePageWindow(options.Offset, options.Limit); err != nil {
		return ModelLeasePageOptions{}, err
	}
	switch options.Status {
	case "", controlplane.ModelLeaseStatusActive, controlplane.ModelLeaseStatusReleased, controlplane.ModelLeaseStatusExpired:
	default:
		return ModelLeasePageOptions{}, errors.New("model lease status filter is not supported")
	}
	if len(options.Provider) > 64 || len(options.Model) > 128 || len(options.UserID) > 64 || len(options.DeviceID) > 64 || len(options.AccountID) > 64 {
		return ModelLeasePageOptions{}, errors.New("model lease filter is too long")
	}
	switch options.Sort {
	case ModelLeaseSortExpiresDesc, ModelLeaseSortExpiresAsc, ModelLeaseSortStatus, ModelLeaseSortProviderModel:
		return options, nil
	default:
		return ModelLeasePageOptions{}, errors.New("model lease sort is not supported")
	}
}

func compatibilityStoredProduct(product controlplane.ProductCode) controlplane.ProductCode {
	if product == "" {
		return controlplane.ProductAutoLive
	}
	return product
}

func normalizeModelLeaseSummaryStatus(item *controlplane.ModelLeaseAdminSummary, now time.Time) {
	if item == nil || item.Status != controlplane.ModelLeaseStatusActive || item.ExpiresAt == "" {
		return
	}
	expiresAt, err := time.Parse(time.RFC3339, item.ExpiresAt)
	if err == nil && !now.Before(expiresAt) {
		item.Status = controlplane.ModelLeaseStatusExpired
	}
}

func modelLeaseMatchesPageOptions(item controlplane.ModelLeaseAdminSummary, options ModelLeasePageOptions, now time.Time) bool {
	normalizeModelLeaseSummaryStatus(&item, now)
	if options.Product != "" && item.Product != options.Product {
		return false
	}
	if options.Status != "" && item.Status != options.Status {
		return false
	}
	return (options.Provider == "" || item.Provider == options.Provider) &&
		(options.Model == "" || item.Model == options.Model) &&
		(options.UserID == "" || item.UserID == options.UserID) &&
		(options.DeviceID == "" || item.DeviceID == options.DeviceID) &&
		(options.AccountID == "" || item.AccountID == options.AccountID)
}

func sortModelLeaseSummaries(items []controlplane.ModelLeaseAdminSummary, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.ModelLeaseAdminSummary) int {
		switch sortKey {
		case ModelLeaseSortExpiresAsc:
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(a.ExpiresAt, b.ExpiresAt)
			}
		case ModelLeaseSortStatus:
			if a.Status != b.Status {
				return strings.Compare(a.Status, b.Status)
			}
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(b.ExpiresAt, a.ExpiresAt)
			}
		case ModelLeaseSortProviderModel:
			if value := strings.Compare(a.Provider, b.Provider); value != 0 {
				return value
			}
			if value := strings.Compare(a.Model, b.Model); value != 0 {
				return value
			}
		default:
			if a.ExpiresAt != b.ExpiresAt {
				return strings.Compare(b.ExpiresAt, a.ExpiresAt)
			}
		}
		return strings.Compare(b.ID, a.ID)
	})
}

func modelLeaseAdminSummary(lease controlplane.ModelLease) controlplane.ModelLeaseAdminSummary {
	return controlplane.ModelLeaseAdminSummary{
		ID:               lease.ID,
		Product:          compatibilityStoredProduct(lease.Product),
		AccountID:        lease.AccountID,
		UserID:           lease.UserID,
		DeviceID:         lease.DeviceID,
		Purpose:          lease.Purpose,
		Provider:         lease.Provider,
		Model:            lease.Model,
		Status:           lease.Status,
		ExpiresAt:        lease.ExpiresAt,
		ProxyMode:        lease.ProxyMode,
		ConcurrencyLimit: lease.ConcurrencyLimit,
	}
}
