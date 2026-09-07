package httpapi

import (
	"io"
	"net/http"
	"strconv"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

type clientSyncWriteRequest struct {
	Items []controlplane.ClientSyncMutation `json:"items"`
}

type clientSyncPageResponse struct {
	RequestID string `json:"request_id"`
	controlplane.ClientSyncPage
}

type clientSyncWriteResponse struct {
	RequestID string `json:"request_id"`
	controlplane.ClientSyncWriteResult
}

func registerClientSyncRoutes(mux *http.ServeMux, svc *service.ControlPlane, auth *authenticator, repository store.ClientSyncRepository) {
	mux.Handle("GET /api/v1/client/sync/items", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		scope, err := authorizedClientSyncScope(r, svc, auth, actor)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		afterRevision, limit, err := clientSyncPageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if repository == nil {
			writeError(w, r, http.StatusServiceUnavailable, "CLIENT_SYNC_UNAVAILABLE", "用户资产同步暂不可用")
			return
		}
		page, err := repository.ListClientSyncItems(r.Context(), scope, afterRevision, limit)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, clientSyncPageResponse{RequestID: RequestIDFromContext(r.Context()), ClientSyncPage: page})
	}))

	mux.Handle("POST /api/v1/client/sync/items", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		scope, err := authorizedClientSyncScope(r, svc, auth, actor)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		var input clientSyncWriteRequest
		if err := decodeClientSyncBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrClientSyncSchemaInvalid)
			return
		}
		if err := controlplane.ValidateClientSyncMutations(input.Items); err != nil {
			writeAppError(w, r, err)
			return
		}
		if repository == nil {
			writeError(w, r, http.StatusServiceUnavailable, "CLIENT_SYNC_UNAVAILABLE", "用户资产同步暂不可用")
			return
		}
		result, err := repository.WriteClientSyncItems(r.Context(), scope, input.Items)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, clientSyncWriteResponse{RequestID: RequestIDFromContext(r.Context()), ClientSyncWriteResult: result})
	}))
}

func authorizedClientSyncScope(r *http.Request, svc *service.ControlPlane, auth *authenticator, actor controlplane.Actor) (store.ClientSyncScope, error) {
	if actor.Product != controlplane.ProductDouyinDesktop {
		return store.ClientSyncScope{}, controlplane.ErrForbidden
	}
	deviceID := auth.deviceID(r)
	if deviceID == "" {
		return store.ClientSyncScope{}, controlplane.ErrDeviceBindingRequired
	}
	if _, err := svc.GetClientProfileForProduct(r.Context(), actor.UserID, deviceID, actor.Product); err != nil {
		return store.ClientSyncScope{}, err
	}
	return store.ClientSyncScope{Product: actor.Product, UserID: actor.UserID, DeviceID: deviceID}, nil
}

func clientSyncPageParams(r *http.Request) (int64, int, error) {
	query := r.URL.Query()
	for key := range query {
		if key != "after_revision" && key != "limit" {
			return 0, 0, controlplane.ErrClientSyncSchemaInvalid
		}
	}
	afterRevision := int64(0)
	if values, exists := query["after_revision"]; exists {
		if len(values) != 1 {
			return 0, 0, controlplane.ErrClientSyncSchemaInvalid
		}
		parsed, err := strconv.ParseInt(values[0], 10, 64)
		if err != nil || parsed < 0 {
			return 0, 0, controlplane.ErrClientSyncSchemaInvalid
		}
		afterRevision = parsed
	}
	limit := controlplane.MaxClientSyncPageItems
	if values, exists := query["limit"]; exists {
		if len(values) != 1 {
			return 0, 0, controlplane.ErrClientSyncSchemaInvalid
		}
		parsed, err := strconv.Atoi(values[0])
		if err != nil || parsed < 1 || parsed > controlplane.MaxClientSyncPageItems {
			return 0, 0, controlplane.ErrClientSyncSchemaInvalid
		}
		limit = parsed
	}
	return afterRevision, limit, nil
}

func decodeClientSyncBody(r *http.Request, target any) error {
	if r.Body == nil {
		return controlplane.ErrClientSyncSchemaInvalid
	}
	const maxBodyBytes = controlplane.MaxClientSyncBatchItems*controlplane.MaxClientSyncItemBytes + 1024
	raw, err := io.ReadAll(io.LimitReader(r.Body, maxBodyBytes+1))
	if err != nil || len(raw) > maxBodyBytes {
		return controlplane.ErrClientSyncSchemaInvalid
	}
	return decodeJSONBytes(raw, target)
}
