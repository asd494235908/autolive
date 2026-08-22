package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"sort"
	"strings"

	"autoLive/backend/internal/controlplane"
	"github.com/lib/pq"
)

var _ AdminRBACRepository = (*PostgresRepository)(nil)

var ErrNormalizedAdminRBACRepositoryRequired = errors.New("normalized admin rbac repository is required")

const (
	adminRBACListLimit                 = 200
	listAdminPermissionsQuery          = `SELECT code FROM admin_permissions ORDER BY code LIMIT $1`
	getAdminAuthorizationUserQuery     = `SELECT role, status FROM users WHERE id = $1 LIMIT 1`
	getAdminAuthorizationBindingsQuery = `
		SELECT uar.role_code, uar.product, arp.permission_code
		FROM user_admin_roles uar
		LEFT JOIN user_products up
			ON up.user_id = uar.user_id
		   AND up.product = uar.product
		LEFT JOIN admin_role_permissions arp
			ON arp.role_code = uar.role_code
		WHERE uar.user_id = $1
		  AND (
			(uar.role_code = 'super_admin' AND uar.product IS NULL) OR
			(uar.product = $2 AND up.status = 'active')
		  )
	`
	listAdminRolesQuery = `
		SELECT r.code, r.product, r.name, r.built_in, rp.permission_code
		FROM admin_roles r
		LEFT JOIN admin_role_permissions rp
			ON rp.role_code = r.code
		WHERE ($1 = '' OR r.code = 'super_admin' OR r.product = $1)
		ORDER BY r.code ASC, r.product ASC, rp.permission_code ASC
		LIMIT $2
	`
	getAdminRoleQuery = `
		SELECT r.code, r.product, r.name, r.built_in, rp.permission_code
		FROM admin_roles r
		LEFT JOIN admin_role_permissions rp
			ON rp.role_code = r.code
		WHERE r.code = $1
		ORDER BY rp.permission_code ASC
	`
	loadAdminRoleForUpdateQuery = `
		SELECT code, product, name, built_in
		FROM admin_roles
		WHERE code = $1
		FOR UPDATE
	`
	insertAdminRoleQuery = `
		INSERT INTO admin_roles (code, product, name, built_in, created_at, updated_at)
		VALUES ($1, $2, $3, FALSE, $4, $4)
	`
	replaceAdminRolePermissionsQuery = `
		INSERT INTO admin_role_permissions (role_code, permission_code, created_at)
		SELECT $1, permission_code, $3
		FROM UNNEST($2::text[]) AS permission_code
	`
	updateAdminRoleQuery             = `UPDATE admin_roles SET name = $2, updated_at = $3 WHERE code = $1`
	deleteAdminRolePermissionsQuery  = `DELETE FROM admin_role_permissions WHERE role_code = $1`
	deleteAdminRoleQuery             = `DELETE FROM admin_roles WHERE code = $1`
	loadUserAdminRolesForUpdateQuery = `SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 FOR UPDATE`
	listUserAdminRolesQuery          = `
		SELECT role_code, product
		FROM user_admin_roles
		WHERE user_id = $1
		ORDER BY COALESCE(product, ''), role_code
		LIMIT $2
	`
	listUserAdminRolesFilteredQuery = `
		SELECT role_code, product
		FROM user_admin_roles
		WHERE user_id = $1
		  AND ($2 = '' OR product IS NULL OR product = $2)
		ORDER BY COALESCE(product, ''), role_code
		LIMIT $3
	`
	deleteUserAdminRolesQuery         = `DELETE FROM user_admin_roles WHERE user_id = $1`
	loadAssignableRolesForUpdateQuery = `
		SELECT code, product, built_in
		FROM admin_roles
		WHERE code = ANY($1)
		FOR UPDATE
	`
	loadActiveMembershipsQuery = `
		SELECT product
		FROM user_products
		WHERE user_id = $1
		  AND status = 'active'
		  AND product = ANY($2)
	`
	insertScopedUserAdminRolesQuery = `
		INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
		SELECT $1, assignment.role_code, assignment.product, $4
		FROM UNNEST($2::text[], $3::text[]) AS assignment(role_code, product)
	`
	insertGlobalUserAdminRoleQuery = `
		INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
		VALUES ($1, $2, NULL, $3)
	`
	countOtherGlobalSuperAdminsQuery = `
		SELECT COUNT(*)
		FROM user_admin_roles uar
		INNER JOIN users u
			ON u.id = uar.user_id
		WHERE uar.role_code = 'super_admin'
		  AND uar.product IS NULL
		  AND u.status = 'active'
		  AND uar.user_id <> $1
	`
)

