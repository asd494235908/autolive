package httpapi

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestClientSyncGETUsesAuthenticatedScopeAndRevisionPage(t *testing.T) {
	repository := newClientSyncHTTPRepository()
	repository.listResult = controlplane.ClientSyncPage{
		Items:      []controlplane.ClientSyncItem{{Kind: controlplane.ClientSyncKindPersonaVersion, ItemID: "global-persona-v1", Revision: 6, Payload: json.RawMessage(`{"version":1,"content":{},"created_at":"2026-09-05T01:02:03Z"}`)}},
		NextCursor: 6, HasMore: true, ServerRevision: 9,
	}
	handler, token, userID, deviceID := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductDouyinDesktop)

	response := doJSON(t, handler, http.MethodGet, "/api/v1/client/sync/items?after_revision=5&limit=1", nil, token, "")
	if response.Code != http.StatusOK {
		t.Fatalf("GET sync status = %d; body=%s", response.Code, response.Body.String())
	}
	if repository.listScope != (store.ClientSyncScope{Product: controlplane.ProductDouyinDesktop, UserID: userID, DeviceID: deviceID}) || repository.listAfter != 5 || repository.listLimit != 1 {
		t.Fatalf("list call = scope %+v after %d limit %d", repository.listScope, repository.listAfter, repository.listLimit)
	}
	var payload struct {
		RequestID string `json:"request_id"`
		controlplane.ClientSyncPage
	}
	decodeJSON(t, response.Body.Bytes(), &payload)
	if payload.RequestID == "" || payload.NextCursor != 6 || !payload.HasMore || payload.ServerRevision != 9 || len(payload.Items) != 1 {
		t.Fatalf("GET sync response = %+v", payload)
	}
}

func TestClientSyncPOSTUsesAuthenticatedScopeAndReturnsReceipts(t *testing.T) {
	repository := newClientSyncHTTPRepository()
	repository.writeResult = controlplane.ClientSyncWriteResult{Items: []controlplane.ClientSyncReceipt{{MutationID: "mutation-1", Kind: controlplane.ClientSyncKindPersonaVersion, ItemID: "global-persona-v1", Revision: 1}}, ServerRevision: 1}
	handler, token, userID, deviceID := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductDouyinDesktop)
	body := map[string]any{"items": []any{map[string]any{
		"mutation_id": "mutation-1", "kind": "persona_version", "item_id": "global-persona-v1", "base_revision": 0, "deleted": false,
		"payload": map[string]any{"version": 1, "content": map[string]any{}, "created_at": "2026-09-05T01:02:03Z"},
	}}}

	response := doJSON(t, handler, http.MethodPost, "/api/v1/client/sync/items", body, token, "")
	if response.Code != http.StatusOK {
		t.Fatalf("POST sync status = %d; body=%s", response.Code, response.Body.String())
	}
	if repository.writeScope != (store.ClientSyncScope{Product: controlplane.ProductDouyinDesktop, UserID: userID, DeviceID: deviceID}) || len(repository.writeMutations) != 1 {
		t.Fatalf("write call = scope %+v mutations %+v", repository.writeScope, repository.writeMutations)
	}
	if repository.writeMutations[0].MutationID != "mutation-1" || repository.writeMutations[0].BaseRevision != 0 {
		t.Fatalf("write mutation = %+v", repository.writeMutations[0])
	}
}

func TestClientSyncRejectsBodyScopeSecretAndInvalidPagination(t *testing.T) {
	repository := newClientSyncHTTPRepository()
	handler, token, _, _ := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductDouyinDesktop)
	secretBody := map[string]any{"items": []any{map[string]any{
		"mutation_id": "mutation-1", "kind": "persona_version", "item_id": "global-persona-v1", "base_revision": 0, "deleted": false,
		"payload": map[string]any{"version": 1, "content": map[string]any{"api-key": "secret"}, "created_at": "2026-09-05T01:02:03Z"},
	}}}

	tests := []struct {
		name   string
		method string
		path   string
		body   any
	}{
		{name: "body user scope", method: http.MethodPost, path: "/api/v1/client/sync/items", body: map[string]any{"user_id": "other", "items": []any{}}},
		{name: "secret payload", method: http.MethodPost, path: "/api/v1/client/sync/items", body: secretBody},
		{name: "limit over max", method: http.MethodGet, path: "/api/v1/client/sync/items?after_revision=0&limit=201"},
		{name: "negative cursor", method: http.MethodGet, path: "/api/v1/client/sync/items?after_revision=-1&limit=1"},
		{name: "repeated cursor", method: http.MethodGet, path: "/api/v1/client/sync/items?after_revision=0&after_revision=1&limit=1"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			response := doJSON(t, handler, test.method, test.path, test.body, token, "")
			if response.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want %d; body=%s", response.Code, http.StatusBadRequest, response.Body.String())
			}
		})
	}
	if repository.listCalls != 0 || repository.writeCalls != 0 {
		t.Fatalf("invalid requests reached repository: list=%d write=%d", repository.listCalls, repository.writeCalls)
	}
}

