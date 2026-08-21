-- 产品注册与成员关系是规范化事实源，不写入 control_plane_state 快照。
CREATE TABLE IF NOT EXISTS products (
    code TEXT PRIMARY KEY,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT products_code_check CHECK (code IN ('autolive', 'douyin_desktop')),
    CONSTRAINT products_status_check CHECK (status IN ('active', 'disabled'))
);

CREATE TABLE IF NOT EXISTS user_products (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    product TEXT NOT NULL REFERENCES products(code),
    status TEXT NOT NULL DEFAULT 'active',
    entitlement_revision BIGINT NOT NULL DEFAULT 0 CHECK (entitlement_revision >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, product),
    CONSTRAINT user_products_status_check CHECK (status IN ('active', 'disabled'))
);

INSERT INTO products (code, status, created_at)
VALUES
    ('autolive', 'active', CURRENT_TIMESTAMP),
    ('douyin_desktop', 'active', CURRENT_TIMESTAMP)
ON CONFLICT (code) DO NOTHING;

-- 保持历史用户在原产品下可用；新产品必须显式建立成员关系。
INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
SELECT id, 'autolive', 'active', 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
FROM users
ON CONFLICT (user_id, product) DO NOTHING;

-- 先扩展为可空列，完成确定性回填后在本迁移末尾收紧为非空。
ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE auth_sessions
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE model_accounts
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE model_leases
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE model_usage_records
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE model_request_reservations
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE audit_outbox
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE model_pool_test_results
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE user_authorization_policies
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE idempotency_records
    ADD COLUMN IF NOT EXISTS product TEXT;
ALTER TABLE variant_tasks
    ADD COLUMN IF NOT EXISTS product TEXT;

UPDATE devices SET product = 'autolive' WHERE product IS NULL;
UPDATE activation_codes SET product = 'autolive' WHERE product IS NULL;
UPDATE auth_sessions SET product = 'autolive' WHERE product IS NULL;
UPDATE model_accounts SET product = 'autolive' WHERE product IS NULL;
UPDATE model_leases SET product = 'autolive' WHERE product IS NULL;
UPDATE model_usage_records SET product = 'autolive' WHERE product IS NULL;
UPDATE model_request_reservations SET product = 'autolive' WHERE product IS NULL;
UPDATE audit_logs SET product = 'autolive' WHERE product IS NULL;
UPDATE audit_outbox SET product = 'autolive' WHERE product IS NULL;
UPDATE model_pool_test_results SET product = 'autolive' WHERE product IS NULL;
UPDATE user_authorization_policies SET product = 'autolive' WHERE product IS NULL;
UPDATE idempotency_records SET product = 'autolive' WHERE product IS NULL;
UPDATE variant_tasks SET product = 'autolive' WHERE product IS NULL;

ALTER TABLE devices ALTER COLUMN product SET NOT NULL;
ALTER TABLE activation_codes ALTER COLUMN product SET NOT NULL;
ALTER TABLE auth_sessions ALTER COLUMN product SET NOT NULL;
ALTER TABLE model_accounts ALTER COLUMN product SET NOT NULL;
ALTER TABLE model_leases ALTER COLUMN product SET NOT NULL;
ALTER TABLE model_usage_records ALTER COLUMN product SET NOT NULL;
ALTER TABLE model_request_reservations ALTER COLUMN product SET NOT NULL;
ALTER TABLE audit_logs ALTER COLUMN product SET NOT NULL;
ALTER TABLE audit_outbox ALTER COLUMN product SET NOT NULL;
ALTER TABLE model_pool_test_results ALTER COLUMN product SET NOT NULL;
ALTER TABLE user_authorization_policies ALTER COLUMN product SET NOT NULL;
ALTER TABLE idempotency_records ALTER COLUMN product SET NOT NULL;
ALTER TABLE variant_tasks ALTER COLUMN product SET NOT NULL;

