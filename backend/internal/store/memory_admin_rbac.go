package store

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ AdminRBACRepository = (*MemoryStore)(nil)

func seedAdminRBACState(state *State) {
	if state.AdminPermissions == nil {
		state.AdminPermissions = map[string]controlplane.PermissionCode{}
	}
	if state.AdminRoles == nil {
		state.AdminRoles = map[string]AdminRoleRecord{}
	}
	if state.AdminRolePermissions == nil {
		state.AdminRolePermissions = map[string][]controlplane.PermissionCode{}
	}
	if state.UserAdminRoles == nil {
		state.UserAdminRoles = map[string]controlplane.AdminRoleAssignment{}
	}

	for _, permission := range controlplane.PermissionCatalog() {
		state.AdminPermissions[string(permission)] = permission
	}

	state.AdminRoles[controlplane.BuiltinAdminRoleSuperAdmin] = AdminRoleRecord{
		Code:    controlplane.BuiltinAdminRoleSuperAdmin,
		Name:    "超级管理员",
		BuiltIn: true,
	}
	state.AdminRolePermissions[controlplane.BuiltinAdminRoleSuperAdmin] = normalizedCatalogPermissions()
}

func normalizedCatalogPermissions() []controlplane.PermissionCode {
	values := controlplane.PermissionCatalog()
	codes := make([]string, 0, len(values))
	for _, permission := range values {
		codes = append(codes, string(permission))
	}
	normalized, err := controlplane.NormalizePermissionCodes(codes)
	if err != nil {
		panic(err)
	}
	result := make([]controlplane.PermissionCode, 0, len(normalized))
	for _, permission := range normalized {
		result = append(result, controlplane.PermissionCode(permission))
	}
	return result
}

func (s *MemoryStore) ListAdminPermissions(ctx context.Context) ([]controlplane.PermissionCode, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return append([]controlplane.PermissionCode(nil), controlplane.PermissionCatalog()...), nil
}

func (s *MemoryStore) ListAdminRoles(ctx context.Context, product controlplane.ProductCode) ([]AdminRoleRecord, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if product != "" && !product.Valid() {
		return nil, controlplane.ErrInvalidRequest
	}

	var roles []AdminRoleRecord
	err := s.Run(ctx, func(state *State) error {
		ordinaryRoles := make([]AdminRoleRecord, 0, len(state.AdminRoles))
		var globalSuperAdmin *AdminRoleRecord
		for code := range state.AdminRoles {
			role, ok := memoryAdminRoleRecord(state, code)
			if !ok {
				continue
			}
			if role.Code == controlplane.BuiltinAdminRoleSuperAdmin && role.Product == "" {
				roleCopy := role
				globalSuperAdmin = &roleCopy
				continue
			}
			if product != "" && role.Product != product {
				continue
			}
			ordinaryRoles = append(ordinaryRoles, role)
		}
		sortAdminRoleRecords(ordinaryRoles)
		limit := adminRBACScopedRoleLimit
		if product != "" {
			limit = adminRBACListLimit
		}
		if len(ordinaryRoles) > limit {
			ordinaryRoles = ordinaryRoles[:limit]
		}
		items := append([]AdminRoleRecord(nil), ordinaryRoles...)
		if globalSuperAdmin != nil {
			items = append(items, *globalSuperAdmin)
		}
		sortAdminRoleRecords(items)
		roles = append([]AdminRoleRecord(nil), items...)
		return nil
	})
	return roles, err
}

func (s *MemoryStore) GetAdminRole(ctx context.Context, code string) (AdminRoleRecord, error) {
	if err := ctx.Err(); err != nil {
		return AdminRoleRecord{}, err
	}
	code = strings.TrimSpace(code)
	if code == "" {
		return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
	}

	var role AdminRoleRecord
	err := s.Run(ctx, func(state *State) error {
		current, ok := memoryAdminRoleRecord(state, code)
		if !ok {
			return controlplane.ErrAdminRoleNotFound
		}
		role = current
		return nil
	})
	return role, err
}

