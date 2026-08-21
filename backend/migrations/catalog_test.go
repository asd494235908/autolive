package migrations

import (
	"io/fs"
	"strings"
	"testing"
)

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

func TestLatestVersionMatchesEmbeddedCatalog(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}
	if len(entries) != LatestVersion {
		t.Fatalf("embedded migration count = %d, want latest version %d", len(entries), LatestVersion)
	}
}

func TestLatestVersionIsProductIsolationMigration23(t *testing.T) {
	if LatestVersion != 23 {
		t.Fatalf("LatestVersion = %d, want 23", LatestVersion)
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
		"ALTER TABLE devices",
		"ADD COLUMN IF NOT EXISTS product",
		"ALTER COLUMN product SET NOT NULL",
		"devices_product_id_pkey PRIMARY KEY (product, id)",
		"FOREIGN KEY (product, used_by_device_id) REFERENCES devices(product, id)",
		"auth_sessions",
		"user_authorization_policies",
		"idx_devices_product_device_id",
		"idx_auth_sessions_product_device",
	} {
		if !strings.Contains(sql, fragment) {
			t.Fatalf("migration 0023 is missing product-isolation fragment %q", fragment)
		}
	}
	if strings.Contains(sql, "auth_sessions_product_device_fkey") {
		t.Fatal("migration 0023 must not reintroduce auth_sessions device foreign key dropped by migration 0018")
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
