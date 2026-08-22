-- 0024 建立管理员 RBAC 的规范化事实表。
-- product 为空只允许表示全局 super_admin；普通角色与绑定都必须落在具体产品上。

CREATE TABLE IF NOT EXISTS admin_permissions (
    code TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT admin_permissions_code_not_blank CHECK (btrim(code) <> '')
);

CREATE TABLE IF NOT EXISTS admin_roles (
    code TEXT PRIMARY KEY,
    product TEXT,
    name TEXT NOT NULL,
    built_in BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT admin_roles_code_not_blank CHECK (btrim(code) <> ''),
    CONSTRAINT admin_roles_name_not_blank CHECK (btrim(name) <> ''),
    CONSTRAINT admin_roles_product_fkey FOREIGN KEY (product) REFERENCES products(code),
    CONSTRAINT admin_roles_scope_check CHECK (
        (code = 'super_admin' AND product IS NULL AND built_in = TRUE) OR
        (code <> 'super_admin' AND product IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS admin_role_permissions (
    role_code TEXT NOT NULL,
    permission_code TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (role_code, permission_code),
    CONSTRAINT admin_role_permissions_role_fkey
        FOREIGN KEY (role_code) REFERENCES admin_roles(code) ON DELETE CASCADE,
    CONSTRAINT admin_role_permissions_permission_fkey
        FOREIGN KEY (permission_code) REFERENCES admin_permissions(code) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS user_admin_roles (
    user_id TEXT NOT NULL,
    role_code TEXT NOT NULL,
    product TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT user_admin_roles_user_fkey
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CONSTRAINT user_admin_roles_role_fkey
        FOREIGN KEY (role_code) REFERENCES admin_roles(code) ON DELETE RESTRICT,
    CONSTRAINT user_admin_roles_product_fkey
        FOREIGN KEY (product) REFERENCES products(code),
    CONSTRAINT user_admin_roles_scope_check CHECK (
        (role_code = 'super_admin' AND product IS NULL) OR
        (role_code <> 'super_admin' AND product IS NOT NULL)
    ),
    CONSTRAINT user_admin_roles_user_product_fkey
        FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product)
);

CREATE OR REPLACE FUNCTION autolive_validate_admin_role_assignment()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    role_product TEXT;
BEGIN
    SELECT product
    INTO role_product
    FROM admin_roles
    WHERE code = NEW.role_code;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'admin role % does not exist', NEW.role_code;
    END IF;

    IF NEW.role_code = 'super_admin' THEN
        IF NEW.product IS NOT NULL THEN
            RAISE EXCEPTION 'super_admin assignment must be global';
        END IF;
    ELSE
        IF NEW.product IS NULL THEN
            RAISE EXCEPTION 'product-scoped admin assignment requires product';
        END IF;
        IF role_product IS NULL OR role_product <> NEW.product THEN
            RAISE EXCEPTION 'admin role assignment product must match role scope';
        END IF;
    END IF;

    RETURN NEW;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_trigger
        WHERE tgrelid = 'user_admin_roles'::regclass
          AND tgname = 'autolive_validate_admin_role_assignment'
          AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER autolive_validate_admin_role_assignment
            BEFORE INSERT OR UPDATE ON user_admin_roles
            FOR EACH ROW
            EXECUTE FUNCTION autolive_validate_admin_role_assignment();
    END IF;
END
$$;

INSERT INTO admin_permissions (code, created_at)
VALUES
    ('dashboard.read', CURRENT_TIMESTAMP),
    ('users.read', CURRENT_TIMESTAMP),
    ('users.manage', CURRENT_TIMESTAMP),
    ('roles.read', CURRENT_TIMESTAMP),
    ('roles.manage', CURRENT_TIMESTAMP),
    ('roles.assign', CURRENT_TIMESTAMP),
    ('admin_security.manage', CURRENT_TIMESTAMP),
    ('devices.read', CURRENT_TIMESTAMP),
    ('devices.manage', CURRENT_TIMESTAMP),
    ('activation_codes.read', CURRENT_TIMESTAMP),
    ('activation_codes.manage', CURRENT_TIMESTAMP),
    ('activation_codes.reveal', CURRENT_TIMESTAMP),
    ('activation_codes.switch_device', CURRENT_TIMESTAMP),
    ('plans.read', CURRENT_TIMESTAMP),
    ('plans.manage', CURRENT_TIMESTAMP),
    ('plans.publish', CURRENT_TIMESTAMP),
    ('orders.read', CURRENT_TIMESTAMP),
    ('orders.reconcile', CURRENT_TIMESTAMP),
    ('subscriptions.read', CURRENT_TIMESTAMP),
    ('subscriptions.adjust', CURRENT_TIMESTAMP),
    ('payments.read', CURRENT_TIMESTAMP),
    ('payments.reconcile', CURRENT_TIMESTAMP),
    ('password_resets.read', CURRENT_TIMESTAMP),
    ('password_resets.retry', CURRENT_TIMESTAMP),
    ('artifacts.read', CURRENT_TIMESTAMP),
    ('artifacts.manage', CURRENT_TIMESTAMP),
    ('artifacts.publish', CURRENT_TIMESTAMP),
    ('artifacts.revoke', CURRENT_TIMESTAMP),
    ('public_config.read', CURRENT_TIMESTAMP),
    ('public_config.manage', CURRENT_TIMESTAMP),
    ('public_config.publish', CURRENT_TIMESTAMP),
    ('public_config.rollback', CURRENT_TIMESTAMP),
    ('error_reports.read', CURRENT_TIMESTAMP),
    ('error_reports.manage', CURRENT_TIMESTAMP),
    ('feedback.read', CURRENT_TIMESTAMP),
    ('feedback.manage', CURRENT_TIMESTAMP),
    ('model_pool.read', CURRENT_TIMESTAMP),
    ('model_pool.manage', CURRENT_TIMESTAMP),
    ('model_pool.test', CURRENT_TIMESTAMP),
    ('model_pool.rotate_secret', CURRENT_TIMESTAMP),
    ('model_leases.read', CURRENT_TIMESTAMP),
    ('model_leases.reclaim', CURRENT_TIMESTAMP),
    ('model_usage.read', CURRENT_TIMESTAMP),
    ('audit_logs.read', CURRENT_TIMESTAMP),
    ('operations.read', CURRENT_TIMESTAMP),
    ('operations.manage', CURRENT_TIMESTAMP)
ON CONFLICT (code) DO NOTHING;

INSERT INTO admin_roles (code, product, name, built_in, created_at, updated_at)
VALUES ('super_admin', NULL, '超级管理员', TRUE, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
ON CONFLICT (code) DO UPDATE
SET product = EXCLUDED.product,
    name = EXCLUDED.name,
    built_in = EXCLUDED.built_in,
    updated_at = EXCLUDED.updated_at;

INSERT INTO admin_role_permissions (role_code, permission_code, created_at)
SELECT 'super_admin', code, CURRENT_TIMESTAMP
FROM admin_permissions
ON CONFLICT (role_code, permission_code) DO NOTHING;

-- 活动 users.role = 'admin' 回填到全局 super_admin。
INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
SELECT users.id, 'super_admin', NULL, CURRENT_TIMESTAMP
FROM users
WHERE users.role = 'admin'
  AND users.status = 'active'
  AND NOT EXISTS (
      SELECT 1
      FROM user_admin_roles
      WHERE user_admin_roles.user_id = users.id
        AND user_admin_roles.role_code = 'super_admin'
        AND user_admin_roles.product IS NULL
  );

-- users.role = 'user' 不回填任何管理员角色。

INSERT INTO user_admin_roles (user_id, role_code, product, created_at)
SELECT users.id, 'super_admin', NULL, CURRENT_TIMESTAMP
FROM users
WHERE users.id = 'usr_local_admin'
  AND NOT EXISTS (
      SELECT 1
      FROM user_admin_roles
      WHERE user_admin_roles.user_id = users.id
        AND user_admin_roles.role_code = 'super_admin'
        AND user_admin_roles.product IS NULL
  );

CREATE INDEX IF NOT EXISTS idx_admin_roles_product
    ON admin_roles (product, code);
CREATE INDEX IF NOT EXISTS idx_admin_role_permissions_permission
    ON admin_role_permissions (permission_code, role_code);
CREATE INDEX IF NOT EXISTS idx_user_admin_roles_product_user
    ON user_admin_roles (product, user_id, role_code);
CREATE UNIQUE INDEX IF NOT EXISTS uq_user_admin_roles_binding
    ON user_admin_roles (user_id, role_code, COALESCE(product, ''));