func (s *PostgresRepository) ListAdminPermissions(ctx context.Context) ([]controlplane.PermissionCode, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return nil, controlplane.ErrInvalidRequest
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()

	rows, err := s.db.QueryContext(operationCtx, listAdminPermissionsQuery, adminRBACListLimit)
	if err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	defer rows.Close()

	permissions := make([]controlplane.PermissionCode, 0)
	for rows.Next() {
		var permission controlplane.PermissionCode
		if err := rows.Scan(&permission); err != nil {
			return nil, postgresOperationError(operationCtx, err)
		}
		permissions = append(permissions, permission)
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	return permissions, nil
}

func (s *PostgresRepository) GetAdminAuthorization(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.AdminAuthorization, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.AdminAuthorization{}, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return controlplane.AdminAuthorization{}, controlplane.ErrInvalidRequest
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.AdminAuthorization{}, controlplane.ErrUserNotFound
	}
	if product != "" && !product.Valid() {
		return controlplane.AdminAuthorization{}, controlplane.ErrInvalidRequest
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()

	var userRole string
	var userStatus string
	if err := s.db.QueryRowContext(operationCtx, getAdminAuthorizationUserQuery, userID).Scan(&userRole, &userStatus); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return controlplane.AdminAuthorization{}, controlplane.ErrUserNotFound
		}
		return controlplane.AdminAuthorization{}, postgresOperationError(operationCtx, fmt.Errorf("load normalized admin authorization user: %w", err))
	}
	if userStatus == controlplane.UserStatusDisabled {
		return controlplane.AdminAuthorization{}, controlplane.ErrUserDisabled
	}

	rows, err := s.db.QueryContext(operationCtx, getAdminAuthorizationBindingsQuery, userID, product)
	if err != nil {
		return controlplane.AdminAuthorization{}, postgresOperationError(operationCtx, fmt.Errorf("load normalized admin authorization bindings: %w", err))
	}
	defer rows.Close()

	roleSet := map[string]struct{}{}
	permissionSet := map[string]struct{}{}
	globalSuperAdmin := compatibilityLocalSuperAdmin(controlplane.UserSummary{ID: userID, Role: userRole, Status: userStatus})
	for rows.Next() {
		var roleCode string
		var storedProduct sql.NullString
		var permissionCode sql.NullString
		if err := rows.Scan(&roleCode, &storedProduct, &permissionCode); err != nil {
			return controlplane.AdminAuthorization{}, postgresOperationError(operationCtx, fmt.Errorf("scan normalized admin authorization binding: %w", err))
		}
		roleSet[roleCode] = struct{}{}
		if roleCode == controlplane.BuiltinAdminRoleSuperAdmin && !storedProduct.Valid {
			globalSuperAdmin = true
			continue
		}
		if permissionCode.Valid {
			permissionSet[permissionCode.String] = struct{}{}
		}
	}
	if err := rows.Err(); err != nil {
		return controlplane.AdminAuthorization{}, postgresOperationError(operationCtx, fmt.Errorf("iterate normalized admin authorization bindings: %w", err))
	}

	if globalSuperAdmin {
		roleSet[controlplane.BuiltinAdminRoleSuperAdmin] = struct{}{}
		for _, permission := range normalizedCatalogPermissions() {
			permissionSet[string(permission)] = struct{}{}
		}
	}

	roleCodes := make([]string, 0, len(roleSet))
	for roleCode := range roleSet {
		roleCodes = append(roleCodes, roleCode)
	}
	sort.Strings(roleCodes)

	permissions := make([]controlplane.PermissionCode, 0, len(permissionSet))
	for permission := range permissionSet {
		permissions = append(permissions, controlplane.PermissionCode(permission))
	}
	sort.Slice(permissions, func(i, j int) bool { return permissions[i] < permissions[j] })

	return controlplane.AdminAuthorization{
		UserID:           userID,
		Product:          product,
		GlobalSuperAdmin: globalSuperAdmin,
		RoleCodes:        roleCodes,
		Permissions:      permissions,
	}, nil
}

