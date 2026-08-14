package httpapi

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"strconv"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
)

type userEnvelope struct {
	RequestID string                   `json:"request_id"`
	User      controlplane.UserSummary `json:"user"`
}

type userListResponse struct {
	RequestID  string                     `json:"request_id"`
	Items      []controlplane.UserSummary `json:"items"`
	Pagination pagination                 `json:"pagination"`
}

type deviceEnvelope struct {
	RequestID string                     `json:"request_id"`
	Device    controlplane.DeviceSummary `json:"device"`
}

type deviceListResponse struct {
	RequestID  string                       `json:"request_id"`
	Items      []controlplane.DeviceSummary `json:"items"`
	Pagination pagination                   `json:"pagination"`
}

type activationCodeEnvelope struct {
	RequestID      string                      `json:"request_id"`
	ActivationCode controlplane.ActivationCode `json:"activation_code"`
}

type activationCodeListResponse struct {
	RequestID  string                        `json:"request_id"`
	Items      []controlplane.ActivationCode `json:"items"`
	Pagination pagination                    `json:"pagination"`
}

type modelPoolAccountEnvelope struct {
	RequestID string                               `json:"request_id"`
	Account   controlplane.ModelPoolAccountSummary `json:"account"`
}

type modelPoolResponse struct {
	RequestID string                                 `json:"request_id"`
	Accounts  []controlplane.ModelPoolAccountSummary `json:"accounts"`
}

type modelPoolConnectivityTestResponse struct {
	RequestID string `json:"request_id"`
	controlplane.ModelPoolConnectivityTestResult
}

type directLLMCallRecordResponse struct {
	RequestID string `json:"request_id"`
	Recorded  bool   `json:"recorded"`
}

type modelUsageListResponse struct {
	RequestID  string                          `json:"request_id"`
	Items      []controlplane.ModelUsageRecord `json:"items"`
	Pagination pagination                      `json:"pagination"`
}

type auditLogListResponse struct {
	RequestID  string                  `json:"request_id"`
	Items      []controlplane.AuditLog `json:"items"`
	Pagination pagination              `json:"pagination"`
}

type modelLeaseResponse struct {
	RequestID string                  `json:"request_id"`
	Lease     controlplane.ModelLease `json:"lease"`
}

type releaseModelLeaseResponse struct {
	RequestID string `json:"request_id"`
	LeaseID   string `json:"lease_id"`
	Released  bool   `json:"released"`
}

type heartbeatResponse struct {
	RequestID    string `json:"request_id"`
	AcceptedAt   string `json:"accepted_at"`
	DeviceStatus string `json:"device_status"`
}

type clientProfileResponse struct {
	RequestID   string                     `json:"request_id"`
	User        controlplane.UserSummary   `json:"user"`
	Device      controlplane.DeviceSummary `json:"device"`
	Permissions []string                   `json:"permissions"`
}

type pagination struct {
	Page     int `json:"page"`
	PageSize int `json:"page_size"`
	Total    int `json:"total"`
}