-- 0018 有意移除了会话到设备的旧单列外键。迁移前必须先核对其可能的
-- 崩溃遗留引用，不能通过放宽新约束静默丢失绑定事实。
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM auth_sessions AS s
        WHERE s.device_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM devices AS d
              WHERE d.product = s.product AND d.id = s.device_id
          )
    ) THEN
        RAISE EXCEPTION 'auth_sessions contains orphan device bindings; reconcile before migration 0023';
    END IF;
END
$$;

-- 删除历史单列设备/资源外键，随后统一建立带 product 的复合外键。
DO $$
DECLARE
    item RECORD;
BEGIN
    FOR item IN
        SELECT c.conrelid, c.conname
        FROM pg_constraint AS c
        WHERE c.contype = 'f'
          AND c.confrelid IN (
              'devices'::regclass,
              'model_accounts'::regclass,
              'model_leases'::regclass,
              'model_usage_records'::regclass
          )
          AND c.conrelid IN (
              'activation_codes'::regclass,
              'auth_sessions'::regclass,
              'model_leases'::regclass,
              'model_usage_records'::regclass,
              'model_request_reservations'::regclass,
              'audit_logs'::regclass,
              'model_pool_test_results'::regclass,
              'variant_tasks'::regclass
          )
    LOOP
        EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I', item.conrelid, item.conname);
    END LOOP;
END
$$;

-- 设备 id 从跨产品全局主键变为产品范围内的逻辑键；device_key 的唯一性
-- 也必须收口到 product，否则同一客户端 ID 仍会被旧约束挡住。
ALTER TABLE devices DROP CONSTRAINT IF EXISTS devices_pkey;
ALTER TABLE devices DROP CONSTRAINT IF EXISTS devices_device_key_key;

ALTER TABLE activation_codes DROP CONSTRAINT IF EXISTS activation_codes_code_hash_key;
ALTER TABLE model_request_reservations DROP CONSTRAINT IF EXISTS model_request_reservations_pkey;
ALTER TABLE model_request_reservations DROP CONSTRAINT IF EXISTS model_request_reservations_request_id_key;
ALTER TABLE user_authorization_policies DROP CONSTRAINT IF EXISTS user_authorization_policies_pkey;
ALTER TABLE idempotency_records DROP CONSTRAINT IF EXISTS idempotency_records_pkey;
ALTER TABLE audit_outbox DROP CONSTRAINT IF EXISTS audit_outbox_dedupe_key_key;
DROP INDEX IF EXISTS uq_model_usage_lease_client_call;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'devices_product_id_pkey') THEN
        ALTER TABLE devices ADD CONSTRAINT devices_product_id_pkey PRIMARY KEY (product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'devices_product_device_key_key') THEN
        ALTER TABLE devices ADD CONSTRAINT devices_product_device_key_key UNIQUE (product, device_key);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'activation_codes_product_code_hash_key') THEN
        ALTER TABLE activation_codes ADD CONSTRAINT activation_codes_product_code_hash_key UNIQUE (product, code_hash);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_accounts_product_id_key') THEN
        ALTER TABLE model_accounts ADD CONSTRAINT model_accounts_product_id_key UNIQUE (product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_leases_product_id_key') THEN
        ALTER TABLE model_leases ADD CONSTRAINT model_leases_product_id_key UNIQUE (product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_usage_records_product_id_key') THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_product_id_key UNIQUE (product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_scope_pkey') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_scope_pkey PRIMARY KEY (product, scope);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_request_id_key') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_request_id_key UNIQUE (product, request_id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'user_authorization_policies_user_product_pkey') THEN
        ALTER TABLE user_authorization_policies ADD CONSTRAINT user_authorization_policies_user_product_pkey PRIMARY KEY (user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'idempotency_records_product_scope_key_pkey') THEN
        ALTER TABLE idempotency_records ADD CONSTRAINT idempotency_records_product_scope_key_pkey PRIMARY KEY (product, scope, idempotency_key);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'audit_outbox_product_dedupe_key_key') THEN
        ALTER TABLE audit_outbox ADD CONSTRAINT audit_outbox_product_dedupe_key_key UNIQUE (product, dedupe_key);
    END IF;
END
$$;

-- 所有当前领域表的 product 字段均受注册表约束，禁止写入未注册产品。
DO $$
DECLARE
    item RECORD;