func (s *PostgresRepository) ListAdminRoles(ctx context.Context, product controlplane.ProductCode) ([]AdminRoleRecord, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return nil, controlplane.ErrInvalidRequest
	}
	if product != "" && !product.Valid() {
		return nil, controlplane.ErrInvalidRequest
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()

	rows, err := s.db.QueryContext(operationCtx, listAdminRolesQuery, string(product), adminRBACListLimit)
	if err != nil {
		return nil, postgresOperationError(operationCtx, fmt.Errorf("list normalized admin roles: %w", err))
	}
	defer rows.Close()

	return collectAdminRoleRecords(operationCtx, rows)
}

func (s *PostgresRepository) GetAdminRole(ctx context.Context, code string) (AdminRoleRecord, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return AdminRoleRecord{}, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return AdminRoleRecord{}, controlplane.ErrInvalidRequest
	}
	code = strings.TrimSpace(code)
	if code == "" {
		return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()

	rows, err := s.db.QueryContext(operationCtx, getAdminRoleQuery, code)
	if err != nil {
		return AdminRoleRecord{}, postgresOperationError(operationCtx, fmt.Errorf("get normalized admin role: %w", err))
	}
	defer rows.Close()

	roles, err := collectAdminRoleRecords(operationCtx, rows)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	if len(roles) == 0 {
		return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
	}
	return roles[0], nil
}

