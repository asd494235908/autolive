package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
	"golang.org/x/crypto/bcrypt"
)

func TestAdminRBACContractDeclaresRoutesAndWriteGuards(t *testing.T) {
	document := loadOpenAPIContract(t)
	required := []struct {
		path   string
		method string
	}{
		{path: "/api/v1/admin/me", method: "get"},
		{path: "/api/v1/admin/permissions", method: "get"},
		{path: "/api/v1/admin/roles", method: "get"},
		{path: "/api/v1/admin/roles", method: "post"},
		{path: "/api/v1/admin/roles/{role_id}", method: "get"},
		{path: "/api/v1/admin/roles/{role_id}", method: "patch"},
		{path: "/api/v1/admin/roles/{role_id}", method: "delete"},
		{path: "/api/v1/admin/users/{user_id}/roles", method: "get"},
		{path: "/api/v1/admin/users/{user_id}/roles", method: "put"},
	}

	for _, test := range required {
		operation, ok := document.Paths[test.path][test.method]
		if !ok {
			t.Fatalf("%s %s is missing from OpenAPI", test.method, test.path)
		}
		var contract struct {
			Security   []map[string][]string `yaml:"security"`
			Parameters []struct {
				Ref  string `yaml:"$ref"`
				Name string `yaml:"name"`
			} `yaml:"parameters"`
			Responses map[string]any `yaml:"responses"`
		}
		if err := operation.Decode(&contract); err != nil {
			t.Fatalf("decode %s %s: %v", test.method, test.path, err)
		}
		if len(contract.Security) == 0 {
			t.Fatalf("%s %s must declare bearer security", test.method, test.path)
		}
		if _, ok := contract.Responses["401"]; !ok {
			t.Fatalf("%s %s must declare 401", test.method, test.path)
		}
		if test.path != "/api/v1/admin/me" {
			if _, ok := contract.Responses["403"]; !ok {
				t.Fatalf("%s %s must declare 403", test.method, test.path)
			}
		}
		if test.method == "post" || test.method == "patch" || test.method == "put" || test.method == "delete" {
			if _, ok := contract.Responses["409"]; !ok {
				t.Fatalf("%s %s must declare 409", test.method, test.path)
			}
			if !operationHasParameterRef(contract.Parameters, "#/components/parameters/IdempotencyKeyHeader") {
				t.Fatalf("%s %s must require Idempotency-Key", test.method, test.path)
			}
		}
	}

	if operationHasParameterRef(loadOperationParameters(t, document, "/api/v1/admin/me", "get"), "#/components/parameters/AdminProduct") {
		t.Fatal("GET /api/v1/admin/me must not accept product query widening")
	}

	for _, schema := range []string{
		"AdminMeResponse",
		"AdminPermissionsResponse",
		"AdminRole",
		"AdminRoleEnvelope",
		"AdminRoleListResponse",
		"AdminRoleRequest",
		"ReplaceUserAdminRolesRequest",
		"UserAdminRolesResponse",
	} {
		if _, ok := document.Components["schemas"][schema]; !ok {
			t.Fatalf("OpenAPI schema %s is missing", schema)
		}
	}
}

