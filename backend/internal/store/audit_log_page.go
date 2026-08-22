package store

import (
	"errors"
	"slices"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

const (
	AuditLogSortCreatedDesc = "created_at_desc"
	AuditLogSortCreatedAsc  = "created_at_asc"
)

func NormalizeAuditLogPageOptions(options AuditLogPageOptions) (AuditLogPageOptions, error) {
	options.Product = controlplane.ProductCode(strings.TrimSpace(string(options.Product)))
	if options.Product != "" && !options.Product.Valid() {
		return AuditLogPageOptions{}, errors.New("audit log product filter is not supported")
	}
	options.ActorUserID = strings.TrimSpace(options.ActorUserID)
	options.DeviceID = strings.TrimSpace(options.DeviceID)
	options.Action = strings.TrimSpace(options.Action)
	options.TargetType = strings.TrimSpace(options.TargetType)
	options.Outcome = strings.TrimSpace(options.Outcome)
	options.ErrorCode = strings.TrimSpace(options.ErrorCode)
	options.RequestID = strings.TrimSpace(options.RequestID)
	options.Sort = strings.TrimSpace(options.Sort)
	if options.Sort == "" {
		options.Sort = AuditLogSortCreatedDesc
	}
	if err := validatePageWindow(options.Offset, options.Limit); err != nil {
		return AuditLogPageOptions{}, err
	}
	switch options.Outcome {
	case "", "success", "failure", "unknown":
	default:
		return AuditLogPageOptions{}, errors.New("audit outcome filter is not supported")
	}
	if len(options.ActorUserID) > 128 || len(options.DeviceID) > 128 || len(options.Action) > 256 || len(options.TargetType) > 128 || len(options.ErrorCode) > 128 || len(options.RequestID) > 128 {
		return AuditLogPageOptions{}, errors.New("audit log filter is too long")
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
		return AuditLogPageOptions{}, errors.New("audit log time range is invalid")
	}
	switch options.Sort {
	case AuditLogSortCreatedDesc, AuditLogSortCreatedAsc:
		return options, nil
	default:
		return AuditLogPageOptions{}, errors.New("audit log sort is not supported")
	}
}

func auditLogMatchesPageOptions(item controlplane.AuditLog, options AuditLogPageOptions) bool {
	if options.Product != "" && compatibilityStoredProduct(item.Product) != options.Product {
		return false
	}
	if options.ActorUserID != "" && item.ActorUserID != options.ActorUserID {
		return false
	}
	if options.DeviceID != "" && item.DeviceID != options.DeviceID {
		return false
	}
	if options.Action != "" && item.Action != options.Action {
		return false
	}
	if options.TargetType != "" && item.TargetType != options.TargetType {
		return false
	}
	if options.Outcome != "" && item.Outcome != options.Outcome {
		return false
	}
	if options.ErrorCode != "" && item.ErrorCode != options.ErrorCode {
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

// CompareAuditLogCreatedAt compares RFC3339 timestamps by instant so memory
// and snapshot fallbacks match PostgreSQL timestamptz ordering even when an
// input carries a non-UTC offset. Invalid values use a stable string fallback.
func CompareAuditLogCreatedAt(a, b string) int {
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

func sortAuditLogs(items []controlplane.AuditLog, sortKey string) {
	slices.SortFunc(items, func(a, b controlplane.AuditLog) int {
		if a.CreatedAt != b.CreatedAt {
			createdAtOrder := CompareAuditLogCreatedAt(a.CreatedAt, b.CreatedAt)
			if sortKey == AuditLogSortCreatedAsc {
				return createdAtOrder
			}
			return -createdAtOrder
		}
		if sortKey == AuditLogSortCreatedAsc {
			return strings.Compare(a.ID, b.ID)
		}
		return strings.Compare(b.ID, a.ID)
	})
}
