package httpapi

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"net/http"
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
)

type auditPrincipalContextKey struct{}

type auditPrincipal struct {
	actor     controlplane.Actor
	deviceID  string
	errorCode string
	set       bool
}

func auditMiddlewareWithOptions(controlPlane *service.ControlPlane, logger *slog.Logger, metrics *httpMetrics, next http.Handler) http.Handler {
	if logger == nil {
		logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		principal := &auditPrincipal{}
		requestContext := context.WithValue(r.Context(), auditPrincipalContextKey{}, principal)
		buffer := newAuditResponseBuffer()
		next.ServeHTTP(buffer, r.WithContext(requestContext))

		if r.Method == http.MethodGet || !strings.HasPrefix(r.URL.Path, "/api/v1/") {
			buffer.flush(w)
			return
		}
		if !principal.set && !shouldAuditUnauthenticatedRequest(r) {
			buffer.flush(w)
			return
		}
		outcome := "success"
		if buffer.statusCode < http.StatusOK || buffer.statusCode >= http.StatusMultipleChoices {
			outcome = "failure"
		}
		targetType, targetID := auditTarget(r)
		if targetID == "" && targetType == "device" {
			// Client activation/heartbeat carry the device ID in the validated
			// request body; the auth binder has already copied that ID into the
			// audit principal without retaining or parsing the body here.
			targetID = principal.deviceID
		}
		// 审计只记录方法、路由和目标上下文，不读取请求体，避免把密码/Token/API Key 写入日志。
		auditCtx := context.WithoutCancel(r.Context())
		auditProduct := principal.actor.Product
		if !auditProduct.Valid() {
			// Login failures have no authenticated actor yet; legacy login
			// requests are bound to the default product until authentication
			// succeeds and a session product is available.
			auditProduct = controlplane.ProductAutoLive
		}
		if err := controlPlane.RecordAuditForProduct(auditCtx, auditProduct, controlplane.AuditLogInput{
			ActorUserID: principal.actor.UserID,
			Product:     auditProduct,
			DeviceID:    principal.deviceID,
			Action:      r.Method + " " + r.URL.Path,
			TargetType:  targetType,
			TargetID:    targetID,
			Outcome:     outcome,
			StatusCode:  buffer.statusCode,
			ErrorCode:   principal.errorCode,
			RequestID:   RequestIDFromContext(r.Context()),
		}); err != nil {
			if metrics != nil {
				metrics.recordAuditWriteFailure()
			}
			logger.Error("write audit log failed", "error", err, "path", r.URL.Path, "request_id", RequestIDFromContext(r.Context()))
			writeError(w, r.WithContext(requestContext), http.StatusServiceUnavailable, "AUDIT_UNAVAILABLE", "审计服务暂时不可用，请稍后重试")
			return
		}
		buffer.flush(w)
	})
}

// auditResponseBuffer delays committing an API write response until its audit
// record is durable. Current control-plane routes are bounded JSON responses;
// buffering avoids exposing a success result that cannot be audited.
type auditResponseBuffer struct {
	header      http.Header
	body        bytes.Buffer
	statusCode  int
	wroteHeader bool
}

func newAuditResponseBuffer() *auditResponseBuffer {
	return &auditResponseBuffer{header: make(http.Header), statusCode: http.StatusOK}
}

func (b *auditResponseBuffer) Header() http.Header { return b.header }

func (b *auditResponseBuffer) WriteHeader(statusCode int) {
	if b.wroteHeader {
		return
	}
	b.statusCode = statusCode
	b.wroteHeader = true
}

func (b *auditResponseBuffer) Write(payload []byte) (int, error) {
	if !b.wroteHeader {
		b.WriteHeader(http.StatusOK)
	}
	return b.body.Write(payload)
}

func (b *auditResponseBuffer) flush(destination http.ResponseWriter) {
	for key, values := range b.header {
		destination.Header()[key] = append([]string(nil), values...)
	}
	destination.WriteHeader(b.statusCode)
	if b.body.Len() > 0 {
		_, _ = destination.Write(b.body.Bytes())
	}
}

func shouldAuditUnauthenticatedRequest(r *http.Request) bool {
	return r.Method == http.MethodPost && r.URL.Path == "/api/v1/auth/login"
}

func successAuditForDevice(r *http.Request, actor controlplane.Actor, deviceID string) controlplane.AuditLogInput {
	return successAuditForTarget(r, actor, "device", deviceID, deviceID)
}

func successAuditForTarget(r *http.Request, actor controlplane.Actor, targetType, targetID, deviceID string) controlplane.AuditLogInput {
	return controlplane.AuditLogInput{
		ActorUserID: actor.UserID,
		Product:     actor.Product,
		DeviceID:    deviceID,
		Action:      r.Method + " " + r.URL.Path,
		TargetType:  targetType,
		TargetID:    targetID,
		Outcome:     "success",
		StatusCode:  http.StatusOK,
		RequestID:   RequestIDFromContext(r.Context()),
	}
}

func auditTarget(r *http.Request) (string, string) {
	path := r.URL.Path
	for _, target := range []struct {
		prefix string
		name   string
	}{
		{prefix: "/api/v1/admin/users/", name: "user"},
		{prefix: "/api/v1/admin/devices/", name: "device"},
		{prefix: "/api/v1/admin/activation-codes/", name: "activation_code"},
		{prefix: "/api/v1/admin/model-pool/", name: "model_account"},
		{prefix: "/api/v1/admin/model-leases/", name: "model_lease"},
		{prefix: "/api/v1/client/model-leases/", name: "model_lease"},
	} {
		if !strings.HasPrefix(path, target.prefix) {
			continue
		}
		for _, parameter := range []string{"user_id", "device_id", "code_id", "account_id", "lease_id"} {
			if value := r.PathValue(parameter); value != "" {
				return target.name, value
			}
		}
		return target.name, firstPathSegment(path[len(target.prefix):])
	}
	for _, target := range []struct {
		fragment string
		name     string
	}{
		{fragment: "/users", name: "user"},
		{fragment: "/devices", name: "device"},
		{fragment: "/activation-codes", name: "activation_code"},
		{fragment: "/model-pool", name: "model_account"},
		{fragment: "/model-leases", name: "model_lease"},
		{fragment: "/model-usage", name: "model_usage"},
		{fragment: "/audit-logs", name: "audit_log"},
		{fragment: "/auth/", name: "session"},
	} {
		if strings.Contains(path, target.fragment) {
			return target.name, ""
		}
	}
	if strings.HasPrefix(path, "/api/v1/client/") {
		return "device", ""
	}
	return "http_endpoint", ""
}

func firstPathSegment(path string) string {
	if separator := strings.IndexByte(path, '/'); separator >= 0 {
		return path[:separator]
	}
	return path
}
