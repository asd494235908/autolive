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

type userAuthorizationSummaryResponse struct {
	RequestID string                                `json:"request_id"`
	Summary   controlplane.UserAuthorizationSummary `json:"summary"`
}

type userAuthorizationPolicyResponse struct {
	RequestID string                               `json:"request_id"`
	Policy    controlplane.UserAuthorizationPolicy `json:"policy"`
}

type deviceEnvelope struct {
	RequestID string                     `json:"request_id"`
	Device    controlplane.DeviceSummary `json:"device"`
}

type unbindDeviceResponse struct {
	RequestID  string `json:"request_id"`
	DeviceID   string `json:"device_id"`
	DeviceName string `json:"device_name"`
	Platform   string `json:"platform"`
	Status     string `json:"status"`
	Online     bool   `json:"online"`
	LastSeenAt string `json:"last_seen_at"`
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
	RequestID  string                                 `json:"request_id"`
	Accounts   []controlplane.ModelPoolAccountSummary `json:"accounts"`
	Pagination pagination                             `json:"pagination"`
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

type modelLeaseListResponse struct {
	RequestID  string                                `json:"request_id"`
	Items      []controlplane.ModelLeaseAdminSummary `json:"items"`
	Pagination pagination                            `json:"pagination"`
}

type modelLeaseAdminDetailResponse struct {
	RequestID string                             `json:"request_id"`
	Lease     controlplane.ModelLeaseAdminDetail `json:"lease"`
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
	Product     controlplane.ProductCode   `json:"product"`
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
	mux.Handle("POST /api/v1/admin/auth/change-password", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		if actor.UserID != "usr_local_admin" {
			writeAppError(w, r, controlplane.ErrLocalAdminRequired)
			return
		}
		var input controlplane.ChangeLocalAdminPasswordInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		user, err := svc.ChangeLocalAdminPassword(r.Context(), r.Header.Get("Idempotency-Key"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if err := auth.revokeUserSessions(r.Context(), user.ID); err != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "管理员会话暂时无法撤销，请使用相同幂等键重试")
			return
		}
		writeJSON(w, http.StatusOK, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("GET /api/v1/admin/users", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListUsersPage(r.Context(), pageNumber, pageSize)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("GET /api/v1/admin/users/{user_id}/devices", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListDevicesForUserPage(r.Context(), r.PathValue("user_id"), pageNumber, pageSize)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("GET /api/v1/admin/users/{user_id}/authorization-summary", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		summary, err := svc.GetUserAuthorizationSummary(r.Context(), r.PathValue("user_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userAuthorizationSummaryResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Summary:   summary,
		})
	}))

	mux.Handle("PATCH /api/v1/admin/users/{user_id}/authorization", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.UpdateUserAuthorizationInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		policy, err := svc.UpdateUserAuthorization(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("user_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userAuthorizationPolicyResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Policy:    policy,
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
		if err := auth.revokeUserSessions(r.Context(), user.ID); err != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "用户会话暂时无法撤销")
			return
		}
		writeJSON(w, http.StatusOK, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("PATCH /api/v1/admin/users/{user_id}", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.UpdateUserInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		user, err := svc.UpdateUser(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("user_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if err := auth.revokeUserSessions(r.Context(), user.ID); err != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "用户会话暂时无法撤销")
			return
		}
		writeJSON(w, http.StatusOK, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("POST /api/v1/admin/users/{user_id}/reset-password", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.ResetUserPasswordInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		user, err := svc.ResetUserPassword(r.Context(), r.Header.Get("Idempotency-Key"), r.PathValue("user_id"), input)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if err := auth.revokeUserSessions(r.Context(), user.ID); err != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "用户会话暂时无法撤销")
			return
		}
		writeJSON(w, http.StatusOK, userEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			User:      user,
		})
	}))

	mux.Handle("GET /api/v1/admin/devices", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListDevicesPage(r.Context(), pageNumber, pageSize)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("GET /api/v1/admin/devices/{device_id}", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		device, err := svc.GetDevice(r.Context(), r.PathValue("device_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, deviceEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Device:    device,
		})
	}))

	mux.Handle("POST /api/v1/admin/devices/{device_id}/disable", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := r.PathValue("device_id")
		device, err := svc.DisableDeviceWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), deviceID, successAuditForDevice(r, actor, deviceID))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if !svc.SupportsTransactionalDeviceLifecycle() {
			if err := auth.revokeDeviceSessions(r.Context(), device.ID); err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "设备会话暂时无法撤销")
				return
			}
		}
		writeJSON(w, http.StatusOK, deviceEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Device:    device,
		})
	}))

	mux.Handle("POST /api/v1/admin/devices/{device_id}/unbind", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		deviceID := r.PathValue("device_id")
		device, err := svc.UnbindDeviceWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), deviceID, successAuditForDevice(r, actor, deviceID))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if !svc.SupportsTransactionalDeviceLifecycle() {
			if err := auth.revokeDeviceSessions(r.Context(), device.ID); err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "设备会话暂时无法撤销")
				return
			}
		}
		writeJSON(w, http.StatusOK, unbindDeviceResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			DeviceID:   device.ID,
			DeviceName: device.DeviceName,
			Platform:   device.Platform,
			Status:     device.Status,
			Online:     device.Online,
			LastSeenAt: device.LastSeenAt,
		})
	}))

	mux.Handle("GET /api/v1/admin/activation-codes", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		page, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListActivationCodesPage(r.Context(), page, pageSize)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, activationCodeListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: page, PageSize: pageSize, Total: total},
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
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListModelPoolAccountsPage(r.Context(), pageNumber, pageSize)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Accounts:   items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("POST /api/v1/admin/model-pool", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.CreateModelPoolAccountInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		account, err := svc.CreateModelPoolAccountWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), input, successAuditForTarget(r, actor, "model_account", "", ""))
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
		accountID := r.PathValue("account_id")
		account, err := svc.DisableModelPoolAccountWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), accountID, successAuditForTarget(r, actor, "model_account", accountID, ""))
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
		accountID := r.PathValue("account_id")
		account, err := svc.UpdateModelPoolAccountWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), accountID, input, successAuditForTarget(r, actor, "model_account", accountID, ""))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelPoolAccountEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Account:   account,
		})
	}))

	mux.Handle("POST /api/v1/admin/model-pool/{account_id}/rotate-secret", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.RotateModelPoolAccountSecretInput
		if err := decodeJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		accountID := r.PathValue("account_id")
		account, err := svc.RotateModelPoolAccountSecretWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), accountID, input, successAuditForTarget(r, actor, "model_account", accountID, ""))
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
		accountID := r.PathValue("account_id")
		result, err := svc.TestModelPoolAccountWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), accountID, input, successAuditForTarget(r, actor, "model_account", accountID, ""))
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
		if err := requireActorProduct(actor, input.Device.Product); err != nil {
			writeAppError(w, r, err)
			return
		}
		if svc.SupportsTransactionalSessionBinding() {
			setAuditDeviceID(r, input.Device.DeviceID)
			audit := successAuditForDevice(r, actor, input.Device.DeviceID)
			device, err := svc.ActivateDeviceWithSessionBindingAndAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, auth.accessTokenHash(r), input, audit)
			if err != nil {
				writeAppError(w, r, err)
				return
			}
			writeJSON(w, http.StatusOK, struct {
				RequestID string                     `json:"request_id"`
				Device    controlplane.DeviceSummary `json:"device"`
			}{
				RequestID: RequestIDFromContext(r.Context()),
				Device:    device,
			})
			return
		}
		boundByRequest, err := auth.bindDeviceTracked(r, input.Device.DeviceID)
		if err != nil {
			if _, ok := err.(*controlplane.Error); ok {
				writeAppError(w, r, err)
				return
			}
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "登录会话暂时无法绑定设备")
			return
		}
		device, err := svc.ActivateDevice(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, input)
		if err != nil {
			if boundByRequest {
				if compensationErr := auth.clearDeviceBinding(r, input.Device.DeviceID); compensationErr != nil {
					writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "设备激活未提交且会话补偿失败，请保持相同幂等键重试")
					return
				}
			}
			writeAppError(w, r, err)
			return
		}
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
			Product:     actor.Product,
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
		if err := requireActorProduct(actor, input.Product); err != nil {
			writeAppError(w, r, err)
			return
		}
		if svc.SupportsTransactionalSessionBinding() {
			setAuditDeviceID(r, input.DeviceID)
			audit := successAuditForDevice(r, actor, input.DeviceID)
			result, err := svc.RecordHeartbeatWithSessionBindingAndAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, auth.accessTokenHash(r), input, audit)
			if err != nil {
				writeAppError(w, r, err)
				return
			}
			writeJSON(w, http.StatusOK, heartbeatResponse{
				RequestID:    RequestIDFromContext(r.Context()),
				AcceptedAt:   result.AcceptedAt,
				DeviceStatus: result.DeviceStatus,
			})
			return
		}
		boundByRequest, err := auth.bindDeviceTracked(r, input.DeviceID)
		if err != nil {
			if _, ok := err.(*controlplane.Error); ok {
				writeAppError(w, r, err)
				return
			}
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "登录会话暂时无法绑定设备")
			return
		}
		result, err := svc.RecordHeartbeat(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, input)
		if err != nil {
			if boundByRequest {
				if compensationErr := auth.clearDeviceBinding(r, input.DeviceID); compensationErr != nil {
					writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "心跳未提交且会话补偿失败，请保持相同幂等键重试")
					return
				}
			}
			writeAppError(w, r, err)
			return
		}
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
		lease, err := svc.CreateModelLeaseWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, input, successAuditForTarget(r, actor, "model_lease", "", deviceID))
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
		leaseID := r.PathValue("lease_id")
		lease, err := svc.RenewModelLeaseWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, leaseID, input, successAuditForTarget(r, actor, "model_lease", leaseID, deviceID))
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
		leaseID := r.PathValue("lease_id")
		result, err := svc.ReleaseModelLeaseWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, leaseID, input, successAuditForTarget(r, actor, "model_lease", leaseID, deviceID))
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
		_, err := svc.RecordDirectLLMCallWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), actor.UserID, deviceID, RequestIDFromContext(r.Context()), input, successAuditForTarget(r, actor, "model_usage", "", deviceID))
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
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListModelUsagePageWithOptions(r.Context(), pageNumber, pageSize, modelUsageListOptions(r))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelUsageListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("GET /api/v1/admin/model-leases", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListModelLeasesPageWithOptions(r.Context(), pageNumber, pageSize, modelLeaseListOptions(r))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelLeaseListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

	mux.Handle("GET /api/v1/admin/model-leases/{lease_id}", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		lease, err := svc.GetModelLeaseAdminDetail(r.Context(), r.PathValue("lease_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, modelLeaseAdminDetailResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Lease:     lease,
		})
	}))

	mux.Handle("POST /api/v1/admin/model-leases/{lease_id}/reclaim", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		var input controlplane.ReleaseModelLeaseInput
		if err := decodeOptionalJSONBody(r, &input); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		leaseID := r.PathValue("lease_id")
		result, err := svc.ReclaimModelLeaseWithAudit(r.Context(), r.Header.Get("Idempotency-Key"), leaseID, input, successAuditForTarget(r, actor, "model_lease", leaseID, ""))
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

	mux.Handle("GET /api/v1/admin/audit-logs", auth.requireAdmin(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		pageNumber, pageSize, err := pageParams(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		items, total, err := svc.ListAuditLogsPageWithOptions(r.Context(), pageNumber, pageSize, auditLogListOptions(r))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, auditLogListResponse{
			RequestID:  RequestIDFromContext(r.Context()),
			Items:      items,
			Pagination: pagination{Page: pageNumber, PageSize: pageSize, Total: total},
		})
	}))

}

