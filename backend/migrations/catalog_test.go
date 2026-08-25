package migrations

import (
	"fmt"
	"io/fs"
	"regexp"
	"strconv"
	"strings"
	"testing"
)

type migration23ForeignKeyContract struct {
	table             string
	name              string
	parentTable       string
	localColumns      string
	referencedColumns string
}

var migration23ForeignKeyContracts = []migration23ForeignKeyContract{
	{table: "devices", name: "devices_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "activation_codes", name: "activation_codes_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "auth_sessions", name: "auth_sessions_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "model_accounts", name: "model_accounts_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "model_leases", name: "model_leases_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "model_usage_records", name: "model_usage_records_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "model_request_reservations", name: "model_request_reservations_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "audit_logs", name: "audit_logs_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "audit_outbox", name: "audit_outbox_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "model_pool_test_results", name: "model_pool_test_results_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "user_authorization_policies", name: "user_authorization_policies_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "idempotency_records", name: "idempotency_records_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "variant_tasks", name: "variant_tasks_product_fkey", parentTable: "products", localColumns: "product", referencedColumns: "code"},
	{table: "devices", name: "devices_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "activation_codes", name: "activation_codes_used_by_user_product_fkey", parentTable: "user_products", localColumns: "used_by_user_id, product", referencedColumns: "user_id, product"},
	{table: "auth_sessions", name: "auth_sessions_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "model_request_reservations", name: "model_request_reservations_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "model_leases", name: "model_leases_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "model_usage_records", name: "model_usage_records_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "audit_logs", name: "audit_logs_actor_user_product_fkey", parentTable: "user_products", localColumns: "actor_user_id, product", referencedColumns: "user_id, product"},
	{table: "user_authorization_policies", name: "user_authorization_policies_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
	{table: "variant_tasks", name: "variant_tasks_user_product_fkey", parentTable: "user_products", localColumns: "user_id, product", referencedColumns: "user_id, product"},
}

func TestEmbeddedMigrationsUseVersionedUpSQLFiles(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}
	if len(entries) == 0 {
		t.Fatal("embedded migration catalog is empty")
	}
	for _, entry := range entries {
		if entry.IsDir() || len(entry.Name()) < 8 || entry.Name()[:4] < "0001" {
			t.Fatalf("invalid migration entry: %s", entry.Name())
		}
		if len(entry.Name()) < 7 || entry.Name()[len(entry.Name())-7:] != ".up.sql" {
			t.Fatalf("migration %s must end with .up.sql", entry.Name())
		}
	}
}

func TestEmbeddedMigrationsAreContiguousAndAppendOnly(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}

	versionByFile := regexp.MustCompile(`^(\d{4})_.+\.up\.sql$`)
	seen := make(map[int]string, len(entries))

	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		matches := versionByFile.FindStringSubmatch(entry.Name())
		if matches == nil {
			t.Fatalf("migration %s must match 4-digit versioned .up.sql naming", entry.Name())
		}
		version, err := strconv.Atoi(matches[1])
		if err != nil {
			t.Fatalf("parse migration version from %s: %v", entry.Name(), err)
		}
		if prior, ok := seen[version]; ok {
			t.Fatalf("duplicate migration version %04d in %s and %s", version, prior, entry.Name())
		}
		seen[version] = entry.Name()
	}

	for version := 1; version <= LatestVersion; version++ {
		if _, ok := seen[version]; !ok {
			t.Fatalf("missing migration version %04d; embedded catalog must be contiguous from 0001 to %04d", version, LatestVersion)
		}
	}
	if len(seen) != LatestVersion {
		t.Fatalf("embedded migration version count = %d, want %d", len(seen), LatestVersion)
	}
}

func TestLatestVersionMatchesEmbeddedCatalog(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}
	if len(entries) != LatestVersion {
		t.Fatalf("embedded migration count = %d, want latest version %d", len(entries), LatestVersion)
	}
}

