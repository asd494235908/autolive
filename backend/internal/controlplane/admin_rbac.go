package controlplane

import "sort"

type PermissionCode = string

const (
	BuiltinAdminRoleSuperAdmin = "super_admin"
)

var adminPermissionCatalog = []PermissionCode{
	"dashboard.read",
	"users.read",
	"users.manage",
	"roles.read",
	"roles.manage",
	"roles.assign",
	"admin_security.manage",
	"devices.read",
	"devices.manage",
	"activation_codes.read",
	"activation_codes.manage",
	"activation_codes.reveal",
	"activation_codes.switch_device",
	"plans.read",
	"plans.manage",
	"plans.publish",
	"orders.read",
	"orders.reconcile",
	"subscriptions.read",
	"subscriptions.adjust",
	"payments.read",
	"payments.reconcile",
	"password_resets.read",
	"password_resets.retry",
	"artifacts.read",
	"artifacts.manage",
	"artifacts.publish",
	"artifacts.revoke",
	"public_config.read",
	"public_config.manage",
	"public_config.publish",
	"public_config.rollback",
	"error_reports.read",
	"error_reports.manage",
	"feedback.read",
	"feedback.manage",
	"model_pool.read",
	"model_pool.manage",
	"model_pool.test",
	"model_pool.rotate_secret",
	"model_leases.read",
	"model_leases.reclaim",
	"model_usage.read",
	"audit_logs.read",
	"operations.read",
	"operations.manage",
}

var adminPermissionCatalogSet = buildPermissionCatalogSet(adminPermissionCatalog)

func buildPermissionCatalogSet(catalog []PermissionCode) map[PermissionCode]struct{} {
	set := make(map[PermissionCode]struct{}, len(catalog))
	for _, permission := range catalog {
		set[permission] = struct{}{}
	}
	return set
}

func PermissionCatalog() []PermissionCode {
	return append([]PermissionCode(nil), adminPermissionCatalog...)
}

type AdminRole struct {
	Code        string           `json:"code"`
	Name        string           `json:"name"`
	BuiltIn     bool             `json:"built_in"`
	Permissions []PermissionCode `json:"permissions"`
}

type AdminRoleAssignment struct {
	UserID   string      `json:"user_id"`
	RoleCode string      `json:"role_code"`
	Product  ProductCode `json:"product"`
}

type AdminAuthorization struct {
	UserID           string           `json:"user_id"`
	Product          ProductCode      `json:"product"`
	GlobalSuperAdmin bool             `json:"global_super_admin"`
	RoleCodes        []string         `json:"role_codes"`
	Permissions      []PermissionCode `json:"permissions"`
}

func NormalizePermissionCodes(values []string) ([]string, error) {
	if len(values) == 0 {
		return []string{}, nil
	}

	unique := make(map[string]struct{}, len(values))
	for _, value := range values {
		permission := PermissionCode(value)
		if _, ok := adminPermissionCatalogSet[permission]; !ok {
			return nil, ErrAdminPermissionUnknown
		}
		unique[value] = struct{}{}
	}

	normalized := make([]string, 0, len(unique))
	for value := range unique {
		normalized = append(normalized, value)
	}
	sort.Strings(normalized)
	return normalized, nil
}