func (s *PostgresRepository) CreateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return AdminRoleRecord{}, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return AdminRoleRecord{}, controlplane.ErrInvalidRequest
	}
	role, err := normalizeAdminRoleWriteRecord(record)
	if err != nil {
		return AdminRoleRecord{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return AdminRoleRecord{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return AdminRoleRecord{}, err
	}

	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code, s.Now())
	if err != nil {
		return AdminRoleRecord{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != role.Code {
			return AdminRoleRecord{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadAdminRoleTx(operationCtx, tx, role.Code)
	}

	now := s.Now()
	if _, err := tx.ExecContext(operationCtx, insertAdminRoleQuery, role.Code, nullableProduct(role.Product), role.Name, now); err != nil {
		return AdminRoleRecord{}, translateAdminRoleWriteError(operationCtx, fmt.Errorf("insert normalized admin role: %w", err))
	}
	if err := replaceAdminRolePermissionsTx(operationCtx, tx, role.Code, role.Permissions, now); err != nil {
		return AdminRoleRecord{}, err
	}
	if err := tx.Commit(); err != nil {
		return AdminRoleRecord{}, postgresCommitError(operationCtx, "commit normalized admin role creation", err)
	}
	return role, nil
}

func (s *PostgresRepository) UpdateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return AdminRoleRecord{}, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return AdminRoleRecord{}, controlplane.ErrInvalidRequest
	}
	role, err := normalizeAdminRoleWriteRecord(record)
	if err != nil {
		return AdminRoleRecord{}, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return AdminRoleRecord{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return AdminRoleRecord{}, err
	}

	current, err := loadAdminRoleForUpdate(operationCtx, tx, role.Code)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	if current.BuiltIn {
		return AdminRoleRecord{}, controlplane.ErrAdminBuiltInRoleImmutable
	}
	if current.Product != role.Product {
		return AdminRoleRecord{}, controlplane.ErrAdminProductScopeMismatch
	}

	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, role.Code, s.Now())
	if err != nil {
		return AdminRoleRecord{}, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != role.Code {
			return AdminRoleRecord{}, controlplane.ErrIdempotencyConflict
		}
		return s.loadAdminRoleTx(operationCtx, tx, role.Code)
	}

	now := s.Now()
	if _, err := tx.ExecContext(operationCtx, updateAdminRoleQuery, role.Code, role.Name, now); err != nil {
		return AdminRoleRecord{}, postgresOperationError(operationCtx, fmt.Errorf("update normalized admin role: %w", err))
	}
	if _, err := tx.ExecContext(operationCtx, deleteAdminRolePermissionsQuery, role.Code); err != nil {
		return AdminRoleRecord{}, postgresOperationError(operationCtx, fmt.Errorf("delete normalized admin role permissions: %w", err))
	}
	if err := replaceAdminRolePermissionsTx(operationCtx, tx, role.Code, role.Permissions, now); err != nil {
		return AdminRoleRecord{}, err
	}
	if err := tx.Commit(); err != nil {
		return AdminRoleRecord{}, postgresCommitError(operationCtx, "commit normalized admin role update", err)
	}
	return role, nil
}

func (s *PostgresRepository) DeleteAdminRole(ctx context.Context, record AdminRoleDeleteRecord) error {
	if s.modelReadSource != ModelReadSourceNormalized {
		return ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.Code = strings.TrimSpace(record.Code)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.Code == "" {
		return controlplane.ErrInvalidRequest
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return err
	}

	role, err := loadAdminRoleForUpdate(operationCtx, tx, record.Code)
	if err != nil {
		return err
	}
	if role.BuiltIn {
		return controlplane.ErrAdminBuiltInRoleImmutable
	}

	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.Code, s.Now())
	if err != nil {
		return err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != record.Code {
			return controlplane.ErrIdempotencyConflict
		}
		return nil
	}

	if _, err := tx.ExecContext(operationCtx, deleteAdminRoleQuery, record.Code); err != nil {
		return translateAdminRoleDeleteError(operationCtx, fmt.Errorf("delete normalized admin role: %w", err))
	}
	if err := tx.Commit(); err != nil {
		return postgresCommitError(operationCtx, "commit normalized admin role delete", err)
	}
	return nil
}

func (s *PostgresRepository) ListUserAdminRoles(ctx context.Context, userID string, product controlplane.ProductCode) ([]controlplane.AdminRoleAssignment, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return nil, controlplane.ErrInvalidRequest
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return nil, controlplane.ErrUserNotFound
	}
	if product != "" && !product.Valid() {
		return nil, controlplane.ErrInvalidRequest
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	user, err := s.GetUserByID(operationCtx, userID)
	if err != nil {
		return nil, err
	}
	assignments, err := listUserAdminRolesByQuery(operationCtx, s.db, listUserAdminRolesFilteredQuery, userID, string(product), adminRBACListLimit)
	if err != nil {
		return nil, err
	}
	return withCompatibilityLocalSuperAdmin(user, assignments), nil
}

func (s *PostgresRepository) ReplaceUserAdminRoles(ctx context.Context, record UserAdminRoleReplaceRecord) ([]controlplane.AdminRoleAssignment, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, ErrNormalizedAdminRBACRepositoryRequired
	}
	if ctx == nil {
		return nil, controlplane.ErrInvalidRequest
	}
	record.Scope = strings.TrimSpace(record.Scope)
	record.IdempotencyKey = strings.TrimSpace(record.IdempotencyKey)
	record.Fingerprint = strings.TrimSpace(record.Fingerprint)
	record.UserID = strings.TrimSpace(record.UserID)
	if record.Scope == "" || record.IdempotencyKey == "" || record.Fingerprint == "" || record.UserID == "" {
		return nil, controlplane.ErrInvalidRequest
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()
	if err := lockNormalizedControlPlaneMutation(operationCtx, tx); err != nil {
		return nil, err
	}

	user, err := s.loadUserForUpdate(operationCtx, tx, record.UserID)
	if err != nil {
		return nil, err
	}
	if user.Status == controlplane.UserStatusDisabled {
		return nil, controlplane.ErrUserDisabled
	}

	storedFingerprint, storedResourceID, inserted, err := s.reserveUserIdempotency(operationCtx, tx, record.Scope, record.IdempotencyKey, record.Fingerprint, record.UserID, s.Now())
	if err != nil {
		return nil, err
	}
	if !inserted {
		if storedFingerprint != record.Fingerprint || storedResourceID != record.UserID {
			return nil, controlplane.ErrIdempotencyConflict
		}
		return listUserAdminRolesByQuery(operationCtx, tx, listUserAdminRolesQuery, record.UserID, adminRBACListLimit)
	}

	normalizedAssignments, err := s.normalizeUserAdminRoleAssignmentsTx(operationCtx, tx, record.UserID, record.Assignments)
	if err != nil {
		return nil, err
	}

	currentAssignments, err := listUserAdminRolesByQuery(operationCtx, tx, loadUserAdminRolesForUpdateQuery, record.UserID)
	if err != nil {
		return nil, err
	}
	if removesGlobalSuperAdmin(currentAssignments, normalizedAssignments) {
		var otherActiveSuperAdmins int
		if err := tx.QueryRowContext(operationCtx, countOtherGlobalSuperAdminsQuery, record.UserID).Scan(&otherActiveSuperAdmins); err != nil {
			return nil, postgresOperationError(operationCtx, fmt.Errorf("count normalized global super admins: %w", err))
		}
		if otherActiveSuperAdmins == 0 {
			return nil, controlplane.ErrAdminLastSuperAdminProtected
		}
	}

	if _, err := tx.ExecContext(operationCtx, deleteUserAdminRolesQuery, record.UserID); err != nil {
		return nil, postgresOperationError(operationCtx, fmt.Errorf("delete normalized user admin roles: %w", err))
	}
	if err := insertUserAdminRolesTx(operationCtx, tx, record.UserID, normalizedAssignments, s.Now()); err != nil {
		return nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, postgresCommitError(operationCtx, "commit normalized user admin role replacement", err)
	}
	return normalizedAssignments, nil
}

func collectAdminRoleRecords(ctx context.Context, rows *sql.Rows) ([]AdminRoleRecord, error) {
	roles := make([]AdminRoleRecord, 0)
	indexByCode := make(map[string]int)
	for rows.Next() {
		var code string
		var product sql.NullString
		var name string
		var builtIn bool
		var permission sql.NullString
		if err := rows.Scan(&code, &product, &name, &builtIn, &permission); err != nil {
			return nil, postgresOperationError(ctx, fmt.Errorf("scan normalized admin role: %w", err))
		}
		idx, ok := indexByCode[code]
		if !ok {
			role := AdminRoleRecord{
				Code:    code,
				Name:    name,
				BuiltIn: builtIn,
			}
			if product.Valid {
				role.Product = controlplane.ProductCode(product.String)
			}
			roles = append(roles, role)
			idx = len(roles) - 1
			indexByCode[code] = idx
		}
		if permission.Valid {
			roles[idx].Permissions = append(roles[idx].Permissions, controlplane.PermissionCode(permission.String))
		}
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("iterate normalized admin roles: %w", err))
	}
	for i := range roles {
		sort.Slice(roles[i].Permissions, func(left, right int) bool { return roles[i].Permissions[left] < roles[i].Permissions[right] })
		roles[i].Permissions = dedupeSortedPermissions(roles[i].Permissions)
	}
	sortAdminRoleRecords(roles)
	return roles, nil
}

