package service

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestAdminRBACGetAdminAuthorizationUsesPermissionUnionAndFailClosedCompatibility(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_scoped", Username: "scoped", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_global", Username: "global", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_admin_flag", Username: "flag", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_local_admin", Username: "local", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_scoped", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_scoped", Product: controlplane.ProductDouyinDesktop, Status: "active"})

	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "devices_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Device Reader",
		Permissions: []controlplane.PermissionCode{"devices.read"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "devices_manager_douyin",
		Product:     controlplane.ProductDouyinDesktop,
		Name:        "Douyin Device Manager",
		Permissions: []controlplane.PermissionCode{"devices.manage"},
	})

	seedAdminRBACServiceAssignments(t, repository, "usr_scoped",
		controlplane.AdminRoleAssignment{UserID: "usr_scoped", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
		controlplane.AdminRoleAssignment{UserID: "usr_scoped", RoleCode: "devices_reader_auto", Product: controlplane.ProductAutoLive},
		controlplane.AdminRoleAssignment{UserID: "usr_scoped", RoleCode: "devices_manager_douyin", Product: controlplane.ProductDouyinDesktop},
	)
	seedAdminRBACServiceAssignments(t, repository, "usr_global",
		controlplane.AdminRoleAssignment{UserID: "usr_global", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)

	authAuto, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_scoped", Product: controlplane.ProductAutoLive})
	if err != nil {
		t.Fatalf("GetAdminAuthorization(scoped autolive) error = %v", err)
	}
	if authAuto.GlobalSuperAdmin {
		t.Fatalf("GetAdminAuthorization(scoped autolive) unexpectedly global: %+v", authAuto)
	}
	if got := authAuto.RoleCodes; !slices.Equal(got, []string{"devices_reader_auto", "users_reader_auto"}) {
		t.Fatalf("GetAdminAuthorization(scoped autolive) role codes = %#v", got)
	}
	if got := authAuto.Permissions; !slices.Equal(got, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("GetAdminAuthorization(scoped autolive) permissions = %#v", got)
	}

	authDouyin, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_scoped", Product: controlplane.ProductDouyinDesktop})
	if err != nil {
		t.Fatalf("GetAdminAuthorization(scoped douyin) error = %v", err)
	}
	if authDouyin.GlobalSuperAdmin || !slices.Equal(authDouyin.Permissions, []controlplane.PermissionCode{"devices.manage"}) {
		t.Fatalf("GetAdminAuthorization(scoped douyin) = %+v", authDouyin)
	}

	authGlobal, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_global", Product: controlplane.ProductDouyinDesktop})
	if err != nil {
		t.Fatalf("GetAdminAuthorization(global) error = %v", err)
	}
	if !authGlobal.GlobalSuperAdmin || len(authGlobal.Permissions) != len(controlplane.PermissionCatalog()) {
		t.Fatalf("GetAdminAuthorization(global) = %+v", authGlobal)
	}

	authLocalAdmin, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_local_admin", Product: controlplane.ProductDouyinDesktop})
	if err != nil {
		t.Fatalf("GetAdminAuthorization(local admin compatibility) error = %v", err)
	}
	if !authLocalAdmin.GlobalSuperAdmin || !slices.Contains(authLocalAdmin.RoleCodes, controlplane.BuiltinAdminRoleSuperAdmin) {
		t.Fatalf("GetAdminAuthorization(local admin compatibility) = %+v", authLocalAdmin)
	}

	authRoleAdmin, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_admin_flag", Product: controlplane.ProductAutoLive})
	if err != nil {
		t.Fatalf("GetAdminAuthorization(non-local role admin) error = %v", err)
	}
	if authRoleAdmin.GlobalSuperAdmin || len(authRoleAdmin.Permissions) != 0 {
		t.Fatalf("GetAdminAuthorization(non-local role admin) = %+v", authRoleAdmin)
	}
}

func TestAdminRBACGetAdminAuthorizationDisablesImmediately(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_disabled", Username: "disabled", Role: controlplane.RoleUser, Status: controlplane.UserStatusDisabled, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_disabled", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.UserAdminRoles["usr_disabled\x1fusers_reader_auto\x1fautolive"] = controlplane.AdminRoleAssignment{
			UserID:   "usr_disabled",
			RoleCode: "users_reader_auto",
			Product:  controlplane.ProductAutoLive,
		}
		return nil
	}); err != nil {
		t.Fatalf("seed disabled assignments: %v", err)
	}

	_, err := svc.GetAdminAuthorization(context.Background(), controlplane.Actor{UserID: "usr_disabled", Product: controlplane.ProductAutoLive})
	if !errors.Is(err, controlplane.ErrUserDisabled) {
		t.Fatalf("GetAdminAuthorization(disabled) error = %v, want user disabled", err)
	}
}