func TestMigration23ProductIsolationContract(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0023_补齐多产品控制面.up.sql")
	if err != nil {
		t.Fatalf("read migration 0023: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"CREATE TABLE IF NOT EXISTS products",
		"'autolive'",
		"'douyin_desktop'",
		"CREATE TABLE IF NOT EXISTS user_products",
		"user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE",
		"product TEXT NOT NULL REFERENCES products(code)",
		"ALTER TABLE devices",
		"ADD COLUMN IF NOT EXISTS product",
		"DEFAULT 'autolive'",
		"auth_sessions",
		"user_authorization_policies",
		"autolive_seed_default_user_product",
		"idx_devices_product",
		"idx_auth_sessions_product",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0023 is missing product-isolation fragment %q", fragment)
		}
	}
	normalizedSQL := strings.ToLower(strings.Join(strings.Fields(sql), " "))
	legacyAuthSessionsDeviceReference := regexp.MustCompile(`(?:alter\s+table(?:\s+only)?|create\s+table)\s+auth_sessions\b[^;]*references\s+devices\b`)
	if legacyAuthSessionsDeviceReference.MatchString(normalizedSQL) {
		t.Fatal("migration 0023 must not reintroduce any auth_sessions foreign key referencing devices")
	}
	for _, fragment := range []string{
		"ALTER COLUMN product SET NOT NULL",
		"DROP CONSTRAINT IF EXISTS devices_pkey",
		"PRIMARY KEY (product, id)",
		"auth_sessions contains orphan device bindings",
	} {
		if strings.Contains(sql, fragment) {
			t.Fatalf("migration 0023 must remain a compatibility expansion before Task 3-5 propagation, found forbidden fragment %q", fragment)
		}
	}
}

func TestMigration23ProductScopedConstraintChecksAreTableQualified(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0023_补齐多产品控制面.up.sql")
	if err != nil {
		t.Fatalf("read migration 0023: %v", err)
	}
	sql := string(payload)
	normalizedSQL := strings.ToLower(strings.Join(strings.Fields(sql), " "))
	for _, fragment := range []string{
		"WHERE conrelid = 'devices'::regclass AND conname = 'devices_user_product_fkey'",
		"WHERE conrelid = 'activation_codes'::regclass AND conname = 'activation_codes_used_by_user_product_fkey'",
		"WHERE conrelid = 'auth_sessions'::regclass AND conname = 'auth_sessions_user_product_fkey'",
		"WHERE conrelid = 'model_leases'::regclass AND conname = 'model_leases_user_product_fkey'",
		"WHERE conrelid = 'model_usage_records'::regclass AND conname = 'model_usage_records_user_product_fkey'",
		"WHERE conrelid = 'audit_logs'::regclass AND conname = 'audit_logs_actor_user_product_fkey'",
		"WHERE conrelid = 'user_authorization_policies'::regclass AND conname = 'user_authorization_policies_user_product_fkey'",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0023 must scope product FK check with table-qualified regclass, missing %q", fragment)
		}
	}
	if strings.Contains(sql, "IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = '") {
		t.Fatal("migration 0023 must not use global pg_constraint conname-only guards for product FK checks")
	}
	if !strings.Contains(normalizedSQL, "foreign key (product) references products(code)") {
		t.Fatal("migration 0023 must define product foreign keys on the product column")
	}
}

func TestMigration23ForeignKeyContractCoversAllProductReferences(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0023_补齐多产品控制面.up.sql")
	if err != nil {
		t.Fatalf("read migration 0023: %v", err)
	}
	sql := string(payload)
	normalizedSQL := strings.Join(strings.Fields(sql), " ")
	userProductForeignKeyCount := 0
	productForeignKeyCount := 0

	for _, contract := range migration23ForeignKeyContracts {
		if contract.parentTable == "products" {
			productForeignKeyCount++
			fragment := fmt.Sprintf("('%s'::regclass, '%s')", contract.table, contract.name)
			if !strings.Contains(sql, fragment) {
				t.Fatalf("migration 0023 product FK catalog is missing %q", fragment)
			}
			continue
		}

		userProductForeignKeyCount++
		guard := fmt.Sprintf("WHERE conrelid = '%s'::regclass AND conname = '%s'", contract.table, contract.name)
		if !strings.Contains(sql, guard) {
			t.Fatalf("migration 0023 user_products FK guard is missing %q", guard)
		}
		definition := fmt.Sprintf(
			"ALTER TABLE %s ADD CONSTRAINT %s FOREIGN KEY (%s) REFERENCES %s(%s);",
			contract.table,
			contract.name,
			contract.localColumns,
			contract.parentTable,
			contract.referencedColumns,
		)
		if !strings.Contains(normalizedSQL, strings.Join(strings.Fields(definition), " ")) {
			t.Fatalf("migration 0023 is missing user_products FK definition %q", definition)
		}
	}

	if productForeignKeyCount != 13 {
		t.Fatalf("migration 0023 product FK contract count = %d, want 13", productForeignKeyCount)
	}
	if userProductForeignKeyCount != 9 {
		t.Fatalf("migration 0023 user_products FK contract count = %d, want 9", userProductForeignKeyCount)
	}
	if actual := strings.Count(normalizedSQL, "REFERENCES user_products(user_id, product)"); actual != userProductForeignKeyCount {
		t.Fatalf("migration 0023 user_products FK definition count = %d, want %d", actual, userProductForeignKeyCount)
	}
}

