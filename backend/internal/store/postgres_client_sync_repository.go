package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ClientSyncRepository = (*PostgresRepository)(nil)

func (s *PostgresRepository) ListClientSyncItems(ctx context.Context, scope ClientSyncScope, afterRevision int64, limit int) (controlplane.ClientSyncPage, error) {
	if err := validateClientSyncScope(scope); err != nil || afterRevision < 0 || limit < 1 || limit > controlplane.MaxClientSyncPageItems {
		return controlplane.ClientSyncPage{}, controlplane.ErrClientSyncSchemaInvalid
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ClientSyncPage{}, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, &sql.TxOptions{ReadOnly: true, Isolation: sql.LevelRepeatableRead})
	if err != nil {
		return controlplane.ClientSyncPage{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := s.authorizeClientSyncScope(operationCtx, tx, scope); err != nil {
		return controlplane.ClientSyncPage{}, err
	}

	var serverRevision int64
	err = tx.QueryRowContext(operationCtx, `
		SELECT current_revision FROM client_sync_workspaces
		WHERE product = $1 AND user_id = $2
	`, scope.Product, scope.UserID).Scan(&serverRevision)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return controlplane.ClientSyncPage{}, postgresOperationError(operationCtx, fmt.Errorf("read client sync workspace: %w", err))
	}

	rows, err := tx.QueryContext(operationCtx, `
		SELECT kind, item_id, revision, payload_jsonb, deleted, updated_by_device_id, updated_at FROM client_sync_items
		WHERE product = $1 AND user_id = $2 AND revision > $3
		ORDER BY revision
		LIMIT $4
	`, scope.Product, scope.UserID, afterRevision, limit+1)
	if err != nil {
		return controlplane.ClientSyncPage{}, postgresOperationError(operationCtx, fmt.Errorf("list client sync items: %w", err))
	}
	defer rows.Close()

	items := make([]controlplane.ClientSyncItem, 0, limit+1)
	for rows.Next() {
		var item controlplane.ClientSyncItem
		var payload []byte
		var updatedAt sql.NullTime
		if err := rows.Scan(&item.Kind, &item.ItemID, &item.Revision, &payload, &item.Deleted, &item.UpdatedByDeviceID, &updatedAt); err != nil {
			return controlplane.ClientSyncPage{}, postgresOperationError(operationCtx, fmt.Errorf("scan client sync item: %w", err))
		}
		if !item.Deleted {
			item.Payload = append(json.RawMessage(nil), payload...)
		}
		if updatedAt.Valid {
			item.UpdatedAt = updatedAt.Time.UTC().Format(timeFormatRFC3339)
		}
		items = append(items, item)
	}
	if err := rows.Err(); err != nil {
		return controlplane.ClientSyncPage{}, postgresOperationError(operationCtx, fmt.Errorf("iterate client sync items: %w", err))
	}

	hasMore := len(items) > limit
	if hasMore {
		items = items[:limit]
	}
	nextCursor := afterRevision
	if len(items) > 0 {
		nextCursor = items[len(items)-1].Revision
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ClientSyncPage{}, postgresCommitError(operationCtx, "commit client sync page", err)
	}
	return controlplane.ClientSyncPage{Items: items, NextCursor: nextCursor, HasMore: hasMore, ServerRevision: serverRevision}, nil
}

func (s *PostgresRepository) WriteClientSyncItems(ctx context.Context, scope ClientSyncScope, mutations []controlplane.ClientSyncMutation) (controlplane.ClientSyncWriteResult, error) {
	if err := validateClientSyncScope(scope); err != nil {
		return controlplane.ClientSyncWriteResult{}, err
	}
	if err := controlplane.ValidateClientSyncMutations(mutations); err != nil {
		return controlplane.ClientSyncWriteResult{}, err
	}
	if err := ctx.Err(); err != nil {
		return controlplane.ClientSyncWriteResult{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.ClientSyncWriteResult{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := s.authorizeClientSyncScope(operationCtx, tx, scope); err != nil {
		return controlplane.ClientSyncWriteResult{}, err
	}
	now := s.Now()
	if _, err := tx.ExecContext(operationCtx, `
		INSERT INTO client_sync_workspaces (product, user_id, current_revision, created_at, updated_at)
		VALUES ($1, $2, 0, $3, $3)
		ON CONFLICT (product, user_id) DO NOTHING
	`, scope.Product, scope.UserID, now); err != nil {
		return controlplane.ClientSyncWriteResult{}, postgresOperationError(operationCtx, fmt.Errorf("ensure client sync workspace: %w", err))
	}
	var currentRevision int64
	if err := tx.QueryRowContext(operationCtx, `
		SELECT current_revision FROM client_sync_workspaces
		WHERE product = $1 AND user_id = $2
		FOR UPDATE
	`, scope.Product, scope.UserID).Scan(&currentRevision); err != nil {
		return controlplane.ClientSyncWriteResult{}, postgresOperationError(operationCtx, fmt.Errorf("lock client sync workspace: %w", err))
	}

	receipts := make([]controlplane.ClientSyncReceipt, 0, len(mutations))
	for _, mutation := range mutations {
		receipt, replayed, err := s.applyClientSyncMutation(operationCtx, tx, scope, mutation, &currentRevision, now)
		if err != nil {
			return controlplane.ClientSyncWriteResult{}, err
		}
		receipts = append(receipts, receipt)
		if replayed {
			continue
		}
	}
	if err := tx.Commit(); err != nil {
		return controlplane.ClientSyncWriteResult{}, postgresCommitError(operationCtx, "commit client sync items", err)
	}
	return controlplane.ClientSyncWriteResult{Items: receipts, ServerRevision: currentRevision}, nil
}

func (s *PostgresRepository) applyClientSyncMutation(ctx context.Context, tx *sql.Tx, scope ClientSyncScope, mutation controlplane.ClientSyncMutation, currentRevision *int64, now time.Time) (controlplane.ClientSyncReceipt, bool, error) {
	requestHash, err := controlplane.ClientSyncMutationHash(mutation)
	if err != nil {
		return controlplane.ClientSyncReceipt{}, false, err
	}
	var storedHash string
	var storedResponse []byte
	err = tx.QueryRowContext(ctx, `
		SELECT request_hash, response_jsonb FROM client_sync_mutations
		WHERE product = $1 AND user_id = $2 AND device_id = $3 AND mutation_id = $4
	`, scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).Scan(&storedHash, &storedResponse)
	if err == nil {
		if storedHash != requestHash {
			return controlplane.ClientSyncReceipt{}, false, controlplane.ErrClientSyncMutationConflict
		}
		var receipt controlplane.ClientSyncReceipt
		if err := json.Unmarshal(storedResponse, &receipt); err != nil {
			return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("decode client sync receipt: %w", err))
		}
		return receipt, true, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("read client sync mutation: %w", err))
	}

	var storedRevision int64
	err = tx.QueryRowContext(ctx, `
		SELECT revision FROM client_sync_items
		WHERE product = $1 AND user_id = $2 AND kind = $3 AND item_id = $4
	`, scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).Scan(&storedRevision)
	switch {
	case errors.Is(err, sql.ErrNoRows) && mutation.Deleted:
		return controlplane.ClientSyncReceipt{}, false, controlplane.ErrClientSyncConflict
	case errors.Is(err, sql.ErrNoRows) && mutation.BaseRevision != 0:
		return controlplane.ClientSyncReceipt{}, false, controlplane.ErrClientSyncConflict
	case err == nil && mutation.BaseRevision != storedRevision:
		return controlplane.ClientSyncReceipt{}, false, controlplane.ErrClientSyncConflict
	case err != nil && !errors.Is(err, sql.ErrNoRows):
		return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("read client sync item revision: %w", err))
	}
	if err := s.validateClientSyncReferences(ctx, tx, scope, mutation); err != nil {
		return controlplane.ClientSyncReceipt{}, false, err
	}

	*currentRevision = *currentRevision + 1
	if _, err := tx.ExecContext(ctx, `
		UPDATE client_sync_workspaces SET current_revision = $3, updated_at = $4
		WHERE product = $1 AND user_id = $2
	`, scope.Product, scope.UserID, *currentRevision, now); err != nil {
		return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("advance client sync revision: %w", err))
	}
	payload := json.RawMessage(`{}`)
	if !mutation.Deleted {
		payload, err = controlplane.CanonicalClientSyncPayload(mutation.Payload)
		if err != nil {
			return controlplane.ClientSyncReceipt{}, false, err
		}
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO client_sync_items (product, user_id, kind, item_id, revision, payload_jsonb, deleted, updated_by_device_id, updated_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
		ON CONFLICT (product, user_id, kind, item_id) DO UPDATE SET
			revision = EXCLUDED.revision,
			payload_jsonb = EXCLUDED.payload_jsonb,
			deleted = EXCLUDED.deleted,
			updated_by_device_id = EXCLUDED.updated_by_device_id,
			updated_at = EXCLUDED.updated_at
	`, scope.Product, scope.UserID, mutation.Kind, mutation.ItemID, *currentRevision, []byte(payload), mutation.Deleted, scope.DeviceID, now); err != nil {
		return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("upsert client sync item: %w", err))
	}
	receipt := controlplane.ClientSyncReceipt{MutationID: mutation.MutationID, Kind: mutation.Kind, ItemID: mutation.ItemID, Revision: *currentRevision, Deleted: mutation.Deleted}
	receiptJSON, err := json.Marshal(receipt)
	if err != nil {
		return controlplane.ClientSyncReceipt{}, false, err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO client_sync_mutations (product, user_id, device_id, mutation_id, request_hash, response_jsonb, created_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
	`, scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID, requestHash, receiptJSON, now); err != nil {
		return controlplane.ClientSyncReceipt{}, false, postgresOperationError(ctx, fmt.Errorf("store client sync mutation receipt: %w", err))
	}
	return receipt, false, nil
}

func (s *PostgresRepository) validateClientSyncReferences(ctx context.Context, tx *sql.Tx, scope ClientSyncScope, mutation controlplane.ClientSyncMutation) error {
	if mutation.Deleted {
		var hasChildren bool
		var err error
		switch mutation.Kind {
		case controlplane.ClientSyncKindKnowledgeSource:
			err = tx.QueryRowContext(ctx, `SELECT EXISTS (
				SELECT 1 FROM client_sync_items
				WHERE product=$1 AND user_id=$2 AND kind='knowledge_document' AND deleted=FALSE
				  AND payload_jsonb->>'source_id'=$3
			)`, scope.Product, scope.UserID, mutation.ItemID).Scan(&hasChildren)
		case controlplane.ClientSyncKindKnowledgeDocument:
			err = tx.QueryRowContext(ctx, `SELECT EXISTS (
				SELECT 1 FROM client_sync_items
				WHERE product=$1 AND user_id=$2 AND deleted=FALSE AND (
				  (kind='knowledge_chunk' AND payload_jsonb->>'document_id'=$3) OR
				  (kind='knowledge_rule' AND payload_jsonb->>'document_id'=$3)
				)
			)`, scope.Product, scope.UserID, mutation.ItemID).Scan(&hasChildren)
		case controlplane.ClientSyncKindKnowledgeChunk:
			err = tx.QueryRowContext(ctx, `SELECT EXISTS (
				SELECT 1 FROM client_sync_items
				WHERE product=$1 AND user_id=$2 AND kind='knowledge_rule' AND deleted=FALSE
				  AND payload_jsonb->'record_ids' ? $3
			)`, scope.Product, scope.UserID, mutation.ItemID).Scan(&hasChildren)
		default:
			return nil
		}
		if err != nil {
			return postgresOperationError(ctx, fmt.Errorf("validate client sync delete references: %w", err))
		}
		if hasChildren {
			return controlplane.ErrClientSyncConflict
		}
		return nil
	}

	var payload struct {
		SourceID   string   `json:"source_id"`
		DocumentID string   `json:"document_id"`
		RecordIDs  []string `json:"record_ids"`
		Enabled    bool     `json:"enabled"`
	}
	switch mutation.Kind {
	case controlplane.ClientSyncKindKnowledgeDocument,
		controlplane.ClientSyncKindKnowledgeChunk,
		controlplane.ClientSyncKindKnowledgeRule:
		if err := json.Unmarshal(mutation.Payload, &payload); err != nil {
			return controlplane.ErrClientSyncSchemaInvalid
		}
	default:
		return nil
	}

	parentKind := controlplane.ClientSyncKindKnowledgeSource
	parentID := payload.SourceID
	if mutation.Kind != controlplane.ClientSyncKindKnowledgeDocument {
		parentKind = controlplane.ClientSyncKindKnowledgeDocument
		parentID = payload.DocumentID
	}
	var parentExists bool
	if err := tx.QueryRowContext(ctx, `SELECT EXISTS (
		SELECT 1 FROM client_sync_items
		WHERE product=$1 AND user_id=$2 AND kind=$3 AND item_id=$4 AND deleted=FALSE
	)`, scope.Product, scope.UserID, parentKind, parentID).Scan(&parentExists); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("validate client sync parent: %w", err))
	}
	if !parentExists {
		return controlplane.ErrClientSyncSchemaInvalid
	}
	if mutation.Kind == controlplane.ClientSyncKindKnowledgeChunk {
		var breaksRule bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (
			SELECT 1 FROM client_sync_items
			WHERE product=$1 AND user_id=$2 AND kind='knowledge_rule' AND deleted=FALSE
			  AND payload_jsonb->'record_ids' ? $3
			  AND (payload_jsonb->>'document_id'<>$4 OR $5=FALSE)
		)`, scope.Product, scope.UserID, mutation.ItemID, payload.DocumentID, payload.Enabled).Scan(&breaksRule); err != nil {
			return postgresOperationError(ctx, fmt.Errorf("validate client sync chunk references: %w", err))
		}
		if breaksRule {
			return controlplane.ErrClientSyncSchemaInvalid
		}
		return nil
	}
	if mutation.Kind != controlplane.ClientSyncKindKnowledgeRule {
		return nil
	}
	for _, recordID := range payload.RecordIDs {
		var recordExists bool
		if err := tx.QueryRowContext(ctx, `SELECT EXISTS (
			SELECT 1 FROM client_sync_items
			WHERE product=$1 AND user_id=$2 AND kind='knowledge_chunk' AND item_id=$3
			  AND deleted=FALSE AND payload_jsonb->>'document_id'=$4
			  AND payload_jsonb->>'enabled'='true'
		)`, scope.Product, scope.UserID, recordID, payload.DocumentID).Scan(&recordExists); err != nil {
			return postgresOperationError(ctx, fmt.Errorf("validate client sync rule record: %w", err))
		}
		if !recordExists {
			return controlplane.ErrClientSyncSchemaInvalid
		}
	}
	return nil
}

func (s *PostgresRepository) authorizeClientSyncScope(ctx context.Context, tx *sql.Tx, scope ClientSyncScope) error {
	var authorized bool
	err := tx.QueryRowContext(ctx, `
		SELECT EXISTS (
			SELECT 1
			FROM devices AS d
			JOIN user_products AS up ON up.user_id = d.user_id AND up.product = d.product
			JOIN products AS p ON p.code = d.product
			JOIN activation_device_bindings AS adb ON adb.device_id = d.id AND adb.product = d.product AND adb.user_id = d.user_id
			JOIN activation_codes AS ac ON ac.id = adb.activation_code_id AND ac.product = adb.product AND ac.bound_user_id = adb.user_id
			WHERE d.product = $1 AND d.user_id = $2 AND d.id = $3
			  AND d.status = 'active' AND up.status = 'active' AND p.status = 'active'
			  AND ac.status IN ('active', 'used') AND ac.expires_at > $4
		)
	`, scope.Product, scope.UserID, scope.DeviceID, s.Now()).Scan(&authorized)
	if err != nil {
		return postgresOperationError(ctx, fmt.Errorf("authorize client sync scope: %w", err))
	}
	if !authorized {
		return controlplane.ErrDeviceBindingRequired
	}
	return nil
}

func validateClientSyncScope(scope ClientSyncScope) error {
	if scope.Product != controlplane.ProductDouyinDesktop || !controlplane.ValidClientSyncIdentifier(scope.UserID) || !controlplane.ValidClientSyncIdentifier(scope.DeviceID) {
		return controlplane.ErrClientSyncSchemaInvalid
	}
	return nil
}

const timeFormatRFC3339 = "2006-01-02T15:04:05Z07:00"
