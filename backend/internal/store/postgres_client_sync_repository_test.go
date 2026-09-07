package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresClientSyncWriteAllocatesRevisionAndPersistsReceipt(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := clientSyncTestMutation()

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(0)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).WillReturnError(sql.ErrNoRows)
	mock.ExpectExec(regexp.QuoteMeta("UPDATE client_sync_workspaces SET current_revision = $3, updated_at = $4")).WithArgs(scope.Product, scope.UserID, int64(1), now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID, int64(1), sqlmock.AnyArg(), false, scope.DeviceID, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID, sqlmock.AnyArg(), sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	result, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if err != nil {
		t.Fatalf("WriteClientSyncItems() error = %v", err)
	}
	if result.ServerRevision != 1 || len(result.Items) != 1 || result.Items[0].Revision != 1 {
		t.Fatalf("write result = %+v", result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRollsBackEarlierMutationWhenLaterCASConflicts(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	first := clientSyncTestMutation()
	second := clientSyncTestMutation()
	second.MutationID = "mutation-2"
	second.ItemID = "global-persona-v2"
	second.Payload = json.RawMessage(`{"version":2,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`)
	second.BaseRevision = 1

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(3)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, first.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, first.Kind, first.ItemID).WillReturnError(sql.ErrNoRows)
	mock.ExpectExec(regexp.QuoteMeta("UPDATE client_sync_workspaces SET current_revision = $3, updated_at = $4")).WithArgs(scope.Product, scope.UserID, int64(4), now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_items")).WithArgs(scope.Product, scope.UserID, first.Kind, first.ItemID, int64(4), sqlmock.AnyArg(), false, scope.DeviceID, now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, first.MutationID, sqlmock.AnyArg(), sqlmock.AnyArg(), now).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, second.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, second.Kind, second.ItemID).WillReturnRows(sqlmock.NewRows([]string{"revision"}).AddRow(int64(2)))
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{first, second})
	if !errors.Is(err, controlplane.ErrClientSyncConflict) {
		t.Fatalf("WriteClientSyncItems() error = %v, want sync conflict", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRejectsTombstoneForMissingItem(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := clientSyncTestMutation()
	mutation.Deleted = true
	mutation.Payload = nil

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(0)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).WillReturnError(sql.ErrNoRows)
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if !errors.Is(err, controlplane.ErrClientSyncConflict) {
		t.Fatalf("WriteClientSyncItems() error = %v, want sync conflict", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRejectsKnowledgeDocumentWithoutActiveSource(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := controlplane.ClientSyncMutation{
		MutationID: "mutation-document-1",
		Kind:       controlplane.ClientSyncKindKnowledgeDocument,
		ItemID:     "document-1",
		Payload:    json.RawMessage(`{"source_id":"source-1","version":1,"content_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parser_version":"v1","chunker_version":"v1","status":"parsed","metadata":{},"created_at":"2026-09-05T01:02:03Z"}`),
	}

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(0)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (")).WithArgs(scope.Product, scope.UserID, controlplane.ClientSyncKindKnowledgeSource, "source-1").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(false))
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if !errors.Is(err, controlplane.ErrClientSyncSchemaInvalid) {
		t.Fatalf("WriteClientSyncItems() error = %v, want schema invalid", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRejectsChunkMoveThatBreaksExistingRule(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := controlplane.ClientSyncMutation{
		MutationID: "mutation-chunk-move", Kind: controlplane.ClientSyncKindKnowledgeChunk,
		ItemID: "chunk-1", BaseRevision: 4,
		Payload: json.RawMessage(`{"document_id":"document-2","ordinal":0,"text":"企业软件实施知识","locator":{},"chunker_version":"v1","enabled":true,"created_at":"2026-09-05T01:02:03Z"}`),
	}

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(4)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).WillReturnRows(sqlmock.NewRows([]string{"revision"}).AddRow(int64(4)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (")).WithArgs(scope.Product, scope.UserID, controlplane.ClientSyncKindKnowledgeDocument, "document-2").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	mock.ExpectQuery("(?s)"+regexp.QuoteMeta("SELECT EXISTS (")+".*"+regexp.QuoteMeta("kind='knowledge_rule'")).WithArgs(scope.Product, scope.UserID, "chunk-1", "document-2", true).WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if !errors.Is(err, controlplane.ErrClientSyncSchemaInvalid) {
		t.Fatalf("WriteClientSyncItems() error = %v, want schema invalid", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRejectsRuleReferencingDisabledChunk(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := controlplane.ClientSyncMutation{
		MutationID: "mutation-rule-disabled", Kind: controlplane.ClientSyncKindKnowledgeRule,
		ItemID:  "rule-1",
		Payload: json.RawMessage(`{"document_id":"document-1","condition_kind":"literal_contains","condition_text":"price","reply_guidance":"answer from catalog","literal_terms":["price"],"record_ids":["chunk-1"],"rule_fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"active","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z"}`),
	}

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(0)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT revision FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, mutation.Kind, mutation.ItemID).WillReturnError(sql.ErrNoRows)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (")).WithArgs(scope.Product, scope.UserID, controlplane.ClientSyncKindKnowledgeDocument, "document-1").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
	mock.ExpectQuery("(?s)"+regexp.QuoteMeta("kind='knowledge_chunk'")+".*"+regexp.QuoteMeta("payload_jsonb->>'enabled'")).WithArgs(scope.Product, scope.UserID, "chunk-1", "document-1").WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(false))
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if !errors.Is(err, controlplane.ErrClientSyncSchemaInvalid) {
		t.Fatalf("WriteClientSyncItems() error = %v, want schema invalid", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteReturnsStoredIdempotentReceipt(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := clientSyncTestMutation()
	hash, err := controlplane.ClientSyncMutationHash(mutation)
	if err != nil {
		t.Fatalf("mutation hash error = %v", err)
	}
	receipt := controlplane.ClientSyncReceipt{MutationID: mutation.MutationID, Kind: mutation.Kind, ItemID: mutation.ItemID, Revision: 4}
	receiptJSON, err := json.Marshal(receipt)
	if err != nil {
		t.Fatalf("marshal receipt: %v", err)
	}

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(4)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnRows(sqlmock.NewRows([]string{"request_hash", "response_jsonb"}).AddRow(hash, receiptJSON))
	mock.ExpectCommit()

	result, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if err != nil {
		t.Fatalf("WriteClientSyncItems() error = %v", err)
	}
	if len(result.Items) != 1 || result.Items[0] != receipt || result.ServerRevision != 4 {
		t.Fatalf("idempotent result = %+v", result)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncWriteRejectsMutationIDWithDifferentMeaning(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()
	mutation := clientSyncTestMutation()

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO client_sync_workspaces")).WithArgs(scope.Product, scope.UserID, now).WillReturnResult(sqlmock.NewResult(0, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(4)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT request_hash, response_jsonb FROM client_sync_mutations")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, mutation.MutationID).WillReturnRows(sqlmock.NewRows([]string{"request_hash", "response_jsonb"}).AddRow("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", `{"revision":4}`))
	mock.ExpectRollback()

	_, err := repository.WriteClientSyncItems(context.Background(), scope, []controlplane.ClientSyncMutation{mutation})
	if !errors.Is(err, controlplane.ErrClientSyncMutationConflict) {
		t.Fatalf("WriteClientSyncItems() error = %v, want mutation conflict", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresClientSyncListUsesRevisionCursorAndBoundedLookahead(t *testing.T) {
	database, mock, repository, now := newClientSyncRepositoryTest(t)
	defer database.Close()
	scope := clientSyncTestScope()

	mock.ExpectBegin()
	expectValidClientSyncScope(mock, scope, now)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT current_revision FROM client_sync_workspaces")).WithArgs(scope.Product, scope.UserID).WillReturnRows(sqlmock.NewRows([]string{"current_revision"}).AddRow(int64(9)))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT kind, item_id, revision, payload_jsonb, deleted, updated_by_device_id, updated_at FROM client_sync_items")).WithArgs(scope.Product, scope.UserID, int64(5), 2).WillReturnRows(
		sqlmock.NewRows([]string{"kind", "item_id", "revision", "payload_jsonb", "deleted", "updated_by_device_id", "updated_at"}).
			AddRow(controlplane.ClientSyncKindPersonaVersion, "persona-1", int64(6), `{"content":{},"created_at":"2026-09-05T01:02:03Z","version":1}`, false, scope.DeviceID, now).
			AddRow(controlplane.ClientSyncKindMemory, "memory-1", int64(7), `{}`, true, scope.DeviceID, now),
	)
	mock.ExpectCommit()

	page, err := repository.ListClientSyncItems(context.Background(), scope, 5, 1)
	if err != nil {
		t.Fatalf("ListClientSyncItems() error = %v", err)
	}
	if len(page.Items) != 1 || page.NextCursor != 6 || !page.HasMore || page.ServerRevision != 9 {
		t.Fatalf("page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func newClientSyncRepositoryTest(t *testing.T) (*sql.DB, sqlmock.Sqlmock, *PostgresRepository, time.Time) {
	t.Helper()
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	now := time.Date(2026, 9, 5, 3, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		database.Close()
		t.Fatalf("repository constructor error = %v", err)
	}
	return database, mock, repository, now
}

func clientSyncTestScope() ClientSyncScope {
	return ClientSyncScope{Product: controlplane.ProductDouyinDesktop, UserID: "usr-1", DeviceID: "device-1"}
}

func clientSyncTestMutation() controlplane.ClientSyncMutation {
	return controlplane.ClientSyncMutation{
		MutationID: "mutation-1", Kind: controlplane.ClientSyncKindPersonaVersion, ItemID: "global-persona-v1",
		Payload: json.RawMessage(`{"version":1,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`),
	}
}

func expectValidClientSyncScope(mock sqlmock.Sqlmock, scope ClientSyncScope, now time.Time) {
	mock.ExpectQuery(regexp.QuoteMeta("SELECT EXISTS (")).WithArgs(scope.Product, scope.UserID, scope.DeviceID, now).WillReturnRows(sqlmock.NewRows([]string{"exists"}).AddRow(true))
}
