package httpapi

import (
	"net/http"
	"regexp"
	"slices"
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
)

const (
	httpAdminRoleCodeMaxLen    = 128
	httpAdminRoleNameMaxLen    = 128
	httpAdminAssignmentsMaxLen = 200
)

var httpIDPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{7,63}$`)

type adminMeResponse struct {
	RequestID        string                        `json:"request_id"`
	User             controlplane.UserSummary      `json:"user"`
	Product          controlplane.ProductCode      `json:"product"`
	GlobalSuperAdmin bool                          `json:"global_super_admin"`
	RoleCodes        []string                      `json:"role_codes"`
	Permissions      []controlplane.PermissionCode `json:"permissions"`
}

type adminPermissionsResponse struct {
	RequestID   string                        `json:"request_id"`
	Permissions []controlplane.PermissionCode `json:"permissions"`
}

type adminRoleRequest struct {
	Code        string   `json:"code,omitempty"`
	Product     string   `json:"product,omitempty"`
	Name        string   `json:"name"`
	Permissions []string `json:"permissions"`
}

type adminRoleEnvelope struct {
	RequestID string                `json:"request_id"`
	Role      service.AdminRoleView `json:"role"`
}

type adminRoleListResponse struct {
	RequestID string                  `json:"request_id"`
	Roles     []service.AdminRoleView `json:"roles"`
}

type replaceUserAdminRolesRequest struct {
	Product   string   `json:"product"`
	RoleCodes []string `json:"role_codes"`
}

type userAdminRolesResponse struct {
	RequestID   string                             `json:"request_id"`
	Assignments []controlplane.AdminRoleAssignment `json:"assignments"`
}

func registerAdminRBACRoutes(mux *http.ServeMux, svc *service.ControlPlane, auth *authenticator) {
	mux.Handle("GET /api/v1/admin/me", auth.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		user, err := svc.GetUser(r.Context(), actor.UserID)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		authorization, err := svc.GetAdminAuthorization(r.Context(), actor)
		if err != nil {
			writeAdminAuthorizationError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, adminMeResponse{
			RequestID:        RequestIDFromContext(r.Context()),
			User:             user,
			Product:          authorization.Product,
			GlobalSuperAdmin: authorization.GlobalSuperAdmin,
			RoleCodes:        authorization.RoleCodes,
			Permissions:      authorization.Permissions,
		})
	}))

	mux.Handle("GET /api/v1/admin/permissions", auth.requirePermission("roles.read", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		permissions, err := svc.ListAdminPermissions(r.Context(), actor)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, adminPermissionsResponse{
			RequestID:   RequestIDFromContext(r.Context()),
			Permissions: permissions,
		})
	}))

	mux.Handle("GET /api/v1/admin/roles", auth.requirePermission("roles.read", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		product, err := parseOptionalAdminProductQuery(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		roles, err := svc.ListAdminRoles(r.Context(), actor, product)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, adminRoleListResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Roles:     roles,
		})
	}))

	mux.Handle("GET /api/v1/admin/roles/{role_id}", auth.requirePermission("roles.read", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		roleCode, err := boundedPathValue(r.PathValue("role_id"), httpAdminRoleCodeMaxLen)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		role, err := svc.GetAdminRole(r.Context(), actor, roleCode)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, adminRoleEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Role:      role,
		})
	}))

	mux.Handle("POST /api/v1/admin/roles", auth.requirePermission("roles.manage", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		input, err := decodeAdminRoleRequest(r, true)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		role, err := svc.CreateAdminRoleWithAudit(r.Context(), actor, r.Header.Get("Idempotency-Key"), input, successAdminRBACAudit(r, actor, http.StatusCreated, "admin_role", input.Code))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusCreated, adminRoleEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Role:      role,
		})
	}))

	mux.Handle("PATCH /api/v1/admin/roles/{role_id}", auth.requirePermission("roles.manage", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		roleCode, err := boundedPathValue(r.PathValue("role_id"), httpAdminRoleCodeMaxLen)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		input, err := decodeAdminRoleRequest(r, false)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		role, err := svc.UpdateAdminRoleWithAudit(r.Context(), actor, r.Header.Get("Idempotency-Key"), roleCode, input, successAdminRBACAudit(r, actor, http.StatusOK, "admin_role", roleCode))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, adminRoleEnvelope{
			RequestID: RequestIDFromContext(r.Context()),
			Role:      role,
		})
	}))

	mux.Handle("DELETE /api/v1/admin/roles/{role_id}", auth.requirePermission("roles.manage", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		roleCode, err := boundedPathValue(r.PathValue("role_id"), httpAdminRoleCodeMaxLen)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if err := svc.DeleteAdminRoleWithAudit(r.Context(), actor, r.Header.Get("Idempotency-Key"), roleCode, successAdminRBACAudit(r, actor, http.StatusNoContent, "admin_role", roleCode)); err != nil {
			writeAppError(w, r, err)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}))

	mux.Handle("GET /api/v1/admin/users/{user_id}/roles", auth.requirePermission("roles.assign", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		userID, err := boundedIDPathValue(r.PathValue("user_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		product, err := parseRequiredAdminProductQuery(r)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		assignments, err := svc.ListUserAdminRoles(r.Context(), actor, userID, product)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userAdminRolesResponse{
			RequestID:   RequestIDFromContext(r.Context()),
			Assignments: assignments,
		})
	}))

	mux.Handle("PUT /api/v1/admin/users/{user_id}/roles", auth.requirePermission("roles.assign", func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		userID, err := boundedIDPathValue(r.PathValue("user_id"))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		var request replaceUserAdminRolesRequest
		if err := decodeJSONBody(r, &request); err != nil {
			writeAppError(w, r, controlplane.ErrInvalidRequest)
			return
		}
		product, err := parseRequiredAdminProduct(request.Product)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		assignments, err := buildAssignments(userID, product, request.RoleCodes)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		result, err := svc.ReplaceUserAdminRolesWithAudit(r.Context(), actor, r.Header.Get("Idempotency-Key"), userID, product, assignments, successAdminRBACAudit(r, actor, http.StatusOK, "admin_role_assignment", userID))
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		writeJSON(w, http.StatusOK, userAdminRolesResponse{
			RequestID:   RequestIDFromContext(r.Context()),
			Assignments: result,
		})
	}))
}

func decodeAdminRoleRequest(r *http.Request, requireCodeAndProduct bool) (service.AdminRoleSpec, error) {
	var request adminRoleRequest
	if err := decodeJSONBody(r, &request); err != nil {
		return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	code := strings.TrimSpace(request.Code)
	if code != "" && len(code) > httpAdminRoleCodeMaxLen {
		return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	name := strings.TrimSpace(request.Name)
	if name == "" || len(name) > httpAdminRoleNameMaxLen {
		return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	product := strings.TrimSpace(request.Product)
	if requireCodeAndProduct && (code == "" || product == "") {
		return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	if product != "" {
		if _, err := controlplane.ParseProductCode(product); err != nil {
			return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
		}
	}
	if len(request.Permissions) > len(controlplane.PermissionCatalog()) {
		return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	permissions := make([]controlplane.PermissionCode, 0, len(request.Permissions))
	for _, permission := range request.Permissions {
		current := strings.TrimSpace(permission)
		if current == "" || len(current) > httpAdminRoleCodeMaxLen {
			return service.AdminRoleSpec{}, controlplane.ErrInvalidRequest
		}
		permissions = append(permissions, controlplane.PermissionCode(current))
	}
	return service.AdminRoleSpec{
		Code:        code,
		Product:     controlplane.ProductCode(product),
		Name:        name,
		Permissions: permissions,
	}, nil
}

func parseOptionalAdminProductQuery(r *http.Request) (controlplane.ProductCode, error) {
	values, ok := r.URL.Query()["product"]
	if !ok {
		return "", nil
	}
	if len(values) != 1 {
		return "", controlplane.ErrInvalidRequest
	}
	if strings.TrimSpace(values[0]) == "" {
		return "", nil
	}
	return controlplane.ParseProductCode(values[0])
}

func parseRequiredAdminProductQuery(r *http.Request) (controlplane.ProductCode, error) {
	values := r.URL.Query()["product"]
	if len(values) != 1 {
		return "", controlplane.ErrInvalidRequest
	}
	return parseRequiredAdminProduct(values[0])
}

func parseRequiredAdminProduct(raw string) (controlplane.ProductCode, error) {
	if strings.TrimSpace(raw) == "" {
		return "", controlplane.ErrInvalidRequest
	}
	return controlplane.ParseProductCode(raw)
}

func buildAssignments(userID string, product controlplane.ProductCode, roleCodes []string) ([]controlplane.AdminRoleAssignment, error) {
	if len(roleCodes) > httpAdminAssignmentsMaxLen {
		return nil, controlplane.ErrInvalidRequest
	}
	assignments := make([]controlplane.AdminRoleAssignment, 0, len(roleCodes))
	seen := make(map[string]struct{}, len(roleCodes))
	for _, roleCode := range roleCodes {
		current := strings.TrimSpace(roleCode)
		if current == "" || len(current) > httpAdminRoleCodeMaxLen {
			return nil, controlplane.ErrInvalidRequest
		}
		if _, exists := seen[current]; exists {
			continue
		}
		seen[current] = struct{}{}
		assignments = append(assignments, controlplane.AdminRoleAssignment{
			UserID:   userID,
			RoleCode: current,
			Product:  product,
		})
	}
	slices.SortFunc(assignments, func(left, right controlplane.AdminRoleAssignment) int {
		if left.RoleCode != right.RoleCode {
			return strings.Compare(left.RoleCode, right.RoleCode)
		}
		return strings.Compare(left.UserID, right.UserID)
	})
	return assignments, nil
}

func boundedPathValue(value string, maxLen int) (string, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" || len(trimmed) > maxLen {
		return "", controlplane.ErrInvalidRequest
	}
	return trimmed, nil
}

func boundedIDPathValue(value string) (string, error) {
	trimmed := strings.TrimSpace(value)
	if !httpIDPattern.MatchString(trimmed) {
		return "", controlplane.ErrInvalidRequest
	}
	return trimmed, nil
}

func successAdminRBACAudit(r *http.Request, actor controlplane.Actor, statusCode int, targetType, targetID string) controlplane.AuditLogInput {
	return controlplane.AuditLogInput{
		ActorUserID: actor.UserID,
		Product:     actor.Product,
		Action:      r.Method + " " + r.URL.Path,
		TargetType:  targetType,
		TargetID:    targetID,
		Outcome:     "success",
		StatusCode:  statusCode,
		RequestID:   RequestIDFromContext(r.Context()),
	}
}