func (s *MemoryStore) CreateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error) {
	if err := ctx.Err(); err != nil {
		return AdminRoleRecord{}, err
	}
	role, err := normalizeAdminRoleWriteRecord(record)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	record.Audit, err = normalizeOptionalAuditInputForProduct(record.Audit, role.Product)
	if err != nil {
		return AdminRoleRecord{}, err
	}

	var created AdminRoleRecord
	err = s.Run(ctx, func(state *State) error {
		if replayed, existingRole, err := checkAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code, func() (AdminRoleRecord, error) {
			current, ok := memoryAdminRoleRecord(state, role.Code)
			if !ok {
				return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
			}
			return current, nil
		}); err != nil {
			return err
		} else if replayed {
			recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, role.Code)
			created = existingRole
			return nil
		}

		if _, exists := state.AdminRoles[role.Code]; exists {
			return controlplane.ErrInvalidRequest
		}

		state.AdminRoles[role.Code] = AdminRoleRecord{
			Code:    role.Code,
			Product: role.Product,
			Name:    role.Name,
			BuiltIn: role.BuiltIn,
		}
		state.AdminRolePermissions[role.Code] = append([]controlplane.PermissionCode(nil), role.Permissions...)
		storeAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code)
		recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, role.Code)
		created, _ = memoryAdminRoleRecord(state, role.Code)
		return nil
	})
	return created, err
}

func (s *MemoryStore) UpdateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error) {
	if err := ctx.Err(); err != nil {
		return AdminRoleRecord{}, err
	}
	role, err := normalizeAdminRoleWriteRecord(record)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	record.Audit, err = normalizeOptionalAuditInputForProduct(record.Audit, role.Product)
	if err != nil {
		return AdminRoleRecord{}, err
	}

	var updated AdminRoleRecord
	err = s.Run(ctx, func(state *State) error {
		if replayed, existingRole, err := checkAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code, func() (AdminRoleRecord, error) {
			current, ok := memoryAdminRoleRecord(state, role.Code)
			if !ok {
				return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
			}
			return current, nil
		}); err != nil {
			return err
		} else if replayed {
			recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, role.Code)
			updated = existingRole
			return nil
		}

		current, ok := state.AdminRoles[role.Code]
		if !ok {
			return controlplane.ErrAdminRoleNotFound
		}
		if current.BuiltIn {
			return controlplane.ErrAdminBuiltInRoleImmutable
		}
		if current.Product != role.Product {
			return controlplane.ErrAdminProductScopeMismatch
		}

		state.AdminRoles[role.Code] = AdminRoleRecord{
			Code:    current.Code,
			Product: current.Product,
			Name:    role.Name,
			BuiltIn: current.BuiltIn,
		}
		state.AdminRolePermissions[role.Code] = append([]controlplane.PermissionCode(nil), role.Permissions...)
		storeAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code)
		recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, role.Code)
		updated, _ = memoryAdminRoleRecord(state, role.Code)
		return nil
	})
	return updated, err
}

func (s *MemoryStore) DeleteAdminRole(ctx context.Context, record AdminRoleDeleteRecord) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.Code = strings.TrimSpace(record.Code)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.Code == "" {
		return controlplane.ErrInvalidRequest
	}

	return s.Run(ctx, func(state *State) error {
		if replayed, _, err := checkAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Code, func() (AdminRoleRecord, error) {
			return AdminRoleRecord{}, nil
		}); err != nil {
			return err
		} else if replayed {
			recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, record.Code)
			return nil
		}

		role, ok := state.AdminRoles[record.Code]
		if !ok {
			return controlplane.ErrAdminRoleNotFound
		}
		audit, err := normalizeOptionalAuditInputForProduct(record.Audit, role.Product)
		if err != nil {
			return err
		}
		if role.BuiltIn {
			return controlplane.ErrAdminBuiltInRoleImmutable
		}
		for _, assignment := range state.UserAdminRoles {
			if assignment.RoleCode == record.Code {
				return controlplane.ErrAdminRoleAssigned
			}
		}
		delete(state.AdminRoles, record.Code)
		delete(state.AdminRolePermissions, record.Code)
		storeAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Code)
		recordMemoryAdminRBACSuccessAudit(state, s.Now(), audit, record.Code)
		return nil
	})
}