func TestLatestVersionIsAccountActivationMigration25(t *testing.T) {
	if LatestVersion != 25 {
		t.Fatalf("LatestVersion = %d, want 25", LatestVersion)
	}
}

func TestMigration25BindsActivationAuthorizationToAccountAndDevice(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0025_激活授权绑定账号与设备.up.sql")
	if err != nil {
		t.Fatalf("read migration 0025: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS bound_user_id",
		"SET bound_user_id = used_by_user_id",
		"SET status = 'revoked'",
		"activation_codes_bound_user_product_fkey",
		"CREATE TABLE IF NOT EXISTS activation_device_bindings",
		"FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product)",
		"uq_activation_codes_binding_identity",
		"uq_devices_binding_identity",
		"FOREIGN KEY (activation_code_id, product, user_id)",
		"REFERENCES activation_codes(id, product, bound_user_id)",
		"FOREIGN KEY (device_id, product, user_id)",
		"REFERENCES devices(id, product, user_id)",
		"GROUP BY used_by_device_id",
		"HAVING COUNT(*) = 1",
		"ON CONFLICT (device_id) DO NOTHING",
		"idx_activation_codes_account_capacity",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0025 is missing account activation fragment %q", fragment)
		}
	}
	if strings.Contains(sql, "SET bound_devices =") {
		t.Fatal("migration 0025 must preserve unknown historical occupied capacity")
	}
}

func TestMigration24AdminRBACContract(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0024_管理员RBAC与产品角色.up.sql")
	if err != nil {
		t.Fatalf("read migration 0024: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"CREATE TABLE IF NOT EXISTS admin_permissions",
		"CREATE TABLE IF NOT EXISTS admin_roles",
		"CREATE TABLE IF NOT EXISTS admin_role_permissions",
		"CREATE TABLE IF NOT EXISTS user_admin_roles",
		"FOREIGN KEY (product) REFERENCES products(code)",
		"FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product)",
		"FOREIGN KEY (role_code) REFERENCES admin_roles(code)",
		"FOREIGN KEY (permission_code) REFERENCES admin_permissions(code)",
		"super_admin",
		"usr_local_admin",
		"users.role = 'admin'",
		"users.role = 'user'",
		"idx_admin_roles_product",
		"idx_user_admin_roles_product_user",
		"uq_user_admin_roles_binding",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0024 is missing RBAC fragment %q", fragment)
		}
	}
}

func TestActivationMultiDeviceMigrationAddsCapacityAndCount(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0022_支持激活码多设备绑定.up.sql")
	if err != nil {
		t.Fatalf("read migration 0022: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS max_devices",
		"ADD COLUMN IF NOT EXISTS bound_devices",
		"used_by_device_id IS NOT NULL",
		"max_devices BETWEEN 1 AND 100",
		"bound_devices BETWEEN 0 AND max_devices",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0022 is missing fragment %q", fragment)
		}
	}
}

func TestUsageSummaryMigrationGuardsHistoricalDuplicates(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0012_补齐调用摘要幂等唯一约束.up.sql")
	if err != nil {
		t.Fatalf("read migration 0012: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"GROUP BY lease_id, client_call_id",
		"RAISE EXCEPTION",
		"uq_model_usage_lease_client_call",
		"WHERE client_call_id <> ''",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0012 is missing safety fragment %q", fragment)
		}
	}
}

func TestNormalizedStateMigrationAddsRequiredFields(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0013_补齐规范化设备与模型测试字段.up.sql")
	if err != nil {
		t.Fatalf("read migration 0013: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS device_name",
		"ADD COLUMN IF NOT EXISTS platform",
		"CREATE TABLE IF NOT EXISTS model_pool_test_results",
		"payload JSONB NOT NULL",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0013 is missing fragment %q", fragment)
		}
	}
}

