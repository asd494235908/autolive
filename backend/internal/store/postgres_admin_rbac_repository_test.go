package store

import (
	"context"
	"errors"
	"regexp"
	"slices"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
	"github.com/lib/pq"
)

func TestPostgresRepositoryGetAdminAuthorizationUsesScopedBindingsAndGlobalSuperAdmin(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectQuery(regexp.QuoteMeta("SELECT role, status FROM users WHERE id = $1 LIMIT 1")).
		WithArgs("usr_scoped").
		WillReturnRows(sqlmock.NewRows([]string{"role", "status"}).AddRow(controlplane.RoleUser, controlplane.UserStatusActive))
	mock.ExpectQuery(regexp.QuoteMeta(`
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
	`)).
		WithArgs("usr_scoped", controlplane.ProductAutoLive).
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product", "permission_code"}).
			AddRow("users_reader_auto", controlplane.ProductAutoLive, "users.read").
			AddRow("super_admin", nil, nil).
			AddRow("devices_reader_auto", controlplane.ProductAutoLive, "devices.read").
			AddRow("users_reader_auto", controlplane.ProductAutoLive, "users.read"))

	authorization, err := repository.GetAdminAuthorization(context.Background(), "usr_scoped", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetAdminAuthorization() error = %v", err)
	}
	if !authorization.GlobalSuperAdmin {
		t.Fatalf("authorization.GlobalSuperAdmin = false, want true")
	}
	if !slices.Equal(authorization.RoleCodes, []string{"devices_reader_auto", "super_admin", "users_reader_auto"}) {
		t.Fatalf("authorization.RoleCodes = %#v", authorization.RoleCodes)
	}
	if !slices.Equal(authorization.Permissions, controlplane.PermissionCatalog()) {
		t.Fatalf("authorization.Permissions = %#v, want full catalog", authorization.Permissions)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetAdminAuthorizationReturnsEmptyWhenUserHasNoBindings(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectQuery(regexp.QuoteMeta("SELECT role, status FROM users WHERE id = $1 LIMIT 1")).
		WithArgs("usr_empty").
		WillReturnRows(sqlmock.NewRows([]string{"role", "status"}).AddRow(controlplane.RoleUser, controlplane.UserStatusActive))
	mock.ExpectQuery(regexp.QuoteMeta(`
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
	`)).
		WithArgs("usr_empty", controlplane.ProductAutoLive).
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product", "permission_code"}))

	authorization, err := repository.GetAdminAuthorization(context.Background(), "usr_empty", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetAdminAuthorization() error = %v", err)
	}
	if authorization.GlobalSuperAdmin || len(authorization.RoleCodes) != 0 || len(authorization.Permissions) != 0 {
		t.Fatalf("authorization = %+v, want empty authorization", authorization)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryGetAdminAuthorizationDoesNotTreatDatabaseErrorAsSuccess(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, time.Now, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectQuery(regexp.QuoteMeta("SELECT role, status FROM users WHERE id = $1 LIMIT 1")).
		WithArgs("usr_broken").
		WillReturnRows(sqlmock.NewRows([]string{"role", "status"}).AddRow(controlplane.RoleUser, controlplane.UserStatusActive))
	mock.ExpectQuery(regexp.QuoteMeta(`
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
	`)).
		WithArgs("usr_broken", controlplane.ProductAutoLive).
		WillReturnError(errors.New("driver read failure"))

	_, err = repository.GetAdminAuthorization(context.Background(), "usr_broken", controlplane.ProductAutoLive)
	if err == nil {
		t.Fatalf("GetAdminAuthorization() error = nil, want failure")
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryAdminRoleCRUDUsesNormalizedTransactions(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "create-admin-role:auto", "fp-create", "ops_autolive", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-create", "ops_autolive"))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO admin_roles (code, product, name, built_in, created_at, updated_at)
		VALUES ($1, $2, $3, FALSE, $4, $4)
	`)).
		WithArgs("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", now).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO admin_role_permissions (role_code, permission_code, created_at)
		SELECT $1, permission_code, $3
		FROM UNNEST($2::text[]) AS permission_code
	`)).
		WithArgs("ops_autolive", pq.Array([]string{"devices.read", "users.read"}), now).
		WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectCommit()

	created, err := repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "create-admin-role:auto",
		Fingerprint:    "fp-create",
		Role: AdminRoleRecord{
			Code:        "ops_autolive",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Ops",
			Permissions: []controlplane.PermissionCode{"users.read", "devices.read", "users.read"},
		},
	})
	if err != nil {
		t.Fatalf("CreateAdminRole() error = %v", err)
	}
	if created.Code != "ops_autolive" || !slices.Equal(created.Permissions, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("created role = %+v", created)
	}

	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT r.code, r.product, r.name, r.built_in, rp.permission_code
		FROM admin_roles r
		LEFT JOIN admin_role_permissions rp
			ON rp.role_code = r.code
		WHERE ($1 = '' OR r.code = 'super_admin' OR r.product = $1)
		ORDER BY r.code ASC, r.product ASC, rp.permission_code ASC
	`)).
		WithArgs(controlplane.ProductAutoLive).
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "name", "built_in", "permission_code"}).
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", false, "users.read").
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", false, "devices.read").
			AddRow("super_admin", nil, "超级管理员", true, "users.manage"))

	roles, err := repository.ListAdminRoles(context.Background(), controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("ListAdminRoles() error = %v", err)
	}
	if got := adminRoleCodes(roles); !slices.Equal(got, []string{"ops_autolive", "super_admin"}) {
		t.Fatalf("role codes = %#v", got)
	}
	if !slices.Equal(roles[0].Permissions, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("role permissions = %#v", roles[0].Permissions)
	}

	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT r.code, r.product, r.name, r.built_in, rp.permission_code
		FROM admin_roles r
		LEFT JOIN admin_role_permissions rp
			ON rp.role_code = r.code
		WHERE r.code = $1
		ORDER BY rp.permission_code ASC
	`)).
		WithArgs("ops_autolive").
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "name", "built_in", "permission_code"}).
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", false, "users.read").
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", false, "devices.read"))

	role, err := repository.GetAdminRole(context.Background(), "ops_autolive")
	if err != nil {
		t.Fatalf("GetAdminRole() error = %v", err)
	}
	if role.Name != "AutoLive Ops" || !slices.Equal(role.Permissions, []controlplane.PermissionCode{"devices.read", "users.read"}) {
		t.Fatalf("loaded role = %+v", role)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, name, built_in
		FROM admin_roles
		WHERE code = $1
		FOR UPDATE
	`)).
		WithArgs("ops_autolive").
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "name", "built_in"}).
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Ops", false))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "update-admin-role:auto", "fp-update", "ops_autolive", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-update", "ops_autolive"))
	mock.ExpectExec(regexp.QuoteMeta("UPDATE admin_roles SET name = $2, updated_at = $3 WHERE code = $1")).
		WithArgs("ops_autolive", "AutoLive Operators", now).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM admin_role_permissions WHERE role_code = $1")).
		WithArgs("ops_autolive").
		WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO admin_role_permissions (role_code, permission_code, created_at)
		SELECT $1, permission_code, $3
		FROM UNNEST($2::text[]) AS permission_code
	`)).
		WithArgs("ops_autolive", pq.Array([]string{"devices.manage", "users.read"}), now).
		WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectCommit()

	updated, err := repository.UpdateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "update-admin-role:auto",
		Fingerprint:    "fp-update",
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

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, name, built_in
		FROM admin_roles
		WHERE code = $1
		FOR UPDATE
	`)).
		WithArgs("ops_autolive").
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "name", "built_in"}).
			AddRow("ops_autolive", controlplane.ProductAutoLive, "AutoLive Operators", false))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "delete-admin-role:auto", "fp-delete", "ops_autolive", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-delete", "ops_autolive"))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM admin_roles WHERE code = $1")).
		WithArgs("ops_autolive").
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()

	if err := repository.DeleteAdminRole(context.Background(), AdminRoleDeleteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "delete-admin-role:auto",
		Fingerprint:    "fp-delete",
		Code:           "ops_autolive",
	}); err != nil {
		t.Fatalf("DeleteAdminRole() error = %v", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryAdminRoleWriteErrorsTranslateToDomainErrors(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "create-admin-role:duplicate", "fp-duplicate", "ops_autolive", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-duplicate", "ops_autolive"))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO admin_roles (code, product, name, built_in, created_at, updated_at)
		VALUES ($1, $2, $3, FALSE, $4, $4)
	`)).
		WithArgs("ops_autolive", controlplane.ProductAutoLive, "Duplicate", now).
		WillReturnError(&pq.Error{Code: "23505", Constraint: "admin_roles_pkey"})
	mock.ExpectRollback()

	_, err = repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "create-admin-role:duplicate",
		Fingerprint:    "fp-duplicate",
		Role: AdminRoleRecord{
			Code:        "ops_autolive",
			Product:     controlplane.ProductAutoLive,
			Name:        "Duplicate",
			Permissions: []controlplane.PermissionCode{"users.read"},
		},
	})
	if !errors.Is(err, controlplane.ErrInvalidRequest) {
		t.Fatalf("CreateAdminRole(duplicate) error = %v, want invalid request", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "create-admin-role:bad-product", "fp-bad-product", "ops_invalid", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-bad-product", "ops_invalid"))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO admin_roles (code, product, name, built_in, created_at, updated_at)
		VALUES ($1, $2, $3, FALSE, $4, $4)
	`)).
		WithArgs("ops_invalid", controlplane.ProductDouyinDesktop, "Bad Product", now).
		WillReturnError(&pq.Error{Code: "23503", Constraint: "admin_roles_product_fkey"})
	mock.ExpectRollback()

	_, err = repository.CreateAdminRole(context.Background(), AdminRoleWriteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "create-admin-role:bad-product",
		Fingerprint:    "fp-bad-product",
		Role: AdminRoleRecord{
			Code:        "ops_invalid",
			Product:     controlplane.ProductDouyinDesktop,
			Name:        "Bad Product",
			Permissions: []controlplane.PermissionCode{"users.read"},
		},
	})
	if !errors.Is(err, controlplane.ErrAdminProductScopeMismatch) {
		t.Fatalf("CreateAdminRole(product fk) error = %v, want product scope mismatch", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, name, built_in
		FROM admin_roles
		WHERE code = $1
		FOR UPDATE
	`)).
		WithArgs("ops_bound").
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "name", "built_in"}).
			AddRow("ops_bound", controlplane.ProductAutoLive, "Bound Role", false))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "delete-admin-role:bound", "fp-bound", "ops_bound", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-bound", "ops_bound"))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM admin_roles WHERE code = $1")).
		WithArgs("ops_bound").
		WillReturnError(&pq.Error{Code: "23503", Constraint: "user_admin_roles_role_fkey"})
	mock.ExpectRollback()

	err = repository.DeleteAdminRole(context.Background(), AdminRoleDeleteRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "delete-admin-role:bound",
		Fingerprint:    "fp-bound",
		Code:           "ops_bound",
	})
	if !errors.Is(err, controlplane.ErrAdminRoleAssigned) {
		t.Fatalf("DeleteAdminRole(bound) error = %v, want role assigned", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryReplaceUserAdminRolesIsIdempotentAndProtectsLastSuperAdmin(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

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

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_replace", "replace", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_replace", "fp-replace", "usr_replace", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-replace", "usr_replace"))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, built_in
		FROM admin_roles
		WHERE code = ANY($1)
		FOR UPDATE
	`)).
		WithArgs(pq.Array([]string{"role_a", "role_b"})).
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "built_in"}).
			AddRow("role_a", controlplane.ProductAutoLive, false).
			AddRow("role_b", controlplane.ProductAutoLive, false))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT product
		FROM user_products
		WHERE user_id = $1
		  AND status = 'active'
		  AND product = ANY($2)
	`)).
		WithArgs("usr_replace", pq.Array([]string{string(controlplane.ProductAutoLive)})).
		WillReturnRows(sqlmock.NewRows([]string{"product"}).AddRow(controlplane.ProductAutoLive))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 FOR UPDATE")).
		WithArgs("usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product"}).
			AddRow("role_b", controlplane.ProductAutoLive))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM user_admin_roles WHERE user_id = $1")).
		WithArgs("usr_replace").
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
		SELECT $1, assignment.role_code, assignment.product, $4
		FROM UNNEST($2::text[], $3::text[]) AS assignment(role_code, product)
	`)).
		WithArgs("usr_replace", pq.Array([]string{"role_a", "role_b"}), pq.Array([]string{string(controlplane.ProductAutoLive), string(controlplane.ProductAutoLive)}), now).
		WillReturnResult(sqlmock.NewResult(1, 2))
	mock.ExpectCommit()

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

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_replace", "replace", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_replace", "fp-replace", "usr_replace", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2 FOR UPDATE")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-replace", "usr_replace"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 ORDER BY COALESCE(product, ''), role_code")).
		WithArgs("usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product"}).
			AddRow("role_a", controlplane.ProductAutoLive).
			AddRow("role_b", controlplane.ProductAutoLive))
	mock.ExpectRollback()

	replayed, err := repository.ReplaceUserAdminRoles(context.Background(), record)
	if err != nil {
		t.Fatalf("ReplaceUserAdminRoles(replay) error = %v", err)
	}
	if !slices.Equal(replayed, want) {
		t.Fatalf("replayed assignments = %#v, want %#v", replayed, want)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_replace", "replace", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_replace", "fp-conflict", "usr_replace", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT fingerprint, resource_id FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2 FOR UPDATE")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_replace").
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-replace", "usr_replace"))
	mock.ExpectRollback()

	_, err = repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          record.Scope,
		IdempotencyKey: record.IdempotencyKey,
		Fingerprint:    "fp-conflict",
		UserID:         record.UserID,
		Assignments:    record.Assignments,
	})
	if !errors.Is(err, controlplane.ErrIdempotencyConflict) {
		t.Fatalf("ReplaceUserAdminRoles(conflict) error = %v, want idempotency conflict", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_local_admin").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_local_admin", "local", controlplane.RoleAdmin, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_local_admin", "fp-last-super", "usr_local_admin", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-last-super", "usr_local_admin"))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 FOR UPDATE")).
		WithArgs("usr_local_admin").
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product"}).
			AddRow(controlplane.BuiltinAdminRoleSuperAdmin, nil))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT COUNT(*)
		FROM user_admin_roles uar
		INNER JOIN users u
			ON u.id = uar.user_id
		WHERE uar.role_code = 'super_admin'
		  AND uar.product IS NULL
		  AND u.status = 'active'
		  AND uar.user_id <> $1
	`)).
		WithArgs("usr_local_admin").
		WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(0))
	mock.ExpectRollback()

	_, err = repository.ReplaceUserAdminRoles(context.Background(), UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_local_admin",
		Fingerprint:    "fp-last-super",
		UserID:         "usr_local_admin",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_local_admin", RoleCode: "role_a", Product: controlplane.ProductAutoLive},
		},
	})
	if !errors.Is(err, controlplane.ErrAdminLastSuperAdminProtected) {
		t.Fatalf("ReplaceUserAdminRoles(last super admin) error = %v, want last super admin protected", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryReplaceUserAdminRolesFailedAttemptDoesNotConsumeIdempotencyAndClassifiesCommitUnknown(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	record := UserAdminRoleReplaceRecord{
		Scope:          "control-plane-state",
		IdempotencyKey: "replace-user-admin-roles:usr_retry",
		Fingerprint:    "fp-retry",
		UserID:         "usr_retry",
		Assignments: []controlplane.AdminRoleAssignment{
			{UserID: "usr_retry", RoleCode: "retry_role", Product: controlplane.ProductDouyinDesktop},
		},
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_retry").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_retry", "retry", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_retry", "fp-retry", "usr_retry", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-retry", "usr_retry"))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, built_in
		FROM admin_roles
		WHERE code = ANY($1)
		FOR UPDATE
	`)).
		WithArgs(pq.Array([]string{"retry_role"})).
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "built_in"}).
			AddRow("retry_role", controlplane.ProductDouyinDesktop, false))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT product
		FROM user_products
		WHERE user_id = $1
		  AND status = 'active'
		  AND product = ANY($2)
	`)).
		WithArgs("usr_retry", pq.Array([]string{string(controlplane.ProductDouyinDesktop)})).
		WillReturnRows(sqlmock.NewRows([]string{"product"}))
	mock.ExpectRollback()

	_, err = repository.ReplaceUserAdminRoles(context.Background(), record)
	if !errors.Is(err, controlplane.ErrAdminProductScopeMismatch) {
		t.Fatalf("ReplaceUserAdminRoles(first attempt) error = %v, want product scope mismatch", err)
	}

	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("SELECT pg_advisory_xact_lock")).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at FROM users WHERE id = $1 FOR UPDATE")).
		WithArgs("usr_retry").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_retry", "retry", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("INSERT INTO idempotency_records (scope, idempotency_key, fingerprint, resource_id, created_at)")).
		WithArgs("control-plane-state", "replace-user-admin-roles:usr_retry", "fp-retry", "usr_retry", now).
		WillReturnRows(sqlmock.NewRows([]string{"fingerprint", "resource_id"}).AddRow("fp-retry", "usr_retry"))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT code, product, built_in
		FROM admin_roles
		WHERE code = ANY($1)
		FOR UPDATE
	`)).
		WithArgs(pq.Array([]string{"retry_role"})).
		WillReturnRows(sqlmock.NewRows([]string{"code", "product", "built_in"}).
			AddRow("retry_role", controlplane.ProductDouyinDesktop, false))
	mock.ExpectQuery(regexp.QuoteMeta(`
		SELECT product
		FROM user_products
		WHERE user_id = $1
		  AND status = 'active'
		  AND product = ANY($2)
	`)).
		WithArgs("usr_retry", pq.Array([]string{string(controlplane.ProductDouyinDesktop)})).
		WillReturnRows(sqlmock.NewRows([]string{"product"}).AddRow(controlplane.ProductDouyinDesktop))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 FOR UPDATE")).
		WithArgs("usr_retry").
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product"}))
	mock.ExpectExec(regexp.QuoteMeta("DELETE FROM user_admin_roles WHERE user_id = $1")).
		WithArgs("usr_retry").
		WillReturnResult(sqlmock.NewResult(1, 0))
	mock.ExpectExec(regexp.QuoteMeta(`
		INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
		SELECT $1, assignment.role_code, assignment.product, $4
		FROM UNNEST($2::text[], $3::text[]) AS assignment(role_code, product)
	`)).
		WithArgs("usr_retry", pq.Array([]string{"retry_role"}), pq.Array([]string{string(controlplane.ProductDouyinDesktop)}), now).
		WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit().WillReturnError(errors.New("connection lost after COMMIT"))

	_, err = repository.ReplaceUserAdminRoles(context.Background(), record)
	if !errors.Is(err, ErrCommitOutcomeUnknown) {
		t.Fatalf("ReplaceUserAdminRoles(commit unknown) error = %v, want commit outcome unknown", err)
	}

	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresRepositoryListUserAdminRolesFiltersByProductAndKeepsGlobalSuperAdmin(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, username, role, status, created_at\n\t\tFROM users\n\t\tWHERE id = $1\n\t\tLIMIT 1")).
		WithArgs("usr_list").
		WillReturnRows(sqlmock.NewRows([]string{"id", "username", "role", "status", "created_at"}).
			AddRow("usr_list", "list", controlplane.RoleUser, controlplane.UserStatusActive, now))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT role_code, product FROM user_admin_roles WHERE user_id = $1 AND ($2 = '' OR product IS NULL OR product = $2) ORDER BY COALESCE(product, ''), role_code")).
		WithArgs("usr_list", string(controlplane.ProductAutoLive)).
		WillReturnRows(sqlmock.NewRows([]string{"role_code", "product"}).
			AddRow(controlplane.BuiltinAdminRoleSuperAdmin, nil).
			AddRow("role_a", controlplane.ProductAutoLive))

	assignments, err := repository.ListUserAdminRoles(context.Background(), "usr_list", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("ListUserAdminRoles() error = %v", err)
	}
	want := []controlplane.AdminRoleAssignment{
		{UserID: "usr_list", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin},
		{UserID: "usr_list", RoleCode: "role_a", Product: controlplane.ProductAutoLive},
	}
	if !slices.Equal(assignments, want) {
		t.Fatalf("assignments = %#v, want %#v", assignments, want)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