func (s *MemoryStore) ListUserAdminRoles(ctx context.Context, userID string, product controlplane.ProductCode) ([]controlplane.AdminRoleAssignment, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return nil, controlplane.ErrUserNotFound
	}
	if product != "" && !product.Valid() {
		return nil, controlplane.ErrInvalidRequest
	}

	var assignments []controlplane.AdminRoleAssignment
	err := s.Run(ctx, func(state *State) error {
		if _, ok := state.Users[userID]; !ok {
			return controlplane.ErrUserNotFound
		}
		assignments = listUserAdminRolesFromState(state, userID, product)
		return nil
	})
	return assignments, err
}

func (s *MemoryStore) ReplaceUserAdminRoles(ctx context.Context, record UserAdminRoleReplaceRecord) ([]controlplane.AdminRoleAssignment, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" {
		return nil, controlplane.ErrInvalidRequest
	}
	if strings.TrimSpace(record.Audit.Action) != "" {
		if !record.Audit.Product.Valid() {
			return nil, controlplane.ErrInvalidRequest
		}
		var err error
		record.Audit, err = normalizeAuditInputForProduct(record.Audit, record.Audit.Product)
		if err != nil {
			return nil, err
		}
	}

	var assignments []controlplane.AdminRoleAssignment
	err := s.Run(ctx, func(state *State) error {
		user, ok := state.Users[record.UserID]
		if !ok {
			return controlplane.ErrUserNotFound
		}
		if user.Status == controlplane.UserStatusDisabled {
			return controlplane.ErrUserDisabled
		}
		if replayed, _, err := checkAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, record.UserID, func() (AdminRoleRecord, error) {
			return AdminRoleRecord{}, nil
		}); err != nil {
			return err
		} else if replayed {
			recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, record.UserID)
			assignments = listUserAdminRolesFromState(state, record.UserID, "")
			return nil
		}

		normalized := make(map[string]controlplane.AdminRoleAssignment, len(record.Assignments))
		for _, assignment := range record.Assignments {
			current, err := normalizeAdminRoleAssignment(state, record.UserID, assignment)
			if err != nil {
				return err
			}
			normalized[adminRoleBindingKey(current.UserID, current.RoleCode, current.Product)] = current
		}

		for key, assignment := range state.UserAdminRoles {
			if assignment.UserID == record.UserID {
				delete(state.UserAdminRoles, key)
			}
		}
		for key, assignment := range normalized {
			state.UserAdminRoles[key] = assignment
		}
		storeAdminRBACIdempotency(state, record.Scope, record.IdempotencyKey, record.Fingerprint, record.UserID)
		recordMemoryAdminRBACSuccessAudit(state, s.Now(), record.Audit, record.UserID)
		assignments = listUserAdminRolesFromState(state, record.UserID, "")
		return nil
	})
	return assignments, err
}

func (s *MemoryStore) GetAdminAuthorization(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.AdminAuthorization, error) {
	if err := ctx.Err(); err != nil {
		return controlplane.AdminAuthorization{}, err
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.AdminAuthorization{}, controlplane.ErrUserNotFound
	}
	if product != "" && !product.Valid() {
		return controlplane.AdminAuthorization{}, controlplane.ErrInvalidRequest
	}

	var authorization controlplane.AdminAuthorization
	err := s.Run(ctx, func(state *State) error {
		user, ok := state.Users[userID]
		if !ok {
			return controlplane.ErrUserNotFound
		}
		if user.Status == controlplane.UserStatusDisabled {
			return controlplane.ErrUserDisabled
		}

		assignments := listUserAdminRolesFromState(state, userID, product)
		globalSuperAdmin := compatibilityLocalSuperAdmin(user) || containsGlobalSuperAdmin(assignments)
		permissionSet := map[string]struct{}{}
		roleSet := map[string]struct{}{}
		for _, assignment := range assignments {
			roleSet[assignment.RoleCode] = struct{}{}
			for _, permission := range state.AdminRolePermissions[assignment.RoleCode] {
				permissionSet[string(permission)] = struct{}{}
			}
		}
		if globalSuperAdmin {
			roleSet[controlplane.BuiltinAdminRoleSuperAdmin] = struct{}{}
			for _, permission := range normalizedCatalogPermissions() {
				permissionSet[string(permission)] = struct{}{}
			}
		}

		roleCodes := make([]string, 0, len(roleSet))
		for code := range roleSet {
			roleCodes = append(roleCodes, code)
		}
		sort.Strings(roleCodes)

		permissions := make([]controlplane.PermissionCode, 0, len(permissionSet))
		for permission := range permissionSet {
			permissions = append(permissions, controlplane.PermissionCode(permission))
		}
		sort.Slice(permissions, func(i, j int) bool { return permissions[i] < permissions[j] })

		authorization = controlplane.AdminAuthorization{
			UserID:           userID,
			Product:          product,
			GlobalSuperAdmin: globalSuperAdmin,
			RoleCodes:        roleCodes,
			Permissions:      permissions,
		}
		return nil
	})
	return authorization, err
}