func TestAdminRBACAdminMeReturnsSessionScopedAuthorization(t *testing.T) {
	env := newAdminRBACHTTPTestEnv(t, "")

	unauthenticated := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/me", nil, "", "")
	if unauthenticated.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated /admin/me status = %d, want %d; body=%s", unauthenticated.Code, http.StatusUnauthorized, unauthenticated.Body.String())
	}

	reader := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/me?product=douyin_desktop", nil, env.tokens["reader"], "")
	if reader.Code != http.StatusOK {
		t.Fatalf("reader /admin/me status = %d, want %d; body=%s", reader.Code, http.StatusOK, reader.Body.String())
	}
	var readerPayload struct {
		Product          controlplane.ProductCode `json:"product"`
		GlobalSuperAdmin bool                     `json:"global_super_admin"`
		RoleCodes        []string                 `json:"role_codes"`
		Permissions      []string                 `json:"permissions"`
	}
	decodeJSON(t, reader.Body.Bytes(), &readerPayload)
	if readerPayload.Product != controlplane.ProductAutoLive {
		t.Fatalf("/admin/me widened product = %q, want %q", readerPayload.Product, controlplane.ProductAutoLive)
	}
	if readerPayload.GlobalSuperAdmin {
		t.Fatalf("reader unexpectedly became global super admin: %+v", readerPayload)
	}
	if !slices.Equal(readerPayload.RoleCodes, []string{"reader_bundle_auto"}) {
		t.Fatalf("reader role codes = %#v", readerPayload.RoleCodes)
	}
	if !slices.Equal(readerPayload.Permissions, []string{
		"activation_codes.read",
		"audit_logs.read",
		"devices.read",
		"model_leases.read",
		"model_pool.read",
		"model_usage.read",
		"users.read",
	}) {
		t.Fatalf("reader permissions = %#v", readerPayload.Permissions)
	}

	plain := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/me", nil, env.tokens["plain"], "")
	if plain.Code != http.StatusOK {
		t.Fatalf("plain /admin/me status = %d, want %d; body=%s", plain.Code, http.StatusOK, plain.Body.String())
	}
	var plainPayload struct {
		GlobalSuperAdmin bool     `json:"global_super_admin"`
		RoleCodes        []string `json:"role_codes"`
		Permissions      []string `json:"permissions"`
	}
	decodeJSON(t, plain.Body.Bytes(), &plainPayload)
	if plainPayload.GlobalSuperAdmin || len(plainPayload.RoleCodes) != 0 || len(plainPayload.Permissions) != 0 {
		t.Fatalf("plain /admin/me payload = %+v, want empty authorization", plainPayload)
	}

	failing := newAdminRBACHTTPTestEnv(t, "usr_broken")
	failClosed := doJSON(t, failing.handler, http.MethodGet, "/api/v1/admin/me", nil, failing.tokens["broken"], "")
	if failClosed.Code != http.StatusServiceUnavailable {
		t.Fatalf("failing /admin/me status = %d, want %d; body=%s", failClosed.Code, http.StatusServiceUnavailable, failClosed.Body.String())
	}
	assertErrorCode(t, failClosed.Body.Bytes(), "ADMIN_AUTHORIZATION_UNAVAILABLE")
}