func TestAdminRBACCreateAdminRoleEnforcesProductScopeAndPermissionSubset(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_operator", Username: "operator", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_operator", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "roles_manager_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive RBAC Manager",
		Permissions: []controlplane.PermissionCode{"roles.manage", "roles.read", "roles.assign", "users.read"},
	})
	seedAdminRBACServiceAssignments(t, repository, "usr_operator",
		controlplane.AdminRoleAssignment{UserID: "usr_operator", RoleCode: "roles_manager_auto", Product: controlplane.ProductAutoLive},
	)

	_, err := svc.CreateAdminRole(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "create-role-cross", AdminRoleSpec{
		Code:        "cross_product_role",
		Product:     controlplane.ProductDouyinDesktop,
		Name:        "Cross Product",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	if !errors.Is(err, controlplane.ErrAdminProductScopeMismatch) {
		t.Fatalf("CreateAdminRole(cross product) error = %v, want product scope mismatch", err)
	}

	_, err = svc.CreateAdminRole(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "create-role-overgrant", AdminRoleSpec{
		Code:        "too_powerful_role",
		Product:     controlplane.ProductAutoLive,
		Name:        "Too Powerful",
		Permissions: []controlplane.PermissionCode{"devices.manage"},
	})
	if !errors.Is(err, controlplane.ErrAdminRoleDelegationForbidden) {
		t.Fatalf("CreateAdminRole(overgrant) error = %v, want delegation forbidden", err)
	}

	_, err = svc.CreateAdminRole(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "create-role-global", AdminRoleSpec{
		Code:        "global_like_role",
		Name:        "Global Like",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	if !errors.Is(err, controlplane.ErrAdminProductScopeMismatch) {
		t.Fatalf("CreateAdminRole(global scoped ordinary role) error = %v, want product scope mismatch", err)
	}
}

func TestAdminRBACUpdateAndDeleteAdminRoleProtectsBuiltInAndBoundRoles(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_global", Username: "global", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_target", Username: "target", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceAssignments(t, repository, "usr_global",
		controlplane.AdminRoleAssignment{UserID: "usr_global", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "bound_auto_role",
		Product:     controlplane.ProductAutoLive,
		Name:        "Bound Role",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_target", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceAssignments(t, repository, "usr_target",
		controlplane.AdminRoleAssignment{UserID: "usr_target", RoleCode: "bound_auto_role", Product: controlplane.ProductAutoLive},
	)

	_, err := svc.UpdateAdminRole(context.Background(), controlplane.Actor{UserID: "usr_global", Product: controlplane.ProductAutoLive}, "update-super-admin", controlplane.BuiltinAdminRoleSuperAdmin, AdminRoleSpec{
		Code:        controlplane.BuiltinAdminRoleSuperAdmin,
		Name:        "Nope",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	if !errors.Is(err, controlplane.ErrAdminBuiltInRoleImmutable) {
		t.Fatalf("UpdateAdminRole(super_admin) error = %v, want builtin immutable", err)
	}

	err = svc.DeleteAdminRole(context.Background(), controlplane.Actor{UserID: "usr_global", Product: controlplane.ProductAutoLive}, "delete-bound-role", "bound_auto_role")
	if !errors.Is(err, controlplane.ErrAdminRoleAssigned) {
		t.Fatalf("DeleteAdminRole(bound role) error = %v, want role assigned", err)
	}
}

func TestAdminRBACReplaceUserAdminRolesPreservesOtherProductsAndChecksDelegationSubset(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_operator", Username: "operator", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_target", Username: "target", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_operator", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_target", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_target", Product: controlplane.ProductDouyinDesktop, Status: "active"})

	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "rbac_manager_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "RBAC Manager",
		Permissions: []controlplane.PermissionCode{"roles.assign", "roles.read", "users.read"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "Users Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "devices_manager_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "Devices Manager",
		Permissions: []controlplane.PermissionCode{"devices.manage"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_douyin",
		Product:     controlplane.ProductDouyinDesktop,
		Name:        "Douyin Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	seedAdminRBACServiceAssignments(t, repository, "usr_operator",
		controlplane.AdminRoleAssignment{UserID: "usr_operator", RoleCode: "rbac_manager_auto", Product: controlplane.ProductAutoLive},
	)
	seedAdminRBACServiceAssignments(t, repository, "usr_target",
		controlplane.AdminRoleAssignment{UserID: "usr_target", RoleCode: "users_reader_douyin", Product: controlplane.ProductDouyinDesktop},
	)

	assignments, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "replace-target-autolive", "usr_target", controlplane.ProductAutoLive, []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
	})
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(scoped preserve) error = %v", err)
	}
	want := []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
		{UserID: "usr_target", RoleCode: "users_reader_douyin", Product: controlplane.ProductDouyinDesktop},
	}
	if !slices.Equal(assignments, want) {
		t.Fatalf("ReplaceUserAdminRoles(scoped preserve) assignments = %#v, want %#v", assignments, want)
	}

	_, err = svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "replace-target-overgrant", "usr_target", controlplane.ProductAutoLive, []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: "devices_manager_auto", Product: controlplane.ProductAutoLive},
	})
	if !errors.Is(err, controlplane.ErrAdminRoleDelegationForbidden) {
		t.Fatalf("ReplaceUserAdminRoles(overgrant assignment) error = %v, want delegation forbidden", err)
	}
}

