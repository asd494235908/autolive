package controlplane

import (
	"slices"
	"testing"
)

func TestPermissionCatalog(t *testing.T) {
	catalog := PermissionCatalog()
	if len(catalog) == 0 {
		t.Fatal("permission catalog is empty")
	}

	seen := make(map[PermissionCode]struct{}, len(catalog))
	required := map[PermissionCode]struct{}{
		"users.read":               {},
		"roles.manage":             {},
		"activation_codes.reveal":  {},
		"model_pool.rotate_secret": {},
		"operations.manage":        {},
	}

	for _, permission := range catalog {
		if _, ok := seen[permission]; ok {
			t.Fatalf("duplicate permission %q", permission)
		}
		seen[permission] = struct{}{}
		delete(required, permission)
	}

	if len(required) != 0 {
		t.Fatalf("missing required permissions: %#v", required)
	}

	catalog[0] = "mutated.permission"
	fresh := PermissionCatalog()
	if fresh[0] == "mutated.permission" {
		t.Fatal("PermissionCatalog should return a defensive copy")
	}
}

func TestNormalizePermissionCodes(t *testing.T) {
	got, err := NormalizePermissionCodes([]string{"users.manage", "users.read", "users.manage"})
	if err != nil || !slices.Equal(got, []string{"users.manage", "users.read"}) {
		t.Fatalf("got %#v, err %v", got, err)
	}

	if _, err := NormalizePermissionCodes([]string{"unknown.permission"}); !IsErrorCode(err, "ADMIN_PERMISSION_UNKNOWN") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestAdminAuthorizationAllowsGlobalSuperAdminScope(t *testing.T) {
	auth := AdminAuthorization{
		UserID:           "usr_local_admin",
		GlobalSuperAdmin: true,
		RoleCodes:        []string{BuiltinAdminRoleSuperAdmin},
		Permissions:      PermissionCatalog(),
	}

	if auth.Product != "" {
		t.Fatalf("expected empty product scope for global super admin, got %q", auth.Product)
	}
	if !auth.GlobalSuperAdmin {
		t.Fatal("expected global super admin flag to be true")
	}
	if !slices.Contains(auth.RoleCodes, BuiltinAdminRoleSuperAdmin) {
		t.Fatalf("expected %q role in %#v", BuiltinAdminRoleSuperAdmin, auth.RoleCodes)
	}
}

func TestAdminRBACStableErrors(t *testing.T) {
	tests := []struct {
		name string
		err  error
		code string
	}{
		{name: "permission denied", err: ErrAdminPermissionDenied, code: "ADMIN_PERMISSION_DENIED"},
		{name: "role not found", err: ErrAdminRoleNotFound, code: "ADMIN_ROLE_NOT_FOUND"},
		{name: "built-in role immutable", err: ErrAdminBuiltInRoleImmutable, code: "ADMIN_BUILTIN_ROLE_IMMUTABLE"},
		{name: "role still assigned", err: ErrAdminRoleAssigned, code: "ADMIN_ROLE_ASSIGNED"},
		{name: "delegation forbidden", err: ErrAdminRoleDelegationForbidden, code: "ADMIN_ROLE_DELEGATION_FORBIDDEN"},
		{name: "last super admin protected", err: ErrAdminLastSuperAdminProtected, code: "ADMIN_LAST_SUPER_ADMIN_PROTECTED"},
		{name: "product scope mismatch", err: ErrAdminProductScopeMismatch, code: "ADMIN_PRODUCT_SCOPE_MISMATCH"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !IsErrorCode(tt.err, tt.code) {
				t.Fatalf("expected code %s, got %v", tt.code, tt.err)
			}
		})
	}
}