BEGIN
    FOR item IN
        SELECT *
        FROM (VALUES
            ('devices'::regclass, 'devices_product_fkey'),
            ('activation_codes'::regclass, 'activation_codes_product_fkey'),
            ('auth_sessions'::regclass, 'auth_sessions_product_fkey'),
            ('model_accounts'::regclass, 'model_accounts_product_fkey'),
            ('model_leases'::regclass, 'model_leases_product_fkey'),
            ('model_usage_records'::regclass, 'model_usage_records_product_fkey'),
            ('model_request_reservations'::regclass, 'model_request_reservations_product_fkey'),
            ('audit_logs'::regclass, 'audit_logs_product_fkey'),
            ('audit_outbox'::regclass, 'audit_outbox_product_fkey'),
            ('model_pool_test_results'::regclass, 'model_pool_test_results_product_fkey'),
            ('user_authorization_policies'::regclass, 'user_authorization_policies_product_fkey'),
            ('idempotency_records'::regclass, 'idempotency_records_product_fkey'),
            ('variant_tasks'::regclass, 'variant_tasks_product_fkey')
        ) AS v(table_name, constraint_name)
    LOOP
        IF NOT EXISTS (
            SELECT 1
            FROM pg_constraint
            WHERE conrelid = item.table_name AND conname = item.constraint_name
        ) THEN
            EXECUTE format(
                'ALTER TABLE %s ADD CONSTRAINT %I FOREIGN KEY (product) REFERENCES products(code)',
                item.table_name,
                item.constraint_name
            );
        END IF;
    END LOOP;
END
$$;