func TestAdminRBACRoleAndAssignmentRoutes(t *testing.T) {
	env := newAdminRBACHTTPTestEnv(t, "")

	permissions := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/permissions", nil, env.tokens["roles"], "")
	if permissions.Code != http.StatusOK {
		t.Fatalf("GET /admin/permissions status = %d, want %d; body=%s", permissions.Code, http.StatusOK, permissions.Body.String())
	}
	var permissionsPayload struct {
		Permissions []string `json:"permissions"`
	}
	decodeJSON(t, permissions.Body.Bytes(), &permissionsPayload)
	if !slices.Contains(permissionsPayload.Permissions, "roles.manage") {
		t.Fatalf("permissions payload = %#v, want roles.manage included", permissionsPayload.Permissions)
	}

	crossProductList := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/roles?product=douyin_desktop", nil, env.tokens["roles"], "")
	if crossProductList.Code != http.StatusForbidden {
		t.Fatalf("cross-product role list status = %d, want %d; body=%s", crossProductList.Code, http.StatusForbidden, crossProductList.Body.String())
	}
	assertErrorCode(t, crossProductList.Body.Bytes(), controlplane.ErrAdminProductScopeMismatch.Code)

	missingIdempotency := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "missing_key_auto",
		"product":     "autolive",
		"name":        "Missing Key",
		"permissions": []string{"users.read"},
	}, env.tokens["roles"], "")
	if missingIdempotency.Code != http.StatusBadRequest {
		t.Fatalf("missing idempotency status = %d, want %d; body=%s", missingIdempotency.Code, http.StatusBadRequest, missingIdempotency.Body.String())
	}
	assertErrorCode(t, missingIdempotency.Body.Bytes(), controlplane.ErrIdempotencyKeyRequired.Code)

	unknownPermission := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "unknown_permission_auto",
		"product":     "autolive",
		"name":        "Unknown Permission",
		"permissions": []string{"unknown.permission"},
	}, env.tokens["roles"], "create-role-unknown")
	if unknownPermission.Code != http.StatusBadRequest {
		t.Fatalf("unknown permission status = %d, want %d; body=%s", unknownPermission.Code, http.StatusBadRequest, unknownPermission.Body.String())
	}
	assertErrorCode(t, unknownPermission.Body.Bytes(), controlplane.ErrAdminPermissionUnknown.Code)

	crossProductCreate := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "cross_product_auto",
		"product":     "douyin_desktop",
		"name":        "Cross Product",
		"permissions": []string{"users.read"},
	}, env.tokens["roles"], "create-role-cross-product")
	if crossProductCreate.Code != http.StatusForbidden {
		t.Fatalf("cross-product role create status = %d, want %d; body=%s", crossProductCreate.Code, http.StatusForbidden, crossProductCreate.Body.String())
	}
	assertErrorCode(t, crossProductCreate.Body.Bytes(), controlplane.ErrAdminProductScopeMismatch.Code)

	createRole := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "ops_auto",
		"product":     "autolive",
		"name":        "AutoLive Ops",
		"permissions": []string{"devices.read", "users.read"},
	}, env.tokens["roles"], "create-role-ops-auto")
	if createRole.Code != http.StatusCreated {
		t.Fatalf("create role status = %d, want %d; body=%s", createRole.Code, http.StatusCreated, createRole.Body.String())
	}

	replayRole := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "ops_auto",
		"product":     "autolive",
		"name":        "AutoLive Ops",
		"permissions": []string{"devices.read", "users.read"},
	}, env.tokens["roles"], "create-role-ops-auto")
	if replayRole.Code != http.StatusCreated {
		t.Fatalf("replay role status = %d, want %d; body=%s", replayRole.Code, http.StatusCreated, replayRole.Body.String())
	}

	conflict := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "ops_auto",
		"product":     "autolive",
		"name":        "Conflicting Role",
		"permissions": []string{"devices.read", "users.read"},
	}, env.tokens["roles"], "create-role-ops-auto")
	if conflict.Code != http.StatusConflict {
		t.Fatalf("conflicting replay status = %d, want %d; body=%s", conflict.Code, http.StatusConflict, conflict.Body.String())
	}
	assertErrorCode(t, conflict.Body.Bytes(), controlplane.ErrIdempotencyConflict.Code)

	getRole := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/roles/ops_auto", nil, env.tokens["roles"], "")
	if getRole.Code != http.StatusOK {
		t.Fatalf("get role status = %d, want %d; body=%s", getRole.Code, http.StatusOK, getRole.Body.String())
	}

	updateRole := doJSON(t, env.handler, http.MethodPatch, "/api/v1/admin/roles/ops_auto", map[string]any{
		"name":        "AutoLive Operations",
		"permissions": []string{"devices.manage", "users.read"},
	}, env.tokens["roles"], "update-role-ops-auto")
	if updateRole.Code != http.StatusOK {
		t.Fatalf("update role status = %d, want %d; body=%s", updateRole.Code, http.StatusOK, updateRole.Body.String())
	}

	createDeleteRole := doJSON(t, env.handler, http.MethodPost, "/api/v1/admin/roles", map[string]any{
		"code":        "delete_me_auto",
		"product":     "autolive",
		"name":        "Delete Me",
		"permissions": []string{"users.read"},
	}, env.tokens["roles"], "create-role-delete-auto")
	if createDeleteRole.Code != http.StatusCreated {
		t.Fatalf("create delete role status = %d, want %d; body=%s", createDeleteRole.Code, http.StatusCreated, createDeleteRole.Body.String())
	}

	deleteRole := doJSON(t, env.handler, http.MethodDelete, "/api/v1/admin/roles/delete_me_auto", nil, env.tokens["roles"], "delete-role-delete-auto")
	if deleteRole.Code != http.StatusNoContent {
		t.Fatalf("delete role status = %d, want %d; body=%s", deleteRole.Code, http.StatusNoContent, deleteRole.Body.String())
	}

	listAssignments := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/users/usr_target/roles?product=autolive", nil, env.tokens["roles"], "")
	if listAssignments.Code != http.StatusOK {
		t.Fatalf("list user roles status = %d, want %d; body=%s", listAssignments.Code, http.StatusOK, listAssignments.Body.String())
	}

	replaceAssignments := doJSON(t, env.handler, http.MethodPut, "/api/v1/admin/users/usr_target/roles", map[string]any{
		"product":    "autolive",
		"role_codes": []string{"ops_auto"},
	}, env.tokens["roles"], "replace-user-roles")
	if replaceAssignments.Code != http.StatusOK {
		t.Fatalf("replace user roles status = %d, want %d; body=%s", replaceAssignments.Code, http.StatusOK, replaceAssignments.Body.String())
	}

	replayAssignments := doJSON(t, env.handler, http.MethodPut, "/api/v1/admin/users/usr_target/roles", map[string]any{
		"product":    "autolive",
		"role_codes": []string{"ops_auto"},
	}, env.tokens["roles"], "replace-user-roles")
	if replayAssignments.Code != http.StatusOK {
		t.Fatalf("replay user roles status = %d, want %d; body=%s", replayAssignments.Code, http.StatusOK, replayAssignments.Body.String())
	}

	conflictingAssignments := doJSON(t, env.handler, http.MethodPut, "/api/v1/admin/users/usr_target/roles", map[string]any{
		"product":    "autolive",
		"role_codes": []string{"roles_manager_auto"},
	}, env.tokens["roles"], "replace-user-roles")
	if conflictingAssignments.Code != http.StatusConflict {
		t.Fatalf("conflicting user roles status = %d, want %d; body=%s", conflictingAssignments.Code, http.StatusConflict, conflictingAssignments.Body.String())
	}
	assertErrorCode(t, conflictingAssignments.Body.Bytes(), controlplane.ErrIdempotencyConflict.Code)

	crossProductAssignments := doJSON(t, env.handler, http.MethodPut, "/api/v1/admin/users/usr_target/roles", map[string]any{
		"product":    "douyin_desktop",
		"role_codes": []string{"ops_auto"},
	}, env.tokens["roles"], "replace-user-roles-cross")
	if crossProductAssignments.Code != http.StatusForbidden {
		t.Fatalf("cross-product user roles status = %d, want %d; body=%s", crossProductAssignments.Code, http.StatusForbidden, crossProductAssignments.Body.String())
	}
	assertErrorCode(t, crossProductAssignments.Body.Bytes(), controlplane.ErrAdminProductScopeMismatch.Code)

	verifyAssignments := doJSON(t, env.handler, http.MethodGet, "/api/v1/admin/users/usr_target/roles?product=autolive", nil, env.tokens["roles"], "")
	if verifyAssignments.Code != http.StatusOK {
		t.Fatalf("verify user roles status = %d, want %d; body=%s", verifyAssignments.Code, http.StatusOK, verifyAssignments.Body.String())
	}
	var assignmentPayload struct {
		Assignments []struct {
			UserID   string                   `json:"user_id"`
			RoleCode string                   `json:"role_code"`
			Product  controlplane.ProductCode `json:"product"`
		} `json:"assignments"`
	}
	decodeJSON(t, verifyAssignments.Body.Bytes(), &assignmentPayload)
	if !slices.Equal(assignmentRoleCodes(assignmentPayload.Assignments), []string{"ops_auto"}) {
		t.Fatalf("assigned role codes = %#v", assignmentRoleCodes(assignmentPayload.Assignments))
	}
}

