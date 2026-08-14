package httpapi

import (
	"context"
	"log/slog"
	"net/http"
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
)

type auditPrincipalContextKey struct{}

type auditPrincipal struct {
	actor    controlplane.Actor
	deviceID string
	set      bool
}

func auditMiddleware(controlPlane *service.ControlPlane, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		principal := &auditPrincipal{}
		requestContext := context.WithValue(r.Context(), auditPrincipalContextKey{}, principal)
		recorder := &statusRecorder{ResponseWriter: w, statusCode: http.StatusOK}
		next.ServeHTTP(recorder, r.WithContext(requestContext))

		if r.Method == http.MethodGet || !strings.HasPrefix(r.URL.Path, "/api/v1/") {
			return
		}
		if !principal.set || principal.actor.UserID == "" {
			return
		}
		// 审计只记录方法、路由和目标上下文，不读取请求体，避免把密码/Token/API Key写入日志。
		auditCtx := context.WithoutCancel(r.Context())
		if err := controlPlane.RecordAudit(auditCtx, controlplane.AuditLogInput{
			ActorUserID: principal.actor.UserID,
			DeviceID:    principal.deviceID,
			Action:      r.Method + " " + r.URL.Path,
			TargetType:  "http_endpoint",
			RequestID:   RequestIDFromContext(r.Context()),
		}); err != nil {
			slog.Default().Error("write audit log failed", "error", err, "path", r.URL.Path, "request_id", RequestIDFromContext(r.Context()))
		}
	})
}