func registerControlPlaneRoutes(mux *http.ServeMux, svc *service.ControlPlane, auth *authenticator) {
	mux.Handle("GET /api/v1/admin/users", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListUsers(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		page, start, end, err := pageRange(r, len(items))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items[start:end],
			Pagination: page,
		})
	}))

	mux.Handle("POST /api/v1/admin/users", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.CreateUserInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		user, err := svc.CreateUser(r.Context(), r.Header.Get("Idempotency-Key"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusCreated, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("POST /api/v1/admin/users/{user_id}/disable", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		user, err := svc.DisableUser(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("user_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("GET /api/v1/admin/devices", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListDevices(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		page, start, end, err := pageRange(r, len(items))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items[start:end],
			Pagination: page,
		})
	}))

	mux.Handle("POST /api/v1/admin/devices/{device_id}/disable", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		device, err := svc.DisableDevice(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("device_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Device:    device,
		})
	}))

	mux.Handle("POST /api/v1/admin/devices/{device_id}/unbind", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		device, err := svc.UnbindDevice(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("device_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Device:    device,
		})
	}))

	mux.Handle("GET /api/v1/admin/activation-codes", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListActivationCodes(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		page, start, end, err := pageRange(r, len(items))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, activationCodeListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items[start:end],
			Pagination: page,
		})
	}))

	mux.Handle("POST /api/v1/admin/activation-codes", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var request struct {
			ExpiresAt  string `json:"expires_at"`
			MaxDevices int    `json:"max_devices"`
		}
		if err := decodeJSONBody(r, &request); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		expiresAt, err := time.Parse(time.RFC3339, request.ExpiresAt)
		if err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		code, err := svc.CreateActivationCode(r.Context(), r.Header.Get("Idempotency-Key"), controlplane.CreateActivationCodeInput{
			ExpiresAt:  expiresAt,
			MaxDevices: request.MaxDevices,
		})
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusCreated, activationCodeEnvelope{
			RequestID:      RequestIDFromContext(r.Context()),
			ActivationCode: code,
		})
	}))

	mux.Handle("POST /api/v1/admin/activation-codes/{code_id}/revoke", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		code, err := svc.RevokeActivationCode(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("code_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, activationCodeEnvelope{
			RequestID:      RequestIDFromContext(r.Context()),
			ActivationCode: code,
		})
	}))

	mux.Handle("GET /api/v1/admin/model-pool", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListModelPoolAccounts(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Accounts:  items,
		})
	}))

	mux.Handle("POST /api/v1/admin/model-pool", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.CreateModelPoolAccountInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		account, err := svc.CreateModelPoolAccount(r.Context(), r.Header.Get("Idempotency-Key"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusCreated, modelPoolAccountEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Account:   account,
		})
	}))

	mux.Handle("POST /api/v1/admin/model-pool/{account_id}/disable", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		account, err := svc.DisableModelPoolAccount(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("account_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolAccountEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Account:   account,
		})
	}))

	mux.Handle("PATCH /api/v1/admin/model-pool/{account_id}", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.UpdateModelPoolAccountInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		account, err := svc.UpdateModelPoolAccount(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("account_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolAccountEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Account:   account,
		})
	}))

	mux.Handle("POST /api/v1/admin/model-pool/{account_id}/test", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.TestModelPoolAccountInput
		if err := decodeOptionalJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		result, err := svc.TestModelPoolAccount(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("account_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolConnectivityTestResponse{
			RequestID:                       RequestIDFromContext(r.Context()),
			ModelPoolConnectivityTestResult: result,
		})
	}))

	mux.Handle("POST /api/v1/client/activate", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.ActivateDeviceInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		device, err := svc.ActivateDevice(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		auth.bindDevice(r, device.ID)
		writeJSON(w, http.StatusOK, struct {
			RequestID string                     `json:"request_id"`
			Device    controlplane.DeviceSummary `json:"device"`
		}{
			RequestID: RequestIDFromContext(r.Context()),
			Device:    device,
		})
	}))

	mux.Handle("GET /api/v1/client/profile", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		profile, err := svc.GetClientProfile(r.Context(), actor.UserID, auth.deviceID(r))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, clientProfileResponse{
			RequestID:   RequestIDFromContext(r.Context()),
			User:        profile.User,
			Device:      profile.Device,
			Permissions: profile.Permissions,
		})
	}))

	mux.Handle("POST /api/v1/client/heartbeat", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.HeartbeatInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		result, err := svc.RecordHeartbeat(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		auth.bindDevice(r, input.DeviceID)
		writeJSON(w, http.StatusOK, heartbeatResponse{
			RequestID:    RequestIDFromContext(r.Context()),
			AcceptedAt:   result.AcceptedAt,
			DeviceStatus: result.DeviceStatus,
		})
	}))

	mux.Handle("POST /api/v1/client/model-leases", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := auth.deviceID(r)
		if deviceID == "" {
			writeAppError(w, r, controlplane.ErrDeviceBindingRequired)
			return
		}
		var input controlplane.CreateModelLeaseInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		lease, err := svc.CreateModelLease(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelLeaseResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Lease:     lease,
		})
	}))

	mux.Handle("POST /api/v1/client/model-leases/{lease_id}/renew", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := auth.deviceID(r)
		if deviceID == "" {
			writeAppError(w, r, controlplane.ErrDeviceBindingRequired)
			return
		}
		var input controlplane.RenewModelLeaseInput
		if err := decodeOptionalJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		lease, err := svc.RenewModelLease(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, r.PathValue("lease_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelLeaseResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Lease:     lease,
		})
	}))

	mux.Handle("POST /api/v1/client/model-leases/{lease_id}/release", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := auth.deviceID(r)
		if deviceID == "" {
			writeAppError(w, r, controlplane.ErrDeviceBindingRequired)
			return
		}
		var input controlplane.ReleaseModelLeaseInput
		if err := decodeOptionalJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		result, err := svc.ReleaseModelLease(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, r.PathValue("lease_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, releaseModelLeaseResponse{
			RequestID: RequestIDFromContext(r.Context()),
			LeaseID:   result.LeaseID,
			Released:  result.Released,
		})
	}))

	mux.Handle("POST /api/v1/client/llm/call-records", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := auth.deviceID(r)
		if deviceID == "" {
			writeAppError(w, r, controlplane.ErrDeviceBindingRequired)
			return
		}
		var input controlplane.CreateDirectLLMCallRecordInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		_, err := svc.RecordDirectLLMCall(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, RequestIDFromContext(r.Context()), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, directLLMCallRecordResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Recorded:  true,
		})
	}))

	mux.Handle("GET /api/v1/admin/model-usage", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListModelUsage(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		page, start, end, err := pageRange(r, len(items))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelUsageListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items[start:end],
			Pagination: page,
		})
	}))

	mux.Handle("GET /api/v1/admin/audit-logs", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		items, err := svc.ListAuditLogs(r.Context())
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		page, start, end, err := pageRange(r, len(items))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, auditLogListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items[start:end],
			Pagination: page,
		})
	}))

}

