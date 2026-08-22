package service

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

const (
	adminRoleCodeMaxLen = 128
	adminRoleNameMaxLen = 128
)

type AdminRoleSpec struct {
	Code        string
	Product     controlplane.ProductCode
	Name        string
	Permissions []controlplane.PermissionCode
}

type AdminRoleView struct {
	Code        string
	Product     controlplane.ProductCode
	Name        string
	BuiltIn     bool
	Permissions []controlplane.PermissionCode
}

func (s *ControlPlane) GetAdminAuthorization(ctx context.Context, actor controlplane.Actor) (controlplane.AdminAuthorization, error) {
	if err := checkContext(ctx); err != nil {
		return controlplane.AdminAuthorization{}, err
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return controlplane.AdminAuthorization{}, err
	}
	product, err := normalizeActorProduct(actor)
	if err != nil {
		return controlplane.AdminAuthorization{}, err
	}
	userID := strings.TrimSpace(actor.UserID)
	if userID == "" {
		return controlplane.AdminAuthorization{}, controlplane.ErrUnauthenticated
	}
	return repository.GetAdminAuthorization(ctx, userID, product)
}

func (s *ControlPlane) ListAdminPermissions(ctx context.Context, actor controlplane.Actor) ([]controlplane.PermissionCode, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return nil, err
	}
	if _, _, err := s.requireAdminPermission(ctx, actor, "roles.read"); err != nil {
		return nil, err
	}
	return repository.ListAdminPermissions(ctx)
}

func (s *ControlPlane) ListAdminRoles(ctx context.Context, actor controlplane.Actor, product controlplane.ProductCode) ([]AdminRoleView, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return nil, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.read")
	if err != nil {
		return nil, err
	}
	targetProduct, err := resolveReadProductScope(auth, actorProduct, product)
	if err != nil {
		return nil, err
	}
	roles, err := repository.ListAdminRoles(ctx, targetProduct)
	if err != nil {
		return nil, err
	}
	return mapAdminRoleRecords(roles), nil
}