func dedupeSortedPermissions(values []controlplane.PermissionCode) []controlplane.PermissionCode {
	if len(values) == 0 {
		return values
	}
	result := make([]controlplane.PermissionCode, 0, len(values))
	for _, value := range values {
		if len(result) == 0 || result[len(result)-1] != value {
			result = append(result, value)
		}
	}
	return result
}

func loadAdminRoleForUpdate(ctx context.Context, tx *sql.Tx, code string) (AdminRoleRecord, error) {
	var role AdminRoleRecord
	var product sql.NullString
	if err := tx.QueryRowContext(ctx, loadAdminRoleForUpdateQuery, code).Scan(&role.Code, &product, &role.Name, &role.BuiltIn); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
		}
		return AdminRoleRecord{}, postgresOperationError(ctx, fmt.Errorf("load normalized admin role for update: %w", err))
	}
	if product.Valid {
		role.Product = controlplane.ProductCode(product.String)
	}
	return role, nil
}

func (s *PostgresRepository) loadAdminRoleTx(ctx context.Context, tx *sql.Tx, code string) (AdminRoleRecord, error) {
	rows, err := tx.QueryContext(ctx, getAdminRoleQuery, code)
	if err != nil {
		return AdminRoleRecord{}, postgresOperationError(ctx, fmt.Errorf("load normalized admin role replay: %w", err))
	}
	defer rows.Close()

	roles, err := collectAdminRoleRecords(ctx, rows)
	if err != nil {
		return AdminRoleRecord{}, err
	}
	if len(roles) == 0 {
		return AdminRoleRecord{}, controlplane.ErrAdminRoleNotFound
	}
	return roles[0], nil
}