func TestPermissionRoutesUseInstantPermissionsInsteadOfActorRole(t *testing.T) {
	env := newAdminRBACHTTPTestEnv(t, "")

	for _, test := range []struct {
		name       string
		method     string
		path       string
		token      string
		body       any
		idempotent string
		wantStatus int
	}{
		{name: "users read", method: http.MethodGet, path: "/api/v1/admin/users", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "devices read", method: http.MethodGet, path: "/api/v1/admin/devices", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "activation codes read", method: http.MethodGet, path: "/api/v1/admin/activation-codes", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "model pool read", method: http.MethodGet, path: "/api/v1/admin/model-pool", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "model usage read", method: http.MethodGet, path: "/api/v1/admin/model-usage", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "model leases read", method: http.MethodGet, path: "/api/v1/admin/model-leases", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "audit logs read", method: http.MethodGet, path: "/api/v1/admin/audit-logs?action=seed", token: env.tokens["reader"], wantStatus: http.StatusOK},
		{name: "security manage reaches handler", method: http.MethodPost, path: "/api/v1/admin/auth/change-password", token: env.tokens["security"], body: map[string]any{}, idempotent: "security-invalid-body", wantStatus: http.StatusBadRequest},
	} {
		t.Run(test.name, func(t *testing.T) {
			response := doJSON(t, env.handler, test.method, test.path, test.body, test.token, test.idempotent)
			if response.Code != test.wantStatus {
				t.Fatalf("%s status = %d, want %d; body=%s", test.name, response.Code, test.wantStatus, response.Body.String())
			}
		})
	}

	failing := newAdminRBACHTTPTestEnv(t, "usr_broken")
	failClosed := doJSON(t, failing.handler, http.MethodGet, "/api/v1/admin/users", nil, failing.tokens["broken"], "")
	if failClosed.Code != http.StatusServiceUnavailable {
		t.Fatalf("fail-closed users list status = %d, want %d; body=%s", failClosed.Code, http.StatusServiceUnavailable, failClosed.Body.String())
	}
	assertErrorCode(t, failClosed.Body.Bytes(), "ADMIN_AUTHORIZATION_UNAVAILABLE")
}