func TestAdminRBACReplaceUserAdminRolesRejectsGlobalAssignmentsFromScopedAdmin(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_operator", Username: "operator", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_target", Username: "target", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_operator", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "rbac_manager_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "RBAC Manager",
		Permissions: []controlplane.PermissionCode{"roles.assign", "roles.read", "users.read"},
	})
	seedAdminRBACServiceAssignments(t, repository, "usr_operator",
		controlplane.AdminRoleAssignment{UserID: "usr_operator", RoleCode: "rbac_manager_auto", Product: controlplane.ProductAutoLive},
	)

	_, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_operator", Product: controlplane.ProductAutoLive}, "replace-target-global", "usr_target", "", []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	})
	if !errors.Is(err, controlplane.ErrAdminRoleDelegationForbidden) {
		t.Fatalf("ReplaceUserAdminRoles(global assignment from scoped admin) error = %v, want delegation forbidden", err)
	}
}

func TestAdminRBACReplaceUserAdminRolesProtectsLastSuperAdminAndLocalAdminCompatibility(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_last_super", Username: "last", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_local_admin", Username: "local", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceAssignments(t, repository, "usr_last_super",
		controlplane.AdminRoleAssignment{UserID: "usr_last_super", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_local_admin", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "Users Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})

	if err := repository.Run(context.Background(), func(state *store.State) error {
		delete(state.Users, "usr_local_admin")
		return nil
	}); err != nil {
		t.Fatalf("delete local admin fixture: %v", err)
	}

	_, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_last_super", Product: controlplane.ProductAutoLive}, "remove-last-super", "usr_last_super", "", nil)
	if !errors.Is(err, controlplane.ErrAdminLastSuperAdminProtected) {
		t.Fatalf("ReplaceUserAdminRoles(remove last super admin) error = %v, want last super admin protected", err)
	}

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_local_admin", Username: "local", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	assignments, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_last_super", Product: controlplane.ProductAutoLive}, "local-admin-compat", "usr_local_admin", controlplane.ProductAutoLive, []controlplane.AdminRoleAssignment{
		{UserID: "usr_local_admin", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
	})
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(local admin compatibility) error = %v", err)
	}
	if !slices.Contains(assignments, controlplane.AdminRoleAssignment{UserID: "usr_local_admin", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin}) {
		t.Fatalf("ReplaceUserAdminRoles(local admin compatibility) assignments = %#v, want compatibility super admin", assignments)
	}
}