func replaceAdminRolePermissionsTx(ctx context.Context, tx *sql.Tx, code string, permissions []controlplane.PermissionCode, now interface{}) error {
	if len(permissions) == 0 {
		return nil
	}
	raw := make([]string, 0, len(permissions))
	for _, permission := range permissions {
		raw = append(raw, string(permission))
	}
	if _, err := tx.ExecContext(ctx, replaceAdminRolePermissionsQuery, code, pq.Array(raw), now); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("replace normalized admin role permissions: %w", err))
	}
	return nil
}

func nullableProduct(product controlplane.ProductCode) interface{} {
	if product == "" {
		return nil
	}
	return product
}

func translateAdminRoleWriteError(ctx context.Context, err error) error {
	var pqErr *pq.Error
	if errors.As(err, &pqErr) {
		switch pqErr.Code {
		case "23505":
			return controlplane.ErrInvalidRequest
		case "23503":
			if pqErr.Constraint == "admin_roles_product_fkey" {
				return controlplane.ErrAdminProductScopeMismatch
			}
		}
	}
	return postgresOperationError(ctx, err)
}

func translateAdminRoleDeleteError(ctx context.Context, err error) error {
	var pqErr *pq.Error
	if errors.As(err, &pqErr) && pqErr.Code == "23503" && pqErr.Constraint == "user_admin_roles_role_fkey" {
		return controlplane.ErrAdminRoleAssigned
	}
	return postgresOperationError(ctx, err)
}

func listUserAdminRolesByQuery(ctx context.Context, queryer interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
}, query string, userID string, args ...any) ([]controlplane.AdminRoleAssignment, error) {
	queryArgs := make([]any, 0, len(args)+1)
	queryArgs = append(queryArgs, userID)
	queryArgs = append(queryArgs, args...)
	rows, err := queryer.QueryContext(ctx, query, queryArgs...)
	if err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("list normalized user admin roles: %w", err))
	}
	defer rows.Close()

	assignments := make([]controlplane.AdminRoleAssignment, 0)
	for rows.Next() {
		var roleCode string
		var product sql.NullString
		if err := rows.Scan(&roleCode, &product); err != nil {
			return nil, postgresOperationError(ctx, fmt.Errorf("scan normalized user admin role: %w", err))
		}
		assignment := controlplane.AdminRoleAssignment{UserID: userID, RoleCode: roleCode}
		if product.Valid {
			assignment.Product = controlplane.ProductCode(product.String)
		}
		assignments = append(assignments, assignment)
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("iterate normalized user admin roles: %w", err))
	}
	sortAdminRoleAssignments(assignments)
	return assignments, nil
}