func TestClientSyncRequiresDouyinDesktopBoundActiveDevice(t *testing.T) {
	t.Run("missing bearer", func(t *testing.T) {
		repository := newClientSyncHTTPRepository()
		handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{Username: "admin", Password: testAdminPassword}, repository, store.NewMemorySecretStore(), nil, true)
		response := doJSON(t, handler, http.MethodGet, "/api/v1/client/sync/items", nil, "", "")
		assertAuthSessionErrorCode(t, response, http.StatusUnauthorized, "UNAUTHENTICATED")
	})

	t.Run("autolive product", func(t *testing.T) {
		repository := newClientSyncHTTPRepository()
		handler, token, _, _ := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductAutoLive)
		response := doJSON(t, handler, http.MethodGet, "/api/v1/client/sync/items", nil, token, "")
		assertAuthSessionErrorCode(t, response, http.StatusForbidden, controlplane.ErrForbidden.Code)
	})

	t.Run("disabled device", func(t *testing.T) {
		repository := newClientSyncHTTPRepository()
		handler, token, _, deviceID := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductDouyinDesktop)
		if err := repository.Run(context.Background(), func(state *store.State) error {
			device := state.Devices[deviceID]
			device.Status = controlplane.DeviceStatusDisabled
			state.Devices[deviceID] = device
			return nil
		}); err != nil {
			t.Fatalf("disable device: %v", err)
		}
		response := doJSON(t, handler, http.MethodGet, "/api/v1/client/sync/items", nil, token, "")
		if response.Code != http.StatusForbidden {
			t.Fatalf("disabled device status = %d; body=%s", response.Code, response.Body.String())
		}
	})
}

func TestClientSyncMapsCASAndMutationConflicts(t *testing.T) {
	for _, test := range []struct {
		name string
		err  error
		code string
	}{
		{name: "CAS", err: controlplane.ErrClientSyncConflict, code: "SYNC_CONFLICT"},
		{name: "mutation", err: controlplane.ErrClientSyncMutationConflict, code: "SYNC_MUTATION_CONFLICT"},
	} {
		t.Run(test.name, func(t *testing.T) {
			repository := newClientSyncHTTPRepository()
			repository.writeErr = test.err
			handler, token, _, _ := newAuthorizedClientSyncRouter(t, repository, controlplane.ProductDouyinDesktop)
			response := doJSON(t, handler, http.MethodPost, "/api/v1/client/sync/items", map[string]any{"items": []any{map[string]any{
				"mutation_id": "mutation-1", "kind": "persona_version", "item_id": "global-persona-v1", "base_revision": 0, "deleted": true,
			}}}, token, "")
			assertAuthSessionErrorCode(t, response, http.StatusConflict, test.code)
		})
	}
}

type clientSyncHTTPRepository struct {
	*store.MemoryStore
	listResult     controlplane.ClientSyncPage
	listErr        error
	listScope      store.ClientSyncScope
	listAfter      int64
	listLimit      int
	listCalls      int
	writeResult    controlplane.ClientSyncWriteResult
	writeErr       error
	writeScope     store.ClientSyncScope
	writeMutations []controlplane.ClientSyncMutation
	writeCalls     int
}

func newClientSyncHTTPRepository() *clientSyncHTTPRepository {
	return &clientSyncHTTPRepository{MemoryStore: store.NewMemoryStore(time.Now)}
}

func (r *clientSyncHTTPRepository) ListClientSyncItems(_ context.Context, scope store.ClientSyncScope, afterRevision int64, limit int) (controlplane.ClientSyncPage, error) {
	r.listCalls++
	r.listScope, r.listAfter, r.listLimit = scope, afterRevision, limit
	return r.listResult, r.listErr
}

func (r *clientSyncHTTPRepository) WriteClientSyncItems(_ context.Context, scope store.ClientSyncScope, mutations []controlplane.ClientSyncMutation) (controlplane.ClientSyncWriteResult, error) {
	r.writeCalls++
	r.writeScope = scope
	r.writeMutations = append([]controlplane.ClientSyncMutation(nil), mutations...)
	return r.writeResult, r.writeErr
}

func newAuthorizedClientSyncRouter(t *testing.T, repository *clientSyncHTTPRepository, product controlplane.ProductCode) (http.Handler, string, string, string) {
	t.Helper()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{Username: "admin", Password: testAdminPassword}, repository, store.NewMemorySecretStore(), nil, true)
	admin := loginTokensAtPathForTest(t, handler, "/api/v1/auth/login", `{"username":"admin","password":"correct-password","product":"`+string(product)+`"}`)
	clientToken, userID := createDesktopUserForProductForTest(t, handler, admin.AccessToken, "sync-user-"+string(product), product)
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Products[string(product)] = controlplane.ProductSummary{Code: product, Status: "active"}
		state.UserProducts[userID+":"+string(product)] = controlplane.UserProductMembership{UserID: userID, Product: product, Status: "active"}
		return nil
	}); err != nil {
		t.Fatalf("seed product membership: %v", err)
	}
	created := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{"user_id": userID, "expires_at": time.Now().UTC().Add(time.Hour).Format(time.RFC3339), "max_devices": 1}, admin.AccessToken, "sync-code-"+string(product))
	if created.Code != http.StatusCreated {
		t.Fatalf("create activation status = %d; body=%s", created.Code, created.Body.String())
	}
	deviceID := "device-sync-" + string(product)
	activated := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", map[string]any{"device": map[string]any{
		"product": product, "device_id": deviceID, "device_name": "Sync Device", "platform": "windows", "app_version": "1.0.0",
	}}, clientToken, "sync-activate-"+string(product))
	if activated.Code != http.StatusOK {
		t.Fatalf("activate status = %d; body=%s", activated.Code, activated.Body.String())
	}
	return handler, clientToken, userID, deviceID
}