-- 产品范围与用户、设备及相关资源的关系必须同时受数据库约束保护。
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'devices_user_product_fkey') THEN
        ALTER TABLE devices ADD CONSTRAINT devices_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'activation_codes_used_by_user_product_fkey') THEN
        ALTER TABLE activation_codes ADD CONSTRAINT activation_codes_used_by_user_product_fkey
            FOREIGN KEY (used_by_user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'activation_codes_product_device_fkey') THEN
        ALTER TABLE activation_codes ADD CONSTRAINT activation_codes_product_device_fkey
            FOREIGN KEY (product, used_by_device_id) REFERENCES devices(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'auth_sessions_user_product_fkey') THEN
        ALTER TABLE auth_sessions ADD CONSTRAINT auth_sessions_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_leases_user_product_fkey') THEN
        ALTER TABLE model_leases ADD CONSTRAINT model_leases_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_leases_product_device_fkey') THEN
        ALTER TABLE model_leases ADD CONSTRAINT model_leases_product_device_fkey
            FOREIGN KEY (product, device_id) REFERENCES devices(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_leases_product_account_fkey') THEN
        ALTER TABLE model_leases ADD CONSTRAINT model_leases_product_account_fkey
            FOREIGN KEY (product, account_id) REFERENCES model_accounts(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_usage_records_user_product_fkey') THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_usage_records_product_device_fkey') THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_product_device_fkey
            FOREIGN KEY (product, device_id) REFERENCES devices(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_usage_records_product_account_fkey') THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_product_account_fkey
            FOREIGN KEY (product, account_id) REFERENCES model_accounts(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_usage_records_product_lease_fkey') THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_product_lease_fkey
            FOREIGN KEY (product, lease_id) REFERENCES model_leases(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_device_fkey') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_device_fkey
            FOREIGN KEY (product, device_id) REFERENCES devices(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_account_fkey') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_account_fkey
            FOREIGN KEY (product, account_id) REFERENCES model_accounts(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_lease_fkey') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_lease_fkey
            FOREIGN KEY (product, lease_id) REFERENCES model_leases(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_request_reservations_product_usage_fkey') THEN
        ALTER TABLE model_request_reservations ADD CONSTRAINT model_request_reservations_product_usage_fkey
            FOREIGN KEY (product, usage_id) REFERENCES model_usage_records(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'audit_logs_actor_user_product_fkey') THEN
        ALTER TABLE audit_logs ADD CONSTRAINT audit_logs_actor_user_product_fkey
            FOREIGN KEY (actor_user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'audit_logs_product_device_fkey') THEN
        ALTER TABLE audit_logs ADD CONSTRAINT audit_logs_product_device_fkey
            FOREIGN KEY (product, device_id) REFERENCES devices(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'model_pool_test_results_product_account_fkey') THEN
        ALTER TABLE model_pool_test_results ADD CONSTRAINT model_pool_test_results_product_account_fkey
            FOREIGN KEY (product, account_id) REFERENCES model_accounts(product, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'user_authorization_policies_user_product_fkey') THEN
        ALTER TABLE user_authorization_policies ADD CONSTRAINT user_authorization_policies_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'variant_tasks_user_product_fkey') THEN
        ALTER TABLE variant_tasks ADD CONSTRAINT variant_tasks_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'variant_tasks_product_device_fkey') THEN
        ALTER TABLE variant_tasks ADD CONSTRAINT variant_tasks_product_device_fkey
            FOREIGN KEY (product, device_id) REFERENCES devices(product, id);
    END IF;
END
$$;

CREATE UNIQUE INDEX IF NOT EXISTS uq_model_usage_product_lease_client_call
    ON model_usage_records (product, lease_id, client_call_id)
    WHERE client_call_id <> '';

CREATE INDEX IF NOT EXISTS idx_user_products_product_status
    ON user_products (product, status, user_id);
CREATE INDEX IF NOT EXISTS idx_user_products_user_status
    ON user_products (user_id, status, product);
CREATE INDEX IF NOT EXISTS idx_devices_product_device_id
    ON devices (product, id);
CREATE INDEX IF NOT EXISTS idx_devices_product_user_id
    ON devices (product, user_id);
CREATE INDEX IF NOT EXISTS idx_activation_codes_product_status
    ON activation_codes (product, status, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_product_device
    ON auth_sessions (product, device_id, revoked_at)
    WHERE device_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_auth_sessions_product_user_active
    ON auth_sessions (product, user_id, revoked_at);
CREATE INDEX IF NOT EXISTS idx_model_accounts_product_provider_model_status
    ON model_accounts (product, provider, model, status);
CREATE INDEX IF NOT EXISTS idx_model_leases_product_user_status_expiry
    ON model_leases (product, user_id, status, expires_at, id);
CREATE INDEX IF NOT EXISTS idx_model_leases_product_device_status_expiry
    ON model_leases (product, device_id, status, expires_at, id);
CREATE INDEX IF NOT EXISTS idx_model_usage_product_user_created_at
    ON model_usage_records (product, user_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_model_request_reservations_product_status
    ON model_request_reservations (product, status, started_at, scope);
CREATE INDEX IF NOT EXISTS idx_audit_logs_product_created_at
    ON audit_logs (product, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_product_device_created_at
    ON audit_logs (product, device_id, created_at DESC, id DESC)
    WHERE device_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_audit_outbox_product_dispatch
    ON audit_outbox (product, status, next_attempt_at, created_at, id);
CREATE INDEX IF NOT EXISTS idx_model_pool_test_results_product_account_created_at
    ON model_pool_test_results (product, account_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_user_authorization_policies_product_user
    ON user_authorization_policies (product, user_id);
CREATE INDEX IF NOT EXISTS idx_idempotency_records_product_created_at
    ON idempotency_records (product, created_at, scope, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_variant_tasks_product_user_updated_at
    ON variant_tasks (product, user_id, updated_at DESC, id DESC);

-- 产品 code 不能被更新；停用产品使用 status，不复用 code。
CREATE OR REPLACE FUNCTION autolive_products_code_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.code IS DISTINCT FROM OLD.code THEN
        RAISE EXCEPTION 'product code is immutable';
    END IF;
    RETURN NEW;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_trigger
        WHERE tgrelid = 'products'::regclass
          AND tgname = 'products_code_immutable'
          AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER products_code_immutable
            BEFORE UPDATE OF code ON products
            FOR EACH ROW
            EXECUTE FUNCTION autolive_products_code_immutable();
    END IF;
END
$$;