func (s *ControlPlane) GetAdminRole(ctx context.Context, actor controlplane.Actor, code string) (AdminRoleView, error) {
	if err := checkContext(ctx); err != nil {
		return AdminRoleView{}, err
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return AdminRoleView{}, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.read")
	if err != nil {
		return AdminRoleView{}, err
	}
	role, err := repository.GetAdminRole(ctx, strings.TrimSpace(code))
	if err != nil {
		return AdminRoleView{}, err
	}
	if !auth.GlobalSuperAdmin && role.Product != actorProduct {
		return AdminRoleView{}, controlplane.ErrAdminProductScopeMismatch
	}
	return mapAdminRoleRecord(role), nil
}

func (s *ControlPlane) CreateAdminRole(ctx context.Context, actor controlplane.Actor, idempotencyKey string, input AdminRoleSpec) (AdminRoleView, error) {
	return s.CreateAdminRoleWithAudit(ctx, actor, idempotencyKey, input, controlplane.AuditLogInput{})
}

func (s *ControlPlane) CreateAdminRoleWithAudit(ctx context.Context, actor controlplane.Actor, idempotencyKey string, input AdminRoleSpec, audit controlplane.AuditLogInput) (result AdminRoleView, err error) {
	targetCode := strings.TrimSpace(input.Code)
	auditProduct := auditProductForRole(actor.Product, input.Product)
	defer func() {
		auditErr := s.recordAdminRBACAudit(ctx, auditProduct, audit, "admin_role", targetCode, err)
		if err == nil && auditErr != nil {
			err = auditErr
		}
	}()

	if err = checkContext(ctx); err != nil {
		return AdminRoleView{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return AdminRoleView{}, controlplane.ErrIdempotencyKeyRequired
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return AdminRoleView{}, err
	}
	spec, err := normalizeAdminRoleCreateSpec(input)
	if err != nil {
		return AdminRoleView{}, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.manage")
	if err != nil {
		return AdminRoleView{}, err
	}
	if !auth.GlobalSuperAdmin && spec.Product != actorProduct {
		return AdminRoleView{}, controlplane.ErrAdminProductScopeMismatch
	}
	if !permissionsSubset(spec.Permissions, auth.Permissions) {
		return AdminRoleView{}, controlplane.ErrAdminRoleDelegationForbidden
	}
	fingerprint, err := fingerprintValue(struct {
		Product     controlplane.ProductCode      `json:"product"`
		Code        string                        `json:"code"`
		Name        string                        `json:"name"`
		Permissions []controlplane.PermissionCode `json:"permissions"`
	}{
		Product:     spec.Product,
		Code:        spec.Code,
		Name:        spec.Name,
		Permissions: spec.Permissions,
	})
	if err != nil {
		return AdminRoleView{}, err
	}
	role, err := repository.CreateAdminRole(ctx, store.AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: fmt.Sprintf("create-admin-role:%s:%s:%s", spec.Product, spec.Code, idempotencyKey),
		Fingerprint:    fingerprint,
		Role: store.AdminRoleRecord{
			Code:        spec.Code,
			Product:     spec.Product,
			Name:        spec.Name,
			Permissions: append([]controlplane.PermissionCode(nil), spec.Permissions...),
		},
	})
	if err != nil {
		return AdminRoleView{}, err
	}
	return mapAdminRoleRecord(role), nil
}

func (s *ControlPlane) UpdateAdminRole(ctx context.Context, actor controlplane.Actor, idempotencyKey, code string, input AdminRoleSpec) (AdminRoleView, error) {
	return s.UpdateAdminRoleWithAudit(ctx, actor, idempotencyKey, code, input, controlplane.AuditLogInput{})
}

func (s *ControlPlane) UpdateAdminRoleWithAudit(ctx context.Context, actor controlplane.Actor, idempotencyKey, code string, input AdminRoleSpec, audit controlplane.AuditLogInput) (result AdminRoleView, err error) {
	targetCode := strings.TrimSpace(code)
	auditProduct := auditProductForRole(actor.Product, input.Product)
	defer func() {
		auditErr := s.recordAdminRBACAudit(ctx, auditProduct, audit, "admin_role", targetCode, err)
		if err == nil && auditErr != nil {
			err = auditErr
		}
	}()

	if err = checkContext(ctx); err != nil {
		return AdminRoleView{}, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return AdminRoleView{}, controlplane.ErrIdempotencyKeyRequired
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return AdminRoleView{}, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.manage")
	if err != nil {
		return AdminRoleView{}, err
	}
	existing, err := repository.GetAdminRole(ctx, targetCode)
	if err != nil {
		return AdminRoleView{}, err
	}
	auditProduct = auditProductForRole(actor.Product, existing.Product)
	if existing.BuiltIn {
		return AdminRoleView{}, controlplane.ErrAdminBuiltInRoleImmutable
	}
	if !auth.GlobalSuperAdmin && existing.Product != actorProduct {
		return AdminRoleView{}, controlplane.ErrAdminProductScopeMismatch
	}
	spec, err := normalizeAdminRoleUpdateSpec(targetCode, existing, input)
	if err != nil {
		return AdminRoleView{}, err
	}
	if !permissionsSubset(spec.Permissions, auth.Permissions) {
		return AdminRoleView{}, controlplane.ErrAdminRoleDelegationForbidden
	}
	fingerprint, err := fingerprintValue(struct {
		Product     controlplane.ProductCode      `json:"product"`
		Code        string                        `json:"code"`
		Name        string                        `json:"name"`
		Permissions []controlplane.PermissionCode `json:"permissions"`
	}{
		Product:     spec.Product,
		Code:        spec.Code,
		Name:        spec.Name,
		Permissions: spec.Permissions,
	})
	if err != nil {
		return AdminRoleView{}, err
	}
	role, err := repository.UpdateAdminRole(ctx, store.AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: fmt.Sprintf("update-admin-role:%s:%s:%s", spec.Product, spec.Code, idempotencyKey),
		Fingerprint:    fingerprint,
		Role: store.AdminRoleRecord{
			Code:        spec.Code,
			Product:     spec.Product,
			Name:        spec.Name,
			Permissions: append([]controlplane.PermissionCode(nil), spec.Permissions...),
		},
	})
	if err != nil {
		return AdminRoleView{}, err
	}
	return mapAdminRoleRecord(role), nil
}

func (s *ControlPlane) DeleteAdminRole(ctx context.Context, actor controlplane.Actor, idempotencyKey, code string) error {
	return s.DeleteAdminRoleWithAudit(ctx, actor, idempotencyKey, code, controlplane.AuditLogInput{})
}

func (s *ControlPlane) DeleteAdminRoleWithAudit(ctx context.Context, actor controlplane.Actor, idempotencyKey, code string, audit controlplane.AuditLogInput) (err error) {
	targetCode := strings.TrimSpace(code)
	auditProduct := auditProductForRole(actor.Product, "")
	defer func() {
		auditErr := s.recordAdminRBACAudit(ctx, auditProduct, audit, "admin_role", targetCode, err)
		if err == nil && auditErr != nil {
			err = auditErr
		}
	}()

	if err = checkContext(ctx); err != nil {
		return err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return controlplane.ErrIdempotencyKeyRequired
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.manage")
	if err != nil {
		return err
	}
	role, err := repository.GetAdminRole(ctx, targetCode)
	if err != nil {
		return err
	}
	auditProduct = auditProductForRole(actor.Product, role.Product)
	if !auth.GlobalSuperAdmin && role.Product != actorProduct {
		return controlplane.ErrAdminProductScopeMismatch
	}
	fingerprint, err := fingerprintValue(struct {
		Product controlplane.ProductCode `json:"product"`
		Code    string                   `json:"code"`
	}{Product: role.Product, Code: role.Code})
	if err != nil {
		return err
	}
	return repository.DeleteAdminRole(ctx, store.AdminRoleDeleteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: fmt.Sprintf("delete-admin-role:%s:%s:%s", role.Product, role.Code, idempotencyKey),
		Fingerprint:    fingerprint,
		Code:           role.Code,
	})
}

func (s *ControlPlane) ListUserAdminRoles(ctx context.Context, actor controlplane.Actor, userID string, product controlplane.ProductCode) ([]controlplane.AdminRoleAssignment, error) {
	if err := checkContext(ctx); err != nil {
		return nil, err
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return nil, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.assign")
	if err != nil {
		return nil, err
	}
	targetUserID := strings.TrimSpace(userID)
	if targetUserID == "" {
		return nil, controlplane.ErrUserNotFound
	}
	scopeProduct, allowGlobal, err := resolveAssignmentScope(auth, actorProduct, product)
	if err != nil {
		return nil, err
	}
	assignments, err := repository.ListUserAdminRoles(ctx, targetUserID, scopeProduct)
	if err != nil {
		return nil, err
	}
	if allowGlobal || auth.GlobalSuperAdmin {
		return assignments, nil
	}
	return filterAssignmentsByScope(assignments, scopeProduct), nil
}

func (s *ControlPlane) ReplaceUserAdminRoles(ctx context.Context, actor controlplane.Actor, idempotencyKey, userID string, product controlplane.ProductCode, assignments []controlplane.AdminRoleAssignment) ([]controlplane.AdminRoleAssignment, error) {
	return s.ReplaceUserAdminRolesWithAudit(ctx, actor, idempotencyKey, userID, product, assignments, controlplane.AuditLogInput{})
}

func (s *ControlPlane) ReplaceUserAdminRolesWithAudit(ctx context.Context, actor controlplane.Actor, idempotencyKey, userID string, product controlplane.ProductCode, assignments []controlplane.AdminRoleAssignment, audit controlplane.AuditLogInput) (result []controlplane.AdminRoleAssignment, err error) {
	targetUserID := strings.TrimSpace(userID)
	auditProduct := auditProductForRole(actor.Product, product)
	defer func() {
		auditErr := s.recordAdminRBACAudit(ctx, auditProduct, audit, "admin_role_assignment", targetUserID, err)
		if err == nil && auditErr != nil {
			err = auditErr
		}
	}()

	if err = checkContext(ctx); err != nil {
		return nil, err
	}
	if !validIdempotencyKey(idempotencyKey) {
		return nil, controlplane.ErrIdempotencyKeyRequired
	}
	if targetUserID == "" {
		return nil, controlplane.ErrUserNotFound
	}
	repository, err := s.adminRBACRepository()
	if err != nil {
		return nil, err
	}
	auth, actorProduct, err := s.requireAdminPermission(ctx, actor, "roles.assign")
	if err != nil {
		return nil, err
	}
	scopeProduct, globalScope, err := resolveAssignmentScope(auth, actorProduct, product)
	if err != nil {
		return nil, err
	}
	normalizedScopeAssignments, err := s.normalizeDesiredAssignments(ctx, repository, auth, targetUserID, scopeProduct, globalScope, assignments)
	if err != nil {
		return nil, err
	}
	existingAssignments, err := repository.ListUserAdminRoles(ctx, targetUserID, "")
	if err != nil {
		return nil, err
	}
	mergedAssignments := mergeScopedAssignments(existingAssignments, normalizedScopeAssignments, scopeProduct, globalScope)
	if err := s.ensureLastSuperAdminProtection(ctx, targetUserID, mergedAssignments); err != nil {
		return nil, err
	}
	fingerprint, err := fingerprintValue(struct {
		UserID      string                             `json:"user_id"`
		Product     controlplane.ProductCode           `json:"product"`
		GlobalScope bool                               `json:"global_scope"`
		Assignments []controlplane.AdminRoleAssignment `json:"assignments"`
	}{
		UserID:      targetUserID,
		Product:     scopeProduct,
		GlobalScope: globalScope,
		Assignments: normalizedScopeAssignments,
	})
	if err != nil {
		return nil, err
	}
	scopeLabel := string(scopeProduct)
	if globalScope {
		scopeLabel = "global"
	}
	result, err = repository.ReplaceUserAdminRoles(ctx, store.UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: fmt.Sprintf("replace-user-admin-roles:%s:%s:%s", targetUserID, scopeLabel, idempotencyKey),
		Fingerprint:    fingerprint,
		UserID:         targetUserID,
		Assignments:    mergedAssignments,
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

func (s *ControlPlane) adminRBACRepository() (store.AdminRBACRepository, error) {
	repository, ok := s.repository.(store.AdminRBACRepository)
	if ok {
		return repository, nil
	}
	return nil, store.ErrNormalizedAdminRBACRepositoryRequired
}

func normalizeActorProduct(actor controlplane.Actor) (controlplane.ProductCode, error) {
	product := controlplane.ProductCode(strings.TrimSpace(string(actor.Product)))
	if product == "" {
		return controlplane.ProductAutoLive, nil
	}
	if !product.Valid() {
		return "", controlplane.ErrInvalidRequest
	}
	return product, nil
}

func (s *ControlPlane) requireAdminPermission(ctx context.Context, actor controlplane.Actor, permission controlplane.PermissionCode) (controlplane.AdminAuthorization, controlplane.ProductCode, error) {
	product, err := normalizeActorProduct(actor)
	if err != nil {
		return controlplane.AdminAuthorization{}, "", err
	}
	auth, err := s.GetAdminAuthorization(ctx, controlplane.Actor{UserID: actor.UserID, Product: product})
	if err != nil {
		return controlplane.AdminAuthorization{}, "", err
	}
	if !slices.Contains(auth.Permissions, permission) {
		return controlplane.AdminAuthorization{}, "", controlplane.ErrAdminPermissionDenied
	}
	return auth, product, nil
}

func resolveReadProductScope(auth controlplane.AdminAuthorization, actorProduct controlplane.ProductCode, requested controlplane.ProductCode) (controlplane.ProductCode, error) {
	if requested == "" {
		if auth.GlobalSuperAdmin {
			return "", nil
		}
		return actorProduct, nil
	}
	if !requested.Valid() {
		return "", controlplane.ErrInvalidRequest
	}
	if !auth.GlobalSuperAdmin && requested != actorProduct {
		return "", controlplane.ErrAdminProductScopeMismatch
	}
	return requested, nil
}

func resolveAssignmentScope(auth controlplane.AdminAuthorization, actorProduct controlplane.ProductCode, requested controlplane.ProductCode) (controlplane.ProductCode, bool, error) {
	if requested == "" {
		if auth.GlobalSuperAdmin {
			return "", true, nil
		}
		return "", false, controlplane.ErrAdminRoleDelegationForbidden
	}
	if !requested.Valid() {
		return "", false, controlplane.ErrInvalidRequest
	}
	if !auth.GlobalSuperAdmin && requested != actorProduct {
		return "", false, controlplane.ErrAdminProductScopeMismatch
	}
	return requested, false, nil
}

func normalizeAdminRoleCreateSpec(input AdminRoleSpec) (AdminRoleSpec, error) {
	spec := AdminRoleSpec{
		Code:    strings.TrimSpace(input.Code),
		Product: controlplane.ProductCode(strings.TrimSpace(string(input.Product))),
		Name:    strings.TrimSpace(input.Name),
	}
	if spec.Code == "" || spec.Name == "" || len(spec.Code) > adminRoleCodeMaxLen || len(spec.Name) > adminRoleNameMaxLen {
		return AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	if spec.Code == controlplane.BuiltinAdminRoleSuperAdmin || !spec.Product.Valid() {
		return AdminRoleSpec{}, controlplane.ErrAdminProductScopeMismatch
	}
	permissions, err := normalizeServicePermissionCodes(input.Permissions)
	if err != nil {
		return AdminRoleSpec{}, err
	}
	spec.Permissions = permissions
	return spec, nil
}

func normalizeAdminRoleUpdateSpec(code string, existing store.AdminRoleRecord, input AdminRoleSpec) (AdminRoleSpec, error) {
	spec := AdminRoleSpec{
		Code:    strings.TrimSpace(code),
		Product: existing.Product,
		Name:    strings.TrimSpace(input.Name),
	}
	if spec.Code == "" || spec.Name == "" || len(spec.Code) > adminRoleCodeMaxLen || len(spec.Name) > adminRoleNameMaxLen {
		return AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	if input.Code != "" && strings.TrimSpace(input.Code) != spec.Code {
		return AdminRoleSpec{}, controlplane.ErrInvalidRequest
	}
	if input.Product != "" && input.Product != existing.Product {
		return AdminRoleSpec{}, controlplane.ErrAdminProductScopeMismatch
	}
	permissions, err := normalizeServicePermissionCodes(input.Permissions)
	if err != nil {
		return AdminRoleSpec{}, err
	}
	spec.Permissions = permissions
	return spec, nil
}

func normalizeServicePermissionCodes(values []controlplane.PermissionCode) ([]controlplane.PermissionCode, error) {
	raw := make([]string, 0, len(values))
	for _, permission := range values {
		raw = append(raw, string(permission))
	}
	normalized, err := controlplane.NormalizePermissionCodes(raw)
	if err != nil {
		return nil, err
	}
	result := make([]controlplane.PermissionCode, 0, len(normalized))
	for _, permission := range normalized {
		result = append(result, controlplane.PermissionCode(permission))
	}
	return result, nil
}

func permissionsSubset(target []controlplane.PermissionCode, owned []controlplane.PermissionCode) bool {
	if len(target) == 0 {
		return true
	}
	ownedSet := make(map[controlplane.PermissionCode]struct{}, len(owned))
	for _, permission := range owned {
		ownedSet[permission] = struct{}{}
	}
	for _, permission := range target {
		if _, ok := ownedSet[permission]; !ok {
			return false
		}
	}
	return true
}

func mapAdminRoleRecords(records []store.AdminRoleRecord) []AdminRoleView {
	result := make([]AdminRoleView, 0, len(records))
	for _, record := range records {
		result = append(result, mapAdminRoleRecord(record))
	}
	return result
}

func mapAdminRoleRecord(record store.AdminRoleRecord) AdminRoleView {
	return AdminRoleView{
		Code:        record.Code,
		Product:     record.Product,
		Name:        record.Name,
		BuiltIn:     record.BuiltIn,
		Permissions: append([]controlplane.PermissionCode(nil), record.Permissions...),
	}
}

func (s *ControlPlane) normalizeDesiredAssignments(ctx context.Context, repository store.AdminRBACRepository, auth controlplane.AdminAuthorization, userID string, scopeProduct controlplane.ProductCode, globalScope bool, assignments []controlplane.AdminRoleAssignment) ([]controlplane.AdminRoleAssignment, error) {
	normalized := make(map[string]controlplane.AdminRoleAssignment, len(assignments))
	for _, assignment := range assignments {
		current := controlplane.AdminRoleAssignment{
			UserID:   strings.TrimSpace(assignment.UserID),
			RoleCode: strings.TrimSpace(assignment.RoleCode),
			Product:  controlplane.ProductCode(strings.TrimSpace(string(assignment.Product))),
		}
		if current.UserID == "" {
			current.UserID = userID
		}
		if current.UserID != userID || current.RoleCode == "" {
			return nil, controlplane.ErrInvalidRequest
		}
		role, err := repository.GetAdminRole(ctx, current.RoleCode)
		if err != nil {
			return nil, err
		}
		if globalScope {
			if role.Code != controlplane.BuiltinAdminRoleSuperAdmin || current.Product != "" {
				return nil, controlplane.ErrAdminRoleDelegationForbidden
			}
		} else {
			if current.Product != scopeProduct {
				return nil, controlplane.ErrAdminProductScopeMismatch
			}
			if role.Code == controlplane.BuiltinAdminRoleSuperAdmin {
				return nil, controlplane.ErrAdminRoleDelegationForbidden
			}
			if role.Product != scopeProduct {
				return nil, controlplane.ErrAdminProductScopeMismatch
			}
			if !auth.GlobalSuperAdmin && !permissionsSubset(role.Permissions, auth.Permissions) {
				return nil, controlplane.ErrAdminRoleDelegationForbidden
			}
		}
		key := current.UserID + "\x1f" + current.RoleCode + "\x1f" + string(current.Product)
		normalized[key] = current
	}
	result := make([]controlplane.AdminRoleAssignment, 0, len(normalized))
	for _, assignment := range normalized {
		result = append(result, assignment)
	}
	slices.SortFunc(result, compareAdminRoleAssignments)
	return result, nil
}

func mergeScopedAssignments(existing []controlplane.AdminRoleAssignment, desired []controlplane.AdminRoleAssignment, scopeProduct controlplane.ProductCode, globalScope bool) []controlplane.AdminRoleAssignment {
	merged := make([]controlplane.AdminRoleAssignment, 0, len(existing)+len(desired))
	for _, assignment := range existing {
		if globalScope {
			if assignment.Product == "" {
				continue
			}
		} else if assignment.Product == scopeProduct {
			continue
		}
		merged = append(merged, assignment)
	}
	merged = append(merged, desired...)
	slices.SortFunc(merged, compareAdminRoleAssignments)
	return merged
}

func compareAdminRoleAssignments(left, right controlplane.AdminRoleAssignment) int {
	if left.Product != right.Product {
		return strings.Compare(string(left.Product), string(right.Product))
	}
	if left.RoleCode != right.RoleCode {
		return strings.Compare(left.RoleCode, right.RoleCode)
	}
	return strings.Compare(left.UserID, right.UserID)
}

func filterAssignmentsByScope(assignments []controlplane.AdminRoleAssignment, product controlplane.ProductCode) []controlplane.AdminRoleAssignment {
	result := make([]controlplane.AdminRoleAssignment, 0, len(assignments))
	for _, assignment := range assignments {
		if assignment.Product == product {
			result = append(result, assignment)
		}
	}
	slices.SortFunc(result, compareAdminRoleAssignments)
	return result
}

func (s *ControlPlane) ensureLastSuperAdminProtection(ctx context.Context, targetUserID string, next []controlplane.AdminRoleAssignment) error {
	if source, ok := s.repository.(store.NormalizedReadSource); ok && source.UsesNormalizedReadSource() {
		return nil
	}
	targetUserID = strings.TrimSpace(targetUserID)
	if targetUserID == "" {
		return controlplane.ErrUserNotFound
	}
	return s.repository.Run(ctx, func(state *store.State) error {
		user, ok := state.Users[targetUserID]
		if !ok {
			return controlplane.ErrUserNotFound
		}
		if user.Status == controlplane.UserStatusDisabled {
			return controlplane.ErrUserDisabled
		}
		if !isEffectiveGlobalSuperAdmin(user, listAssignmentsFromState(state, targetUserID)) {
			return nil
		}
		if isEffectiveGlobalSuperAdmin(user, next) {
			return nil
		}
		for userID, candidate := range state.Users {
			if userID == targetUserID || candidate.Status != controlplane.UserStatusActive {
				continue
			}
			if isEffectiveGlobalSuperAdmin(candidate, listAssignmentsFromState(state, userID)) {
				return nil
			}
		}
		return controlplane.ErrAdminLastSuperAdminProtected
	})
}

func isEffectiveGlobalSuperAdmin(user controlplane.UserSummary, assignments []controlplane.AdminRoleAssignment) bool {
	if user.ID == "usr_local_admin" && user.Role == controlplane.RoleAdmin && user.Status == controlplane.UserStatusActive {
		return true
	}
	return slices.ContainsFunc(assignments, func(assignment controlplane.AdminRoleAssignment) bool {
		return assignment.RoleCode == controlplane.BuiltinAdminRoleSuperAdmin && assignment.Product == ""
	})
}

func listAssignmentsFromState(state *store.State, userID string) []controlplane.AdminRoleAssignment {
	assignments := make([]controlplane.AdminRoleAssignment, 0, len(state.UserAdminRoles))
	for _, assignment := range state.UserAdminRoles {
		if assignment.UserID == userID {
			assignments = append(assignments, assignment)
		}
	}
	slices.SortFunc(assignments, compareAdminRoleAssignments)
	return assignments
}

func auditProductForRole(actorProduct, targetProduct controlplane.ProductCode) controlplane.ProductCode {
	if targetProduct.Valid() {
		return targetProduct
	}
	product := controlplane.ProductCode(strings.TrimSpace(string(actorProduct)))
	if product.Valid() {
		return product
	}
	return controlplane.ProductAutoLive
}

func (s *ControlPlane) recordAdminRBACAudit(ctx context.Context, product controlplane.ProductCode, audit controlplane.AuditLogInput, targetType, targetID string, operationErr error) error {
	if ctx == nil {
		return nil
	}
	if strings.TrimSpace(audit.Action) == "" {
		return nil
	}
	entry := controlplane.AuditLogInput{
		ActorUserID: strings.TrimSpace(audit.ActorUserID),
		DeviceID:    strings.TrimSpace(audit.DeviceID),
		Action:      strings.TrimSpace(audit.Action),
		TargetType:  targetType,
		TargetID:    strings.TrimSpace(targetID),
		RequestID:   strings.TrimSpace(audit.RequestID),
	}
	if targetType == "" {
		entry.TargetType = strings.TrimSpace(audit.TargetType)
	}
	if operationErr == nil {
		entry.Outcome = "success"
		if audit.StatusCode > 0 {
			entry.StatusCode = audit.StatusCode
		} else {
			entry.StatusCode = 200
		}
	} else {
		entry.Outcome = "failure"
		var appErr *controlplane.Error
		if errors.As(operationErr, &appErr) {
			entry.StatusCode = appErr.Status
			entry.ErrorCode = appErr.Code
		} else if audit.StatusCode >= 400 {
			entry.StatusCode = audit.StatusCode
		} else {
			entry.StatusCode = 500
		}
	}
	return s.recordAuditForProduct(ctx, auditProductForRole(product, product), entry)
}
