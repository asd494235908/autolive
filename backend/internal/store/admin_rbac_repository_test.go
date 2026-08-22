package store

import (
	"context"
	"errors"
	"slices"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestMemoryStoreAdminRBACRoleCRUDAndListFiltering(t *testing.T) {
	repository := newAdminRBACMemoryStore(t)

	permissions, err := repository.ListAdminPermissions(context.Background())
	if err != nil {
		t.Fatalf("ListAdminPermissions() error = %v", err)
	}
	if len(permissions) == 0 || permissions[0] == "" {
		t.Fatalf("permissions = %#v", permissions)
	}

	createdAuto, err := repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "create-admin-role:auto",
		Fingerprint:    "fp-auto-create",
		Role: AdminRoleRecord{
			Code:        "ops_autolive",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Ops",
			Permissions: []controlplane.PermissionCode{"devices.read", "users.read", "devices.read"},
		},
	})
	if err != nil {
		t.Fatalf("CreateAdminRole(autolive) error = %v", err)
	}
	if createdAuto.Product != controlplane.ProductAutoLive || !slices.Equal(createdAuto.Permissions, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("created autolive role = %+v", createdAuto)
	}

	createdDouyin, err := repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "create-admin-role:douyin",
		Fingerprint:    "fp-douyin-create",
		Role: AdminRoleRecord{
			Code:        "ops_douyin",
			Product:     controlplane.ProductDouyinDesktop,
			Name:        "Douyin Ops",
			Permissions: []controlplane.PermissionCode{"devices.manage"},
		},
	})
	if err != nil {
		t.Fatalf("CreateAdminRole(douyin) error = %v", err)
	}
	if createdDouyin.Product != controlplane.ProductDouyinDesktop {
		t.Fatalf("created douyin role = %+v", createdDouyin)
	}

	roles, err := repository.ListAdminRoles(context.Background(), controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("ListAdminRoles() error = %v", err)
	}
	if got := adminRoleCodes(roles); !slices.Equal(got, []string{"ops_autolive", controlplane.BuiltinAdminRoleSuperAdmin}) {
		t.Fatalf("autolive roles = %#v", got)
	}

	role, err := repository.GetAdminRole(context.Background(), "ops_autolive")
	if err != nil {
		t.Fatalf("GetAdminRole() error = %v", err)
	}
	if role.Product != controlplane.ProductAutoLive || role.Name != "AutoLive Ops" {
		t.Fatalf("loaded role = %+v", role)
	}

	updated, err := repository.UpdateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "update-admin-role:auto",
		Fingerprint:    "fp-auto-update",
		Role: AdminRoleRecord{
			Code:        "ops_autolive",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Operators",
			Permissions: []controlplane.PermissionCode{"devices.manage", "users.read"},
		},
	})
	if err != nil {
		t.Fatalf("UpdateAdminRole() error = %v", err)
	}
	if updated.Name != "AutoLive Operators" || !slices.Equal(updated.Permissions, []controlplane.PermissionCode{"devices.manage", "users.read"}) {
		t.Fatalf("updated role = %+v", updated)
	}

	if err := repository.DeleteAdminRole(context.Background(), AdminRoleDeleteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "delete-admin-role:douyin",
		Fingerprint:    "fp-douyin-delete",
		Code:           "ops_douyin",
	}); err != nil {
		t.Fatalf("DeleteAdminRole() error = %v", err)
	}
	if _, err := repository.GetAdminRole(context.Background(), "ops_douyin"); !errors.Is(err, controlplane.ErrAdminRoleNotFound) {
		t.Fatalf("GetAdminRole(deleted) error = %v, want role not found", err)
	}
}