func TestModelAccountCooldownMigrationAddsBoundedRecoveryField(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0014_补齐模型账号冷却时间.up.sql")
	if err != nil {
		t.Fatalf("read migration 0014: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS cooldown_until",
		"model_accounts_cooldown_until_idx",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0014 is missing fragment %q", fragment)
		}
	}
}

func TestAuditSemanticsMigrationAddsResultFields(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0015_补齐审计结果语义字段.up.sql")
	if err != nil {
		t.Fatalf("read migration 0015: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS outcome",
		"ADD COLUMN IF NOT EXISTS status_code",
		"audit_logs_outcome_check",
		"idx_audit_logs_outcome_created_at",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0015 is missing fragment %q", fragment)
		}
	}
}

func TestUserAuthorizationPolicyMigrationAddsBoundedPolicyTable(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0016_补齐用户模型授权策略.up.sql")
	if err != nil {
		t.Fatalf("read migration 0016: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"CREATE TABLE IF NOT EXISTS user_authorization_policies",
		"allowed_models JSONB",
		"daily_token_limit BIGINT",
		"REFERENCES users(id) ON DELETE CASCADE",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0016 is missing fragment %q", fragment)
		}
	}
}

func TestControlPlaneRetentionAndFilterIndexesMigration(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0017_补齐控制面清理与筛选索引.up.sql")
	if err != nil {
		t.Fatalf("read migration 0017: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"idx_auth_sessions_refresh_expiry_active",
		"idx_idempotency_records_created_at",
		"idx_model_pool_test_results_created_at",
		"idx_model_leases_account_status_expiry",
		"idx_model_accounts_provider_model_status",
		"idx_audit_logs_request_created_at",
		"WHERE revoked_at IS NULL",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0017 is missing fragment %q", fragment)
		}
	}
}

func TestSessionBindingRecoveryMigrationAddsMarkerAndIndex(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0018_补齐会话绑定恢复标记.up.sql")
	if err != nil {
		t.Fatalf("read migration 0018: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"ADD COLUMN IF NOT EXISTS device_bound_at",
		"DROP CONSTRAINT",
		"UPDATE auth_sessions",
		"idx_auth_sessions_orphan_device_binding",
		"device_bound_at IS NOT NULL",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0018 is missing fragment %q", fragment)
		}
	}
}

func TestAuditOutboxMigrationAddsDurableRetryFields(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0019_补齐审计投递Outbox.up.sql")
	if err != nil {
		t.Fatalf("read migration 0019: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"CREATE TABLE IF NOT EXISTS audit_outbox",
		"dedupe_key TEXT NOT NULL UNIQUE",
		"status TEXT NOT NULL",
		"next_attempt_at TIMESTAMPTZ NOT NULL",
		"idx_audit_outbox_dispatch",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0019 is missing fragment %q", fragment)
		}
	}
}

func TestNormalizedBackfillMigrationAddsExplicitRuntimeGate(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0020_规范化回填完成状态.up.sql")
	if err != nil {
		t.Fatalf("read migration 0020: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"CREATE TABLE IF NOT EXISTS normalized_backfill_state",
		"status TEXT NOT NULL CHECK (status IN ('pending', 'completed'))",
		"VALUES (TRUE, 'pending'",
		"autolive_require_normalized_backfill_completed",
		"RAISE EXCEPTION",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0020 is missing fragment %q", fragment)
		}
	}
}

func TestManagementFilterIndexesMigrationAddsBoundedCompositeIndexes(t *testing.T) {
	payload, err := fs.ReadFile(FS, "0021_补齐管理查询筛选索引.up.sql")
	if err != nil {
		t.Fatalf("read migration 0021: %v", err)
	}
	sql := string(payload)
	for _, fragment := range []string{
		"idx_model_leases_user_status_expiry",
		"idx_model_leases_device_status_expiry",
		"idx_model_leases_provider_model_status_expiry",
		"idx_audit_logs_actor_created_at",
		"idx_audit_logs_device_created_at",
		"idx_audit_logs_action_created_at",
		"idx_audit_logs_resource_created_at",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0021 is missing fragment %q", fragment)
		}
	}
}