func TestAdminRBACReplaceUserAdminRolesIsIdempotent(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_global_operator", Username: "operator", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_target", Username: "target", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceAssignments(t, repository, "usr_global_operator",
		controlplane.AdminRoleAssignment{UserID: "usr_global_operator", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_target", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "Users Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "devices_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "Devices Reader",
		Permissions: []controlplane.PermissionCode{"devices.read"},
	})

	record := []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
	}
	assignments, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_global_operator", Product: controlplane.ProductAutoLive}, "same-key", "usr_target", controlplane.ProductAutoLive, record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(first) error = %v", err)
	}

	replayed, err := svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_global_operator", Product: controlplane.ProductAutoLive}, "same-key", "usr_target", controlplane.ProductAutoLive, record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(replay) error = %v", err)
	}
	if !slices.Equal(replayed, assignments) {
		t.Fatalf("ReplaceUserAdminRoles(replay) assignments = %#v, want %#v", replayed, assignments)
	}

	_, err = svc.ReplaceUserAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_global_operator", Product: controlplane.ProductAutoLive}, "same-key", "usr_target", controlplane.ProductAutoLive, []controlplane.AdminRoleAssignment{
		{UserID: "usr_target", RoleCode: "devices_reader_auto", Product: controlplane.ProductAutoLive},
	})
	if !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("ReplaceUserAdminRoles(conflict) error = %v, want idempotency conflict", err)
	}
}