func TestMemoryStoreAdminRBACAuthorizationUsesPermissionUnionAndProductFilter(t *testing.T) {
	repository := newAdminRBACMemoryStore(t)
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_scoped", Username: "scoped", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_global", Username: "global", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_local_admin", Username: "local", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	seedAdminRBACMembership(t, repository, controlplane.UserProductMembership{
		UserID:              "usr_scoped",
		Product:             controlplane.ProductAutoLive,
		Status:              "active",
		EntitlementRevision: 0,
		CreatedAt:           "2026-08-22T00:00:00Z",
		UpdatedAt:           "2026-08-22T00:00:00Z",
	})
	seedAdminRBACMembership(t, repository, controlplane.UserProductMembership{
		UserID:              "usr_scoped",
		Product:             controlplane.ProductDouyinDesktop,
		Status:              "active",
		EntitlementRevision: 0,
		CreatedAt:           "2026-08-22T00:00:00Z",
		UpdatedAt:           "2026-08-22T00:00:00Z",
	})

	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "users_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Reader",
		Permissions: []controlplane.PermissionCode{"users.read"},
	})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "devices_reader_auto",
		Product:     controlplane.ProductAutoLive,
		Name:        "AutoLive Device Reader",
		Permissions: []controlplane.PermissionCode{"devices.read"},
	})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "devices_manage_douyin",
		Product:     controlplane.ProductDouyinDesktop,
		Name:        "Douyin Device Manager",
		Permissions: []controlplane.PermissionCode{"devices.manage"},
	})

	if _, err := repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_scoped",
		Fingerprint:    "fp-scoped-replace",
		UserID:         "usr_scoped",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_scoped", RoleCode: "devices_manage_douyin", Product: controlplane.ProductDouyinDesktop},
			{UserID: "usr_scoped", RoleCode: "devices_reader_auto", Product: controlplane.ProductAutoLive},
			{UserID: "usr_scoped", RoleCode: "users_reader_auto", Product: controlplane.ProductAutoLive},
		},
	}); err != nil {
		t.Fatalf("ReplaceUserAdminRoles(scoped) error = %v", err)
	}
	if _, err := repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_global",
		Fingerprint:    "fp-global-replace",
		UserID:         "usr_global",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_global", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
		},
	}); err != nil {
		t.Fatalf("ReplaceUserAdminRoles(global) error = %v", err)
	}

	authAuto, err := repository.GetAdminAuthorization(context.Background(), "usr_scoped", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetAdminAuthorization(autolive) error = %v", err)
	}
	if authAuto.GlobalSuperAdmin {
		t.Fatalf("autolive auth unexpectedly global: %+v", authAuto)
	}
	if got := authAuto.Permissions; !slices.Equal(got, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("autolive permissions = %#v", got)
	}

	authDouyin, err := repository.GetAdminAuthorization(context.Background(), "usr_scoped", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("GetAdminAuthorization(douyin) error = %v", err)
	}
	if got := authDouyin.Permissions; !slices.Equal(got, []controlplane.PermissionCode{"devices.manage"}) {
		t.Fatalf("douyin permissions = %#v", got)
	}

	authGlobal, err := repository.GetAdminAuthorization(context.Background(), "usr_global", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("GetAdminAuthorization(global) error = %v", err)
	}
	if !authGlobal.GlobalSuperAdmin || len(authGlobal.Permissions) != len(controlplane.PermissionCatalog()) {
		t.Fatalf("global auth = %+v", authGlobal)
	}

	authLocalAdmin, err := repository.GetAdminAuthorization(context.Background(), "usr_local_admin", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetAdminAuthorization(local admin compatibility) error = %v", err)
	}
	if !authLocalAdmin.GlobalSuperAdmin || !slices.Contains(authLocalAdmin.RoleCodes, controlplane.BuiltinAdminRoleSuperAdmin) {
		t.Fatalf("local admin compatibility auth = %+v", authLocalAdmin)
	}
}

func TestMemoryStoreDeleteAdminRoleRejectsAssignedRole(t *testing.T) {
	repository := newAdminRBACMemoryStore(t)
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_delete_guard", Username: "guard", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "delete_guard",
		Product:     controlplane.ProductAutoLive,
		Name:        "Delete Guard",
		Permissions: []controlplane.PermissionCode{"roles.read"},
	})
	if _, err := repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_delete_guard",
		Fingerprint:    "fp-delete-guard",
		UserID:         "usr_delete_guard",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_delete_guard", RoleCode: "delete_guard", Product: controlplane.ProductAutoLive},
		},
	}); err != nil {
		t.Fatalf("ReplaceUserAdminRoles() error = %v", err)
	}

	err := repository.DeleteAdminRole(context.Background(), AdminRoleDeleteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "delete-admin-role:guard",
		Fingerprint:    "fp-delete-guard-role",
		Code:           "delete_guard",
	})
	if !errors.Is(err, controlplane.ErrAdminRoleAssigned) {
		t.Fatalf("DeleteAdminRole() error = %v, want role assigned", err)
	}
}