type adminRBACHTTPTestEnv struct {
	handler http.Handler
	tokens  map[string]string
}

type failingAdminRBACRepository struct {
	*store.MemoryStore
	failUserID string
}

func (r *failingAdminRBACRepository) GetAdminAuthorization(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.AdminAuthorization, error) {
	if userID == r.failUserID {
		return controlplane.AdminAuthorization{}, errors.New("rbac authorization unavailable")
	}
	return r.MemoryStore.GetAdminAuthorization(ctx, userID, product)
}

func newAdminRBACHTTPTestEnv(t *testing.T, failUserID string) adminRBACHTTPTestEnv {
	t.Helper()

	now := time.Date(2026, 8, 22, 12, 0, 0, 0, time.UTC)
	baseRepository := store.NewMemoryStore(func() time.Time { return now })
	repository := store.Repository(baseRepository)
	if failUserID != "" {
		repository = &failingAdminRBACRepository{MemoryStore: baseRepository, failUserID: failUserID}
	}

	if err := baseRepository.Run(context.Background(), func(state *store.State) error {
		seedAdminRBACHTTPUser(t, state, now, "usr_reader", "reader", "reader-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_roles", "roles-admin", "roles-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_security", "security-admin", "security-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_plain", "plain-user", "plain-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_target", "target-user", "target-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_global", "global-admin", "global-password", controlplane.RoleUser)
		seedAdminRBACHTTPUser(t, state, now, "usr_broken", "broken-admin", "broken-password", controlplane.RoleUser)

		for _, userID := range []string{"usr_reader", "usr_roles", "usr_security", "usr_plain", "usr_target", "usr_global", "usr_broken"} {
			state.UserProducts[userID+":autolive"] = controlplane.UserProductMembership{UserID: userID, Product: controlplane.ProductAutoLive, Status: "active"}
		}
		state.UserProducts["usr_target:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_target", Product: controlplane.ProductDouyinDesktop, Status: "active"}

		seedAdminRBACHTTPRole(state, store.AdminRoleRecord{
			Code:        "reader_bundle_auto",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Reader Bundle",
			Permissions: []controlplane.PermissionCode{"activation_codes.read", "audit_logs.read", "devices.read", "model_leases.read", "model_pool.read", "model_usage.read", "users.read"},
		})
		seedAdminRBACHTTPRole(state, store.AdminRoleRecord{
			Code:        "roles_manager_auto",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Roles Manager",
			Permissions: []controlplane.PermissionCode{"devices.manage", "devices.read", "roles.assign", "roles.manage", "roles.read", "users.read"},
		})
		seedAdminRBACHTTPRole(state, store.AdminRoleRecord{
			Code:        "security_manager_auto",
			Product:     controlplane.ProductAutoLive,
			Name:        "AutoLive Security Manager",
			Permissions: []controlplane.PermissionCode{"admin_security.manage"},
		})
		seedAdminRBACHTTPRole(state, store.AdminRoleRecord{
			Code:        "douyin_reader",
			Product:     controlplane.ProductDouyinDesktop,
			Name:        "Douyin Reader",
			Permissions: []controlplane.PermissionCode{"users.read"},
		})

		state.UserAdminRoles["usr_reader\x1freader_bundle_auto\x1fautolive"] = controlplane.AdminRoleAssignment{UserID: "usr_reader", RoleCode: "reader_bundle_auto", Product: controlplane.ProductAutoLive}
		state.UserAdminRoles["usr_roles\x1froles_manager_auto\x1fautolive"] = controlplane.AdminRoleAssignment{UserID: "usr_roles", RoleCode: "roles_manager_auto", Product: controlplane.ProductAutoLive}
		state.UserAdminRoles["usr_security\x1fsecurity_manager_auto\x1fautolive"] = controlplane.AdminRoleAssignment{UserID: "usr_security", RoleCode: "security_manager_auto", Product: controlplane.ProductAutoLive}
		state.UserAdminRoles["usr_global\x1fsuper_admin\x1f"] = controlplane.AdminRoleAssignment{UserID: "usr_global", RoleCode: controlplane.BuiltinAdminRoleSuperAdmin}
		state.UserAdminRoles["usr_broken\x1froles_manager_auto\x1fautolive"] = controlplane.AdminRoleAssignment{UserID: "usr_broken", RoleCode: "roles_manager_auto", Product: controlplane.ProductAutoLive}

		state.Users["usr_shared"] = controlplane.UserSummary{ID: "usr_shared", Username: "shared-user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.UserProducts["usr_shared:autolive"] = controlplane.UserProductMembership{UserID: "usr_shared", Product: controlplane.ProductAutoLive, Status: "active"}
		state.UserProducts["usr_shared:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_shared", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		state.Devices["dev_auto"] = controlplane.DeviceSummary{ID: "dev_auto", UserID: "usr_shared", Product: controlplane.ProductAutoLive, DeviceName: "Auto Device", Platform: "macOS", Status: controlplane.DeviceStatusActive}
		state.ActivationCodes["code_auto"] = store.ActivationCodeRecord{ActivationCode: controlplane.ActivationCode{ID: "code_auto", Product: controlplane.ProductAutoLive, Status: controlplane.ActivationCodeStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339), MaxDevices: 2}}
		state.ModelPoolAccounts["account_auto"] = controlplane.ModelPoolAccountSummary{ID: "account_auto", Product: controlplane.ProductAutoLive, Provider: "openai-compatible", Model: "gpt-test", Status: controlplane.ModelAccountStatusActive}
		state.ModelLeases["lease_auto"] = controlplane.ModelLease{ID: "lease_auto", Product: controlplane.ProductAutoLive, UserID: "usr_shared", DeviceID: "dev_auto", AccountID: "account_auto", Provider: "openai-compatible", Model: "gpt-test", Status: controlplane.ModelLeaseStatusActive, ExpiresAt: now.Add(time.Hour).Format(time.RFC3339)}
		state.ModelUsageRecords["usage_auto"] = controlplane.ModelUsageRecord{ID: "usage_auto", Product: controlplane.ProductAutoLive, LeaseID: "lease_auto", Provider: "openai-compatible", Model: "gpt-test", Status: "succeeded", CreatedAt: now.Format(time.RFC3339)}
		state.AuditLogs["audit_auto"] = controlplane.AuditLog{ID: "audit_auto", Product: controlplane.ProductAutoLive, Action: "seed", Outcome: "success", CreatedAt: now.Format(time.RFC3339)}
		state.Users["usr_douyin_only"] = controlplane.UserSummary{ID: "usr_douyin_only", Username: "douyin-user", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
		state.UserProducts["usr_douyin_only:douyin_desktop"] = controlplane.UserProductMembership{UserID: "usr_douyin_only", Product: controlplane.ProductDouyinDesktop, Status: "active"}
		return nil
	}); err != nil {
		t.Fatalf("seed admin rbac http state: %v", err)
	}

	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStoreAndOptions("test", nil, AuthConfig{
		Username: "admin",
		Password: "password",
	}, repository, store.NewMemorySecretStore(), nil, true)

	return adminRBACHTTPTestEnv{
		handler: handler,
		tokens: map[string]string{
			"reader":   loginWithCredentialsAtIPForTest(t, handler, `{"username":"reader","password":"reader-password","product":"autolive"}`, "198.51.100.11:1234"),
			"roles":    loginWithCredentialsAtIPForTest(t, handler, `{"username":"roles-admin","password":"roles-password","product":"autolive"}`, "198.51.100.12:1234"),
			"security": loginWithCredentialsAtIPForTest(t, handler, `{"username":"security-admin","password":"security-password","product":"autolive"}`, "198.51.100.13:1234"),
			"plain":    loginWithCredentialsAtIPForTest(t, handler, `{"username":"plain-user","password":"plain-password","product":"autolive"}`, "198.51.100.14:1234"),
			"global":   loginWithCredentialsAtIPForTest(t, handler, `{"username":"global-admin","password":"global-password","product":"autolive"}`, "198.51.100.15:1234"),
			"broken":   loginWithCredentialsAtIPForTest(t, handler, `{"username":"broken-admin","password":"broken-password","product":"autolive"}`, "198.51.100.16:1234"),
		},
	}
}

func seedAdminRBACHTTPUser(t *testing.T, state *store.State, now time.Time, userID, username, password, role string) {
	t.Helper()
	passwordHash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.MinCost)
	if err != nil {
		t.Fatalf("hash %s password: %v", userID, err)
	}
	state.Users[userID] = controlplane.UserSummary{ID: userID, Username: username, Role: role, Status: controlplane.UserStatusActive, CreatedAt: now.Format(time.RFC3339)}
	state.UserCredentialHashes[userID] = passwordHash
}

func seedAdminRBACHTTPRole(state *store.State, role store.AdminRoleRecord) {
	state.AdminRoles[role.Code] = role
	state.AdminRolePermissions[role.Code] = append([]controlplane.PermissionCode(nil), role.Permissions...)
}

func loadOperationParameters(t *testing.T, document openAPIDocument, path, method string) []struct {
	Ref  string `yaml:"$ref"`
	Name string `yaml:"name"`
} {
	t.Helper()
	node, ok := document.Paths[path][method]
	if !ok {
		t.Fatalf("%s %s is missing", method, path)
	}
	var contract struct {
		Parameters []struct {
			Ref  string `yaml:"$ref"`
			Name string `yaml:"name"`
		} `yaml:"parameters"`
	}
	if err := node.Decode(&contract); err != nil {
		t.Fatalf("decode %s %s parameters: %v", method, path, err)
	}
	return contract.Parameters
}

func operationHasParameterRef(parameters []struct {
	Ref  string `yaml:"$ref"`
	Name string `yaml:"name"`
}, ref string) bool {
	for _, parameter := range parameters {
		if parameter.Ref == ref {
			return true
		}
	}
	return false
}

func assertErrorCode(t *testing.T, body []byte, want string) {
	t.Helper()
	var payload ErrorResponse
	if err := json.Unmarshal(body, &payload); err != nil {
		t.Fatalf("decode error response: %v; body=%s", err, body)
	}
	if payload.Code != want {
		t.Fatalf("error code = %q, want %q; body=%s", payload.Code, want, body)
	}
}

func loginWithCredentialsAtIPForTest(t *testing.T, handler http.Handler, body, remoteAddr string) string {
	t.Helper()

	request := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	request.RemoteAddr = remoteAddr
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d; body=%s", response.Code, http.StatusOK, response.Body.String())
	}

	var payload loginResponse
	decodeJSON(t, response.Body.Bytes(), &payload)
	return payload.Tokens.AccessToken
}

func assignmentRoleCodes(values []struct {
	UserID   string                   `json:"user_id"`
	RoleCode string                   `json:"role_code"`
	Product  controlplane.ProductCode `json:"product"`
}) []string {
	codes := make([]string, 0, len(values))
	for _, value := range values {
		codes = append(codes, value.RoleCode)
	}
	return codes
}