func withCompatibilityLocalSuperAdmin(user controlplane.UserSummary, assignments []controlplane.AdminRoleAssignment) []controlplane.AdminRoleAssignment {
	if !compatibilityLocalSuperAdmin(user) {
		return assignments
	}
	global := controlplane.AdminRoleAssignment{
		UserID:   user.ID,
		RoleCode: controlplane.BuiltinAdminRoleSuperAdmin,
	}
	for _, assignment := range assignments {
		if assignment.RoleCode == global.RoleCode && assignment.Product == "" {
			return assignments
		}
	}
	result := append(append([]controlplane.AdminRoleAssignment(nil), assignments...), global)
	sortAdminRoleAssignments(result)
	return result
}

func (s *PostgresRepository) normalizeUserAdminRoleAssignmentsTx(ctx context.Context, tx *sql.Tx, userID string, assignments []controlplane.AdminRoleAssignment) ([]controlplane.AdminRoleAssignment, error) {
	roleCodes := make([]string, 0)
	roleCodeSet := map[string]struct{}{}
	productSet := map[string]controlplane.ProductCode{}
	normalized := make([]controlplane.AdminRoleAssignment, 0, len(assignments))
	seen := map[string]struct{}{}

	for _, assignment := range assignments {
		current := assignment
		current.UserID = strings.TrimSpace(current.UserID)
		current.RoleCode = strings.TrimSpace(current.RoleCode)
		if current.UserID == "" {
			current.UserID = userID
		}
		if current.UserID != userID || current.RoleCode == "" {
			return nil, controlplane.ErrInvalidRequest
		}
		key := adminRoleBindingKey(current.UserID, current.RoleCode, current.Product)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		normalized = append(normalized, controlplane.AdminRoleAssignment{
			UserID:   current.UserID,
			RoleCode: current.RoleCode,
			Product:  current.Product,
		})
		if _, ok := roleCodeSet[current.RoleCode]; !ok {
			roleCodeSet[current.RoleCode] = struct{}{}
			roleCodes = append(roleCodes, current.RoleCode)
		}
		if current.Product != "" {
			productSet[string(current.Product)] = current.Product
		}
	}

	roleDetails, err := loadAssignableRoles(ctx, tx, roleCodes)
	if err != nil {
		return nil, err
	}
	activeMemberships, err := loadActiveMembershipProducts(ctx, tx, userID, productSet)
	if err != nil {
		return nil, err
	}

	for i := range normalized {
		detail, ok := roleDetails[normalized[i].RoleCode]
		if !ok {
			return nil, controlplane.ErrAdminRoleNotFound
		}
		if normalized[i].RoleCode == controlplane.BuiltinAdminRoleSuperAdmin {
			if normalized[i].Product != "" {
				return nil, controlplane.ErrAdminProductScopeMismatch
			}
			continue
		}
		if !normalized[i].Product.Valid() || detail.Product != normalized[i].Product {
			return nil, controlplane.ErrAdminProductScopeMismatch
		}
		if _, ok := activeMemberships[normalized[i].Product]; !ok {
			return nil, controlplane.ErrAdminProductScopeMismatch
		}
	}
	sortAdminRoleAssignments(normalized)
	return normalized, nil
}

type assignableRoleDetail struct {
	Product controlplane.ProductCode
	BuiltIn bool
}

