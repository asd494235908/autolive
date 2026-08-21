package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"autoLive/backend/internal/controlplane"
)

type ErrorResponse struct {
	Code      string        `json:"code"`
	Message   string        `json:"message"`
	RequestID string        `json:"request_id"`
	Details   []ErrorDetail `json:"details,omitempty"`
}

type ErrorDetail struct {
	Field  string `json:"field"`
	Reason string `json:"reason"`
}

func writeJSON(w http.ResponseWriter, status int, payload any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(payload)
}

func writeError(w http.ResponseWriter, r *http.Request, status int, code, message string) {
	if principal, ok := r.Context().Value(auditPrincipalContextKey{}).(*auditPrincipal); ok {
		principal.errorCode = code
	}
	writeJSON(w, status, ErrorResponse{
		Code:      code,
		Message:   message,
		RequestID: RequestIDFromContext(r.Context()),
	})
}

func writeAppError(w http.ResponseWriter, r *http.Request, err error) {
	var appErr *controlplane.Error
	if errors.As(err, &appErr) {
		writeError(w, r, appErr.Status, appErr.Code, appErr.Message)
		return
	}
	if errors.Is(err, context.DeadlineExceeded) {
		writeError(w, r, http.StatusGatewayTimeout, "REQUEST_TIMEOUT", "请求处理超时")
		return
	}
	if errors.Is(err, context.Canceled) {
		writeError(w, r, http.StatusRequestTimeout, "REQUEST_CANCELED", "请求已取消")
		return
	}
	writeError(w, r, http.StatusInternalServerError, "INTERNAL_ERROR", "服务内部错误")
}