func TestAdminRBACListAdminRolesScopedAdminHidesGlobalSuperAdmin(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_scoped_reader", Username: "reader", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_global_reader", Username: "global", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceMembership(t, repository, controlplane.UserProductMembership{UserID: "usr_scoped_reader", Product: controlplane.ProductAutoLive, Status: "active"})
	seedAdminRBACServiceRole(t, repository, store.AdminRoleRecord{
		Code:        "roles_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Role Reader",
		Permissions: []controlplane.PermissionCode{"roles.read"},
	})
	seedAdminRBACServiceAssignments(t, repository, "usr_scoped_reader",
		controlplane.AdminRoleAssignment{UserID: "usr_scoped_reader", RoleCode: "roles_reader_auto", Product: controlplane.ProductAutoLive},
	)
	seedAdminRBACServiceAssignments(t, repository, "usr_global_reader",
		controlplane.AdminRoleAssignment{UserID: "usr_global_reader", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)

	roles, err := svc.ListAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_scoped_reader", Product: controlplane.ProductAutoLive}, "")
	if err != nil {
		t.Fatalf("ListAdminRoles(scoped) error = %v", err)
	}
	if len(roles) != 1 || roles[0].Code != "roles_reader_auto" {
		t.Fatalf("ListAdminRoles(scoped) = %#v, want only scoped ordinary role", roles)
	}

	globalRoles, err := svc.ListAdminRoles(context.Background(), controlplane.Actor{UserID: "usr_global_reader", Product: controlplane.ProductAutoLive}, "")
	if err != nil {
		t.Fatalf("ListAdminRoles(global) error = %v", err)
	}
	if len(globalRoles) != 2 || !slices.Equal([]string{globalRoles[0].Code, globalRoles[1].Code}, []string{"roles_reader_auto", controlplane.BuiltinAdminRoleSuperAdmin}) {
		t.Fatalf("ListAdminRoles(global) = %#v", globalRoles)
	}
}

func TestAdminRBACCreateAdminRoleWithAuditRedactsRequestBody(t *testing.T) {
	repository := newAdminRBACServiceMemoryStore()
	svc := NewControlPlaneWithRepository(repository)

	seedAdminRBACServiceUser(t, repository, controlplane.UserSummary{ID: "usr_global", Username: "global", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACServiceAssignments(t, repository, "usr_global",
		controlplane.AdminRoleAssignment{UserID: "usr_global", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
	)

	_, err := svc.CreateAdminRoleWithAudit(context.Background(), controlplane.Actor{UserID: "usr_global", Product: controlplane.ProductAutoLive}, "audit-create-role", AdminRoleSpec{
		Code:        "audit_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "body-should-not-appear",
		Permissions: []controlplane.PermissionCode{"users.read"},
	}, controlplane.AuditLogInput{
		ActorUserID: "usr_global",
		Action:      "POST /api/v1/admin/roles",
		TargetType:  "admin_role",
		RequestID:   "req-create-role",
	})
	if err != nil {
		t.Fatalf("CreateAdminRoleWithAudit(success) error = %v", err)
	}

	_, err = svc.CreateAdminRoleWithAudit(context.Background(), controlplane.Actor{UserID: "usr_global", Product: controlplane.ProductAutoLive}, "audit-create-role-failure", AdminRoleSpec{
		Code:        "audit_reader_auto_fail",
		Product:     controlplane.ProductAutoLive,
		Name:        "another-secret-body",
		Permissions: []controlplane.PermissionCode{"unknown.permission"},
	}, controlplane.AuditLogInput{
		ActorUserID: "usr_global",
		Action:      "POST /api/v1/admin/roles",
		TargetType:  "admin_role",
		RequestID:   "req-create-role-fail",
	})
	if !errors.Is(err, controlplane.ErrAdminPermissionUnknown) {
		t.Fatalf("CreateAdminRoleWithAudit(failure) error = %v, want unknown permission", err)
	}

	logs := listAdminRBACAuditLogs(t, repository)
	if len(logs) < 2 {
		t.Fatalf("audit logs = %#v, want at least 2 entries", logs)
	}
	successAudit := logs[1]
	failureAudit := logs[0]
	if successAudit.TargetID != "audit_reader_auto" || successAudit.TargetType != "admin_role" || successAudit.Outcome != "success" || successAudit.ErrorCode != "" {
		t.Fatalf("success audit = %+v", successAudit)
	}
	if failureAudit.TargetID != "audit_reader_auto_fail" || failureAudit.Outcome != "failure" || failureAudit.ErrorCode != controlplane.ErrAdminPermissionUnknown.Code {
		t.Fatalf("failure audit = %+v", failureAudit)
	}

	raw, err := json.Marshal(logs)
	if err != nil {
		t.Fatalf("marshal audit logs: %v", err)
	}
	text := string(raw)
	if strings.Contains(text, "body-should-not-appear") || strings.Contains(text, "another-secret-body") || strings.Contains(text, "unknown.permission") {
		t.Fatalf("audit logs leaked request body or permission list: %s", text)
	}
}

func newAdminRBACServiceMemoryStore() *store.MemoryStore {
	return store.NewMemoryStore(func() time.Time {
		return time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	})
}

func seedAdminRBACServiceUser(t *testing.T, repository *store.MemoryStore, user controlplane.UserSummary) {
	t.Helper()
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.Users[user.ID] = user
		return nil
	}); err != nil {
		t.Fatalf("seed user %s: %v", user.ID, err)
	}
}

func seedAdminRBACServiceMembership(t *testing.T, repository *store.MemoryStore, membership controlplane.UserProductMembership) {
	t.Helper()
	if err := repository.Run(context.Background(), func(state *store.State) error {
		state.UserProducts[membership.UserID+"::"+string(membership.Product)] = membership
		return nil
	}); err != nil {
		t.Fatalf("seed membership %s/%s: %v", membership.UserID, membership.Product, err)
	}
}

func seedAdminRBACServiceRole(t *testing.T, repository *store.MemoryStore, role store.AdminRoleRecord) {
	t.Helper()
	_, err := repository.CreateAdminRole(context.Background(), store.AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "seed-role:" + role.Code,
		Fingerprint:    "seed-role:" + role.Code,
		Role:           role,
	})
	if err != nil {
		t.Fatalf("seed role %s: %v", role.Code, err)
	}
}

func seedAdminRBACServiceAssignments(t *testing.T, repository *store.MemoryStore, userID string, assignments ...controlplane.AdminRoleAssignment) {
	t.Helper()
	_, err := repository.ReplaceUserAdminRoles(context.Background(), store.UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "seed-assignments:" + userID,
		Fingerprint:    "seed-assignments:" + userID,
		UserID:         userID,
		Assignments:    assignments,
	})
	if err != nil {
		t.Fatalf("seed assignments for %s: %v", userID, err)
	}
}

func listAdminRBACAuditLogs(t *testing.T, repository *store.MemoryStore) []controlplane.AuditLog {
	t.Helper()
	var logs []controlplane.AuditLog
	if err := repository.Run(context.Background(), func(state *store.State) error {
		for _, item := range state.AuditLogs {
			logs = append(logs, item)
		}
		return nil
	}); err != nil {
		t.Fatalf("list audit logs: %v", err)
	}
	slices.SortFunc(logs, func(a, b controlplane.AuditLog) int {
		if a.CreatedAt != b.CreatedAt {
			return strings.Compare(b.CreatedAt, a.CreatedAt)
		}
		return strings.Compare(b.ID, a.ID)
	})
	return logs
}