func normalizeAdminRoleWriteRecord(record AdminRoleWriteRecord) (AdminRoleRecord, error) {
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	role := record.Role
	role.Code = strings.TrimSpace(role.Code)
	role.Name = strings.TrimSpace(role.Name)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || role.Code == "" || role.Name == "" {
		return AdminRoleRecord{}, controlplane.ErrInvalidRequest
	}
	if role.Code == controlplane.BuiltinAdminRoleSuperAdmin {
		if role.Product != "" {
			return AdminRoleRecord{}, controlplane.ErrAdminProductScopeMismatch
		}
		role.BuiltIn = true
	} else {
		if !role.Product.Valid() {
			return AdminRoleRecord{}, controlplane.ErrAdminProductScopeMismatch
		}
		role.BuiltIn = false
	}

	permissions, err := normalizePermissionCodesForStore(role.Permissions)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	role.Permissions = permissions
	return role, nil
}

func normalizePermissionCodesForStore(values []controlplane.PermissionCode) ([]controlplane.PermissionCode, error) {
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

func normalizeAdminRoleAssignment(state *State, userID string, assignment controlplane.AdminRoleAssignment) (controlplane.AdminRoleAssignment, error) {
	assignment.UserID = strings.TrimSpace(assignment.UserID)
	assignment.RoleCode = strings.TrimSpace(assignment.RoleCode)
	if assignment.UserID == "" {
		assignment.UserID = userID
	}
	if assignment.UserID != userID || assignment.RoleCode == "" {
		return controlplane.AdminRoleAssignment{}, controlplane.ErrInvalidRequest
	}

	role, ok := state.AdminRoles[assignment.RoleCode]
	if !ok {
		return controlplane.AdminRoleAssignment{}, controlplane.ErrAdminRoleNotFound
	}
	if assignment.RoleCode == controlplane.BuiltinAdminRoleSuperAdmin {
		if assignment.Product != "" {
			return controlplane.AdminRoleAssignment{}, controlplane.ErrAdminProductScopeMismatch
		}
		return controlplane.AdminRoleAssignment{UserID: userID, RoleCode: assignment.RoleCode}, nil
	}
	if !assignment.Product.Valid() || role.Product != assignment.Product || !memoryUserHasProduct(state, userID, assignment.Product) {
		return controlplane.AdminRoleAssignment{}, controlplane.ErrAdminProductScopeMismatch
	}
	return controlplane.AdminRoleAssignment{
		UserID:   userID,
		RoleCode: assignment.RoleCode,
		Product:  assignment.Product,
	}, nil
}

func memoryAdminRoleRecord(state *State, code string) (AdminRoleRecord, bool) {
	role, ok := state.AdminRoles[code]
	if !ok {
		return AdminRoleRecord{}, false
	}
	role.Permissions = append([]controlplane.PermissionCode(nil), state.AdminRolePermissions[code]...)
	sort.Slice(role.Permissions, func(i, j int) bool { return role.Permissions[i] < role.Permissions[j] })
	return role, true
}

func listUserAdminRolesFromState(state *State, userID string, product controlplane.ProductCode) []controlplane.AdminRoleAssignment {
	items := make([]controlplane.AdminRoleAssignment, 0, len(state.UserAdminRoles)+1)
	for _, assignment := range state.UserAdminRoles {
		if assignment.UserID != userID {
			continue
		}
		if product != "" && assignment.Product != "" && assignment.Product != product {
			continue
		}
		items = append(items, assignment)
	}
	if compatibilityLocalSuperAdmin(state.Users[userID]) {
		global := controlplane.AdminRoleAssignment{
			UserID:   userID,
			RoleCode: controlplane.BuiltinAdminRoleSuperAdmin,
		}
		key := adminRoleBindingKey(global.UserID, global.RoleCode, global.Product)
		found := false
		for _, item := range items {
			if adminRoleBindingKey(item.UserID, item.RoleCode, item.Product) == key {
				found = true
				break
			}
		}
		if !found {
			items = append(items, global)
		}
	}
	sortAdminRoleAssignments(items)
	return append([]controlplane.AdminRoleAssignment(nil), items...)
}

func checkAdminRBACIdempotency(
	state *State,
	scope string,
	idempotencyKey string,
	fingerprint string,
	resourceID string,
	loadReplay func() (AdminRoleRecord, error),
) (bool, AdminRoleRecord, error) {
	key := memoryIdempotencyKey(scope, idempotencyKey)
	if existing, ok := state.IdempotencyRecords[key]; ok {
		if existing.Fingerprint != fingerprint || existing.ResourceID != resourceID {
			return false, AdminRoleRecord{}, controlplane.ErrIdempotencyConflict
		}
		replayed, err := loadReplay()
		return true, replayed, err
	}
	return false, AdminRoleRecord{}, nil
}

func storeAdminRBACIdempotency(state *State, scope string, idempotencyKey string, fingerprint string, resourceID string) {
	state.IdempotencyRecords[memoryIdempotencyKey(scope, idempotencyKey)] = IdempotencyRecord{
		Fingerprint: fingerprint,
		ResourceID:  resourceID,
	}
}

func recordMemoryAdminRBACSuccessAudit(state *State, now time.Time, input controlplane.AuditLogInput, targetID string) {
	if strings.TrimSpace(input.Action) == "" {
		return
	}
	if input.RequestID != "" {
		for _, auditLog := range state.AuditLogs {
			if auditLog.RequestID == input.RequestID {
				return
			}
		}
	}
	targetID = strings.TrimSpace(targetID)
	if targetID == "" {
		targetID = strings.TrimSpace(input.TargetID)
	}
	state.SequenceCounters["audit"]++
	id := fmt.Sprintf("audit_%08d", state.SequenceCounters["audit"])
	state.AuditLogs[id] = controlplane.AuditLog{
		ID:          id,
		Product:     input.Product,
		ActorUserID: input.ActorUserID,
		DeviceID:    input.DeviceID,
		Action:      input.Action,
		TargetType:  input.TargetType,
		TargetID:    targetID,
		Outcome:     input.Outcome,
		StatusCode:  input.StatusCode,
		ErrorCode:   input.ErrorCode,
		RequestID:   input.RequestID,
		CreatedAt:   now.UTC().Format(time.RFC3339),
	}
}

func memoryIdempotencyKey(scope, key string) string {
	return scope + "\x1f" + key
}

func adminRoleBindingKey(userID, roleCode string, product controlplane.ProductCode) string {
	return userID + "\x1f" + roleCode + "\x1f" + string(product)
}

func sortAdminRoleRecords(items []AdminRoleRecord) {
	sort.Slice(items, func(i, j int) bool {
		if items[i].Code != items[j].Code {
			return items[i].Code < items[j].Code
		}
		return items[i].Product < items[j].Product
	})
}

func sortAdminRoleAssignments(items []controlplane.AdminRoleAssignment) {
	sort.Slice(items, func(i, j int) bool {
		if items[i].Product != items[j].Product {
			return items[i].Product < items[j].Product
		}
		if items[i].RoleCode != items[j].RoleCode {
			return items[i].RoleCode < items[j].RoleCode
		}
		return items[i].UserID < items[j].UserID
	})
}

func compatibilityLocalSuperAdmin(user controlplane.UserSummary) bool {
	return user.ID == "usr_local_admin" && user.Role == controlplane.RoleAdmin && user.Status == controlplane.UserStatusActive
}

func containsGlobalSuperAdmin(assignments []controlplane.AdminRoleAssignment) bool {
	return slices.ContainsFunc(assignments, func(assignment controlplane.AdminRoleAssignment) bool {
		return assignment.RoleCode == controlplane.BuiltinAdminRoleSuperAdmin && assignment.Product == ""
	})
}