func decodeJSONBody(r *http.Request, target any) error {
	if r.Body == nil {
		return controlplane.ErrInvalidRequest
	}
	const maxJSONBodyBytes = 1 << 20
	raw, err := io.ReadAll(io.LimitReader(r.Body, maxJSONBodyBytes+1))
	if err != nil || len(raw) > maxJSONBodyBytes {
		return controlplane.ErrInvalidRequest
	}
	return decodeJSONBytes(raw, target)
}

func decodeOptionalJSONBody(r *http.Request, target any) error {
	if r.Body == nil {
		return nil
	}
	const maxJSONBodyBytes = 1 << 20
	raw, err := io.ReadAll(io.LimitReader(r.Body, maxJSONBodyBytes+1))
	if err != nil || len(raw) > maxJSONBodyBytes {
		return controlplane.ErrInvalidRequest
	}
	if len(bytes.TrimSpace(raw)) == 0 {
		return nil
	}
	return decodeJSONBytes(raw, target)
}

func decodeJSONBytes(raw []byte, target any) error {
	if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return controlplane.ErrInvalidRequest
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func pageRange(r *http.Request, total int) (pagination, int, int, error) {
	page := 1
	pageSize := 20
	query := r.URL.Query()
	if value := query.Get("page"); value != "" {
		parsed, err := strconv.Atoi(value)
		if err != nil || parsed < 1 {
			return pagination{}, 0, 0, controlplane.ErrInvalidRequest
		}
		page = parsed
	}
	if value := query.Get("page_size"); value != "" {
		parsed, err := strconv.Atoi(value)
		if err != nil || parsed < 1 || parsed > 200 {
			return pagination{}, 0, 0, controlplane.ErrInvalidRequest
		}
		pageSize = parsed
	}
	start := total
	if page <= total/pageSize+1 {
		start = (page - 1) * pageSize
	}
	end := start + pageSize
	if end > total {
		end = total
	}
	return pagination{Page: page, PageSize: pageSize, Total: total}, start, end, nil
}