func loadAssignableRoles(ctx context.Context, tx *sql.Tx, roleCodes []string) (map[string]assignableRoleDetail, error) {
	if len(roleCodes) == 0 {
		return map[string]assignableRoleDetail{}, nil
	}
	sort.Strings(roleCodes)
	rows, err := tx.QueryContext(ctx, loadAssignableRolesForUpdateQuery, pq.Array(roleCodes))
	if err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("load normalized assignable admin roles: %w", err))
	}
	defer rows.Close()

	details := make(map[string]assignableRoleDetail, len(roleCodes))
	for rows.Next() {
		var code string
		var product sql.NullString
		var builtIn bool
		if err := rows.Scan(&code, &product, &builtIn); err != nil {
			return nil, postgresOperationError(ctx, fmt.Errorf("scan normalized assignable admin role: %w", err))
		}
		detail := assignableRoleDetail{BuiltIn: builtIn}
		if product.Valid {
			detail.Product = controlplane.ProductCode(product.String)
		}
		details[code] = detail
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("iterate normalized assignable admin roles: %w", err))
	}
	return details, nil
}

func loadActiveMembershipProducts(ctx context.Context, tx *sql.Tx, userID string, products map[string]controlplane.ProductCode) (map[controlplane.ProductCode]struct{}, error) {
	if len(products) == 0 {
		return map[controlplane.ProductCode]struct{}{}, nil
	}
	values := make([]string, 0, len(products))
	for _, product := range products {
		values = append(values, string(product))
	}
	sort.Strings(values)

	rows, err := tx.QueryContext(ctx, loadActiveMembershipsQuery, userID, pq.Array(values))
	if err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("load normalized active memberships: %w", err))
	}
	defer rows.Close()

	memberships := make(map[controlplane.ProductCode]struct{}, len(values))
	for rows.Next() {
		var product controlplane.ProductCode
		if err := rows.Scan(&product); err != nil {
			return nil, postgresOperationError(ctx, fmt.Errorf("scan normalized active membership: %w", err))
		}
		memberships[product] = struct{}{}
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(ctx, fmt.Errorf("iterate normalized active memberships: %w", err))
	}
	return memberships, nil
}

func removesGlobalSuperAdmin(current, next []controlplane.AdminRoleAssignment) bool {
	currentHas := false
	for _, assignment := range current {
		if assignment.RoleCode == controlplane.BuiltinAdminRoleSuperAdmin && assignment.Product == "" {
			currentHas = true
			break
		}
	}
	if !currentHas {
		return false
	}
	for _, assignment := range next {
		if assignment.RoleCode == controlplane.BuiltinAdminRoleSuperAdmin && assignment.Product == "" {
			return false
		}
	}
	return true
}

func insertUserAdminRolesTx(ctx context.Context, tx *sql.Tx, userID string, assignments []controlplane.AdminRoleAssignment, now interface{}) error {
	if len(assignments) == 0 {
		return nil
	}
	scopedRoleCodes := make([]string, 0, len(assignments))
	scopedProducts := make([]string, 0, len(assignments))
	for _, assignment := range assignments {
		if assignment.Product == "" {
			if _, err := tx.ExecContext(ctx, insertGlobalUserAdminRoleQuery, userID, assignment.RoleCode, now); err != nil {
				return postgresOperationError(ctx, fmt.Errorf("insert normalized global user admin role: %w", err))
			}
			continue
		}
		scopedRoleCodes = append(scopedRoleCodes, assignment.RoleCode)
		scopedProducts = append(scopedProducts, string(assignment.Product))
	}
	if len(scopedRoleCodes) == 0 {
		return nil
	}
	if _, err := tx.ExecContext(ctx, insertScopedUserAdminRolesQuery, userID, pq.Array(scopedRoleCodes), pq.Array(scopedProducts), now); err != nil {
		return postgresOperationError(ctx, fmt.Errorf("insert normalized scoped user admin roles: %w", err))
	}
	return nil
}