func TestMemoryStoreReplaceUserAdminRolesDeduplicatesSortsAndChecksIdempotency(t *testing.T) {
	repository := newAdminRBACMemoryStore(t)
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_replace", Username: "replace", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "role_b",
		Product:     controlplane.ProductAutoLive,
		Name:        "Role B",
		Permissions: []controlplane.PermissionCode{"roles.read"},
	})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "role_a",
		Product:     controlplane.ProductAutoLive,
		Name:        "Role A",
		Permissions: []controlplane.PermissionCode{"roles.manage"},
	})

	record := UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_replace",
		Fingerprint:    "fp-replace",
		UserID:         "usr_replace",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_replace", RoleCode: "role_b", Product: controlplane.ProductAutoLive},
			{UserID: "usr_replace", RoleCode: "role_a", Product: controlplane.ProductAutoLive},
			{UserID: "usr_replace", RoleCode: "role_b", Product: controlplane.ProductAutoLive},
		},
	}
	assignments, err := repository.ReplaceUserAdminRoles(context.Background(), record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles() error = %v", err)
	}
	want := []controlplane.AdminRoleAssignment{
		{UserID: "usr_replace", RoleCode: "role_a", Product: controlplane.ProductAutoLive},
		{UserID: "usr_replace", RoleCode: "role_b", Product: controlplane.ProductAutoLive},
	}
	if !slices.Equal(assignments, want) {
		t.Fatalf("assignments = %#v, want %#v", assignments, want)
	}

	listed, err := repository.ListUserAdminRoles(context.Background(), "usr_replace", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("ListUserAdminRoles() error = %v", err)
	}
	if !slices.Equal(listed, want) {
		t.Fatalf("listed assignments = %#v, want %#v", listed, want)
	}

	replayed, err := repository.ReplaceUserAdminRoles(context.Background(), record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(replay) error = %v", err)
	}
	if !slices.Equal(replayed, want) {
		t.Fatalf("replayed assignments = %#v, want %#v", replayed, want)
	}

	_, err = repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          record.Scope,
		IdempotencyKey: record.IdempotencyKey,
		Fingerprint:    "fp-replace-conflict",
		UserID:         record.UserID,
		Assignments:    record.Assignments,
	})
	if !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("ReplaceUserAdminRoles(conflict) error = %v, want idempotency conflict", err)
	}
}

func TestMemoryStoreReplaceUserAdminRolesFailedAttemptDoesNotConsumeIdempotency(t *testing.T) {
	repository := newAdminRBACMemoryStore(t)
	seedAdminRBACUser(t, repository, controlplane.UserSummary{ID: "usr_retry", Username: "retry", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-22T00:00:00Z"})
	createRoleForTest(t, repository, AdminRoleRecord{
		Code:        "retry_role",
		Product:     controlplane.ProductDouyinDesktop,
		Name:        "Retry Role",
		Permissions: []controlplane.PermissionCode{"roles.read"},
	})

	record := UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_retry",
		Fingerprint:    "fp-retry",
		UserID:         "usr_retry",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_retry", RoleCode: "retry_role", Product: controlplane.ProductDouyinDesktop},
		},
	}

	if _, err := repository.ReplaceUserAdminRoles(context.Background(), record); !errors.Is(err, controlplane.ErrAdminProductScopeMismatch) {
		t.Fatalf("ReplaceUserAdminRoles(first attempt) error = %v, want product scope mismatch", err)
	}

	seedAdminRBACMembership(t, repository, controlplane.UserProductMembership{
		UserID:              "usr_retry",
		Product:             controlplane.ProductDouyinDesktop,
		Status:              "active",
		EntitlementRevision: 0,
		CreatedAt:           "2026-08-22T00:00:00Z",
		UpdatedAt:           "2026-08-22T00:00:00Z",
	})

	assignments, err := repository.ReplaceUserAdminRoles(context.Background(), record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(retry) error = %v", err)
	}

	want := []controlplane.AdminRoleAssignment{
		{UserID: "usr_retry", RoleCode: "retry_role", Product: controlplane.ProductDouyinDesktop},
	}
	if !slices.Equal(assignments, want) {
		t.Fatalf("retry assignments = %#v, want %#v", assignments, want)
	}
}

func newAdminRBACMemoryStore(t *testing.T) *MemoryStore {
	t.Helper()
	return NewMemoryStore(func() time.Time { return time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC) })
}

func seedAdminRBACUser(t *testing.T, repository *MemoryStore, user controlplane.UserSummary) {
	t.Helper()
	if err := repository.Run(context.Background(), func(state *State) error {
		state.Users[user.ID] = user
		return nil
	}); err != nil {
		t.Fatalf("seed user %s: %v", user.ID, err)
	}
}

func seedAdminRBACMembership(t *testing.T, repository *MemoryStore, membership controlplane.UserProductMembership) {
	t.Helper()
	if err := repository.Run(context.Background(), func(state *State) error {
		state.UserProducts[membership.UserID+"::"+string(membership.Product)] = membership
		return nil
	}); err != nil {
		t.Fatalf("seed membership %s/%s: %v", membership.UserID, membership.Product, err)
	}
}

func createRoleForTest(t *testing.T, repository *MemoryStore, role AdminRoleRecord) {
	t.Helper()
	_, err := repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "seed-admin-role:" + role.Code,
		Fingerprint:    "seed:" + role.Code,
		Role:           role,
	})
	if err != nil {
		t.Fatalf("seed role %s: %v", role.Code, err)
	}
}

func adminRoleCodes(roles []AdminRoleRecord) []string {
	codes := make([]string, 0, len(roles))
	for _, role := range roles {
		codes = append(codes, role.Code)
	}
	return codes
}
