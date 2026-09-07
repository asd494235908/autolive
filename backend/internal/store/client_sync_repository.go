package store

import (
	"context"

	"autoLive/backend/internal/controlplane"
)

// ClientSyncRepository is the PostgreSQL-only boundary for authenticated
// douyin-desktop asset synchronization. It deliberately has no memory fallback.
type ClientSyncRepository interface {
	ListClientSyncItems(ctx context.Context, scope ClientSyncScope, afterRevision int64, limit int) (controlplane.ClientSyncPage, error)
	WriteClientSyncItems(ctx context.Context, scope ClientSyncScope, mutations []controlplane.ClientSyncMutation) (controlplane.ClientSyncWriteResult, error)
}

type ClientSyncScope struct {
	Product  controlplane.ProductCode
	UserID   string
	DeviceID string
}
