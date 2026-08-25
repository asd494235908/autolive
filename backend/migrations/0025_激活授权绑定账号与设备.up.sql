-- 激活授权改为创建时绑定账号；历史核销字段继续保留为审计事实。
ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS bound_user_id TEXT;

UPDATE activation_codes
SET bound_user_id = used_by_user_id
WHERE bound_user_id IS NULL
  AND used_by_user_id IS NOT NULL;

-- 过期状态先固化；无法可靠推断账号的旧有效码安全作废，禁止伪造归属。
UPDATE activation_codes
SET status = 'expired'
WHERE expires_at IS NOT NULL
  AND expires_at <= CURRENT_TIMESTAMP
  AND status IN ('active', 'used');

UPDATE activation_codes
SET status = 'revoked'
WHERE bound_user_id IS NULL
  AND status IN ('active', 'used');

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'activation_codes'::regclass
          AND conname = 'activation_codes_bound_user_product_fkey'
    ) THEN
        ALTER TABLE activation_codes
            ADD CONSTRAINT activation_codes_bound_user_product_fkey
            FOREIGN KEY (bound_user_id, product)
            REFERENCES user_products(user_id, product);
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'activation_codes'::regclass
          AND conname = 'activation_codes_bound_user_required_for_live_code'
    ) THEN
        ALTER TABLE activation_codes
            ADD CONSTRAINT activation_codes_bound_user_required_for_live_code
            CHECK (bound_user_id IS NOT NULL OR status IN ('revoked', 'expired'));
    END IF;
END
$$;

-- 复合唯一键为下方跨表归属外键提供数据库级事实源，阻止旁路写入串号。
CREATE UNIQUE INDEX IF NOT EXISTS uq_activation_codes_binding_identity
    ON activation_codes (id, product, bound_user_id);

CREATE UNIQUE INDEX IF NOT EXISTS uq_devices_binding_identity
    ON devices (id, product, user_id);

CREATE TABLE IF NOT EXISTS activation_device_bindings (
    activation_code_id TEXT NOT NULL,
    device_id TEXT PRIMARY KEY,
    product TEXT NOT NULL REFERENCES products(code),
    user_id TEXT NOT NULL,
    bound_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT activation_device_bindings_user_product_fkey
        FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product),
    CONSTRAINT activation_device_bindings_code_owner_fkey
        FOREIGN KEY (activation_code_id, product, user_id)
        REFERENCES activation_codes(id, product, bound_user_id) ON DELETE RESTRICT,
    CONSTRAINT activation_device_bindings_device_owner_fkey
        FOREIGN KEY (device_id, product, user_id)
        REFERENCES devices(id, product, user_id) ON DELETE RESTRICT
);

-- 旧结构只能可靠恢复首次核销设备；其余历史计数继续保留，不猜测设备归属。
WITH unique_historical_devices AS (
    SELECT used_by_device_id
    FROM activation_codes
    WHERE bound_user_id IS NOT NULL
      AND used_by_device_id IS NOT NULL
    GROUP BY used_by_device_id
    HAVING COUNT(*) = 1
)
INSERT INTO activation_device_bindings (activation_code_id, device_id, product, user_id, bound_at)
SELECT ac.id, ac.used_by_device_id, ac.product, ac.bound_user_id, COALESCE(ac.used_at, ac.created_at)
FROM activation_codes AS ac
JOIN unique_historical_devices AS unique_device
  ON unique_device.used_by_device_id = ac.used_by_device_id
JOIN devices AS d
  ON d.id = ac.used_by_device_id
 AND d.user_id = ac.bound_user_id
 AND d.product = ac.product
WHERE ac.bound_user_id IS NOT NULL
  AND ac.used_by_device_id IS NOT NULL
ON CONFLICT (device_id) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_activation_codes_account_capacity
    ON activation_codes (product, bound_user_id, expires_at, id)
    WHERE status IN ('active', 'used');

CREATE INDEX IF NOT EXISTS idx_activation_device_bindings_code
    ON activation_device_bindings (activation_code_id, device_id);

CREATE INDEX IF NOT EXISTS idx_activation_device_bindings_account
    ON activation_device_bindings (product, user_id, device_id);
