-- 0023 只做多产品兼容扩展，不提前收紧旧键或旧写路径。
-- 真正的 product-aware 主键/外键与 NOT NULL 收紧必须等 Task 3-5
-- 完成 product 传播后再用后续迁移执行。

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

INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
SELECT id, 'autolive', 'active', 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
FROM users
ON CONFLICT (user_id, product) DO NOTHING;

CREATE OR REPLACE FUNCTION autolive_seed_default_user_product()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
    VALUES (
        NEW.id,
        'autolive',
        'active',
        0,
        COALESCE(NEW.created_at, CURRENT_TIMESTAMP),
        COALESCE(NEW.created_at, CURRENT_TIMESTAMP)
    )
    ON CONFLICT (user_id, product) DO NOTHING;
    RETURN NEW;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_trigger
        WHERE tgrelid = 'users'::regclass
          AND tgname = 'autolive_seed_default_user_product'
          AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER autolive_seed_default_user_product
            AFTER INSERT ON users
            FOR EACH ROW
            EXECUTE FUNCTION autolive_seed_default_user_product();
    END IF;
END
$$;

ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE auth_sessions
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE model_accounts
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE model_leases
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE model_usage_records
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE model_request_reservations
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE audit_outbox
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE model_pool_test_results
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE user_authorization_policies
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE idempotency_records
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';
ALTER TABLE variant_tasks
    ADD COLUMN IF NOT EXISTS product TEXT DEFAULT 'autolive';

ALTER TABLE devices ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE activation_codes ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE auth_sessions ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE model_accounts ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE model_leases ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE model_usage_records ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE model_request_reservations ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE audit_logs ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE audit_outbox ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE model_pool_test_results ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE user_authorization_policies ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE idempotency_records ALTER COLUMN product SET DEFAULT 'autolive';
ALTER TABLE variant_tasks ALTER COLUMN product SET DEFAULT 'autolive';

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

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'devices'::regclass AND conname = 'devices_user_product_fkey'
    ) THEN
        ALTER TABLE devices ADD CONSTRAINT devices_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'activation_codes'::regclass AND conname = 'activation_codes_used_by_user_product_fkey'
    ) THEN
        ALTER TABLE activation_codes ADD CONSTRAINT activation_codes_used_by_user_product_fkey
            FOREIGN KEY (used_by_user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'auth_sessions'::regclass AND conname = 'auth_sessions_user_product_fkey'
    ) THEN
        ALTER TABLE auth_sessions ADD CONSTRAINT auth_sessions_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'model_leases'::regclass AND conname = 'model_leases_user_product_fkey'
    ) THEN
        ALTER TABLE model_leases ADD CONSTRAINT model_leases_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'model_usage_records'::regclass AND conname = 'model_usage_records_user_product_fkey'
    ) THEN
        ALTER TABLE model_usage_records ADD CONSTRAINT model_usage_records_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'audit_logs'::regclass AND conname = 'audit_logs_actor_user_product_fkey'
    ) THEN
        ALTER TABLE audit_logs ADD CONSTRAINT audit_logs_actor_user_product_fkey
            FOREIGN KEY (actor_user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'user_authorization_policies'::regclass AND conname = 'user_authorization_policies_user_product_fkey'
    ) THEN
        ALTER TABLE user_authorization_policies ADD CONSTRAINT user_authorization_policies_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'variant_tasks'::regclass AND conname = 'variant_tasks_user_product_fkey'
    ) THEN
        ALTER TABLE variant_tasks ADD CONSTRAINT variant_tasks_user_product_fkey
            FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product);
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS idx_user_products_product_status
    ON user_products (product, status, user_id);
CREATE INDEX IF NOT EXISTS idx_user_products_user_status
    ON user_products (user_id, status, product);
CREATE INDEX IF NOT EXISTS idx_devices_product
    ON devices (product, id);
CREATE INDEX IF NOT EXISTS idx_activation_codes_product_status
    ON activation_codes (product, status, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_product
    ON auth_sessions (product, user_id, revoked_at, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_model_accounts_product_status
    ON model_accounts (product, status, provider, model);
CREATE INDEX IF NOT EXISTS idx_model_leases_product_status_expiry
    ON model_leases (product, status, expires_at, id);
CREATE INDEX IF NOT EXISTS idx_model_usage_records_product_created_at
    ON model_usage_records (product, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_model_request_reservations_product_status
    ON model_request_reservations (product, status, started_at, scope);
CREATE INDEX IF NOT EXISTS idx_audit_logs_product_created_at
    ON audit_logs (product, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_audit_outbox_product_dispatch
    ON audit_outbox (product, status, next_attempt_at, created_at, id);
CREATE INDEX IF NOT EXISTS idx_model_pool_test_results_product_created_at
    ON model_pool_test_results (product, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_user_authorization_policies_product_user
    ON user_authorization_policies (product, user_id);
CREATE INDEX IF NOT EXISTS idx_idempotency_records_product_created_at
    ON idempotency_records (product, created_at, scope, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_variant_tasks_product_updated_at
    ON variant_tasks (product, updated_at DESC, id DESC);

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