func requireActorProduct(actor controlplane.Actor, requested controlplane.ProductCode) error {
	product, err := controlplane.ParseProductCode(string(requested))
	if err != nil || !actor.Product.Valid() {
		return controlplane.ErrInvalidRequest
	}
	if product != actor.Product {
		return controlplane.ErrForbidden
	}
	return nil
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

func pageParams(r *http.Request) (int, int, error) {
	page := 1
	pageSize := 20
	query := r.URL.Query()
	if value := query.Get("page"); value != "" {
		parsed, err := strconv.Atoi(value)
		if err != nil || parsed < 1 {
			return 0, 0, controlplane.ErrInvalidRequest
		}
		page = parsed
	}
	if value := query.Get("page_size"); value != "" {
		parsed, err := strconv.Atoi(value)
		if err != nil || parsed < 1 || parsed > 200 {
			return 0, 0, controlplane.ErrInvalidRequest
		}
		pageSize = parsed
	}
	return page, pageSize, nil
}

func modelLeaseListOptions(r *http.Request) service.ModelLeaseListOptions {
	query := r.URL.Query()
	return service.ModelLeaseListOptions{
		Status:    query.Get("status"),
		Provider:  query.Get("provider"),
		Model:     query.Get("model"),
		UserID:    query.Get("user_id"),
		DeviceID:  query.Get("device_id"),
		AccountID: query.Get("account_id"),
		Sort:      query.Get("sort"),
	}
}

func modelUsageListOptions(r *http.Request) service.ModelUsageListOptions {
	query := r.URL.Query()
	return service.ModelUsageListOptions{
		Provider:      query.Get("provider"),
		Model:         query.Get("model"),
		UserID:        query.Get("user_id"),
		DeviceID:      query.Get("device_id"),
		RequestID:     query.Get("request_id"),
		CreatedAfter:  query.Get("created_after"),
		CreatedBefore: query.Get("created_before"),
		Sort:          query.Get("sort"),
	}
}

func auditLogListOptions(r *http.Request) service.AuditLogListOptions {
	query := r.URL.Query()
	return service.AuditLogListOptions{
		ActorUserID:   query.Get("actor_user_id"),
		DeviceID:      query.Get("device_id"),
		Action:        query.Get("action"),
		TargetType:    query.Get("target_type"),
		Outcome:       query.Get("outcome"),
		ErrorCode:     query.Get("error_code"),
		RequestID:     query.Get("request_id"),
		CreatedAfter:  query.Get("created_after"),
		CreatedBefore: query.Get("created_before"),
		Sort:          query.Get("sort"),
	}
}

func pageRange(r *http.Request, total int) (pagination, int, int, error) {
	page, pageSize, err := pageParams(r)
	if err != nil {
		return pagination{}, 0, 0, err
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
