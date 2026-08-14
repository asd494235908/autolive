package httpapi

import (
	"encoding/json"
	"net/http"

	"autoLive/backend/internal/controlplane"
)

type ErrorResponse struct {
	Code      string `json:"code"`
	Message   string `json:"message"`
	RequestID string `json:"request_id"`
}

func writeJSON(w http.ResponseWriter, status int, payload any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(payload)
}

func writeError(w http.ResponseWriter, r *http.Request, status int, code, message string) {
	writeJSON(w, status, ErrorResponse{
		Code:      code,
		Message:   message,
		RequestID: RequestIDFromContext(r.Context()),
	})
}

func writeAppError(w http.ResponseWriter, r *http.Request, err error) {
	if appErr, ok := err.(*controlplane.Error); ok {
		writeError(w, r, appErr.Status, appErr.Code, appErr.Message)
		return
	}
	writeError(w, r, http.StatusInternalServerError, "INTERNAL_ERROR", "服务内部错误")
}
