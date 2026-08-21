package service

import (
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

// AuditLogListOptions is the bounded, public filter surface for the admin
// audit list. Time values remain strings at the HTTP boundary and are parsed
// before reaching the repository.
type AuditLogListOptions struct {
	Product       controlplane.ProductCode
	ActorUserID   string
	DeviceID      string
	Action        string
	TargetType    string
	Outcome       string
	ErrorCode     string
	RequestID     string
	CreatedAfter  string
	CreatedBefore string
	Sort          string
}

func normalizeAuditLogListOptions(options AuditLogListOptions) (store.AuditLogPageOptions, error) {
	result := store.AuditLogPageOptions{
		Product:     controlplane.ProductCode(strings.TrimSpace(string(options.Product))),
		ActorUserID: options.ActorUserID,
		DeviceID:    options.DeviceID,
		Action:      options.Action,
		TargetType:  options.TargetType,
		Outcome:     options.Outcome,
		ErrorCode:   options.ErrorCode,
		RequestID:   options.RequestID,
		Sort:        options.Sort,
	}
	if result.Product != "" && !result.Product.Valid() {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	result.ActorUserID = strings.TrimSpace(result.ActorUserID)
	result.DeviceID = strings.TrimSpace(result.DeviceID)
	result.Action = strings.TrimSpace(result.Action)
	result.TargetType = strings.TrimSpace(result.TargetType)
	result.Outcome = strings.TrimSpace(result.Outcome)
	result.ErrorCode = strings.TrimSpace(result.ErrorCode)
	result.RequestID = strings.TrimSpace(result.RequestID)
	result.Sort = strings.TrimSpace(result.Sort)
	if len(result.ActorUserID) > 128 || len(result.DeviceID) > 128 || len(result.Action) > 256 || len(result.TargetType) > 128 || len(result.ErrorCode) > 128 || len(result.RequestID) > 128 {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	if result.Outcome != "" && result.Outcome != "success" && result.Outcome != "failure" && result.Outcome != "unknown" {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	if result.Sort != "" && result.Sort != store.AuditLogSortCreatedDesc && result.Sort != store.AuditLogSortCreatedAsc {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	createdAfter, err := parseAuditTime(options.CreatedAfter)
	if err != nil {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	createdBefore, err := parseAuditTime(options.CreatedBefore)
	if err != nil {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	result.CreatedAfter = createdAfter
	result.CreatedBefore = createdBefore
	if createdAfter != nil && createdBefore != nil && createdAfter.After(*createdBefore) {
		return store.AuditLogPageOptions{}, controlplane.ErrInvalidRequest
	}
	return result, nil
}

func parseAuditTime(value string) (*time.Time, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil, nil
	}
	parsed, err := time.Parse(time.RFC3339, value)
	if err != nil {
		return nil, err
	}
	parsed = parsed.UTC()
	return &parsed, nil
}
