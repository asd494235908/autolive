-- 为跨存储激活/心跳补偿记录绑定时间；定时协调器据此清理崩溃遗留的孤立绑定。
ALTER TABLE auth_sessions
    ADD COLUMN IF NOT EXISTS device_bound_at TIMESTAMPTZ;

-- 激活流程先写会话绑定、后创建设备；该临时状态不能被设备外键拒绝。
-- 绑定仍由已认证主体校验，且由下方协调清理回收不存在的设备引用。
DO $$
DECLARE
    foreign_key_name TEXT;
BEGIN
    SELECT conname INTO foreign_key_name
    FROM pg_constraint
    WHERE conrelid = 'auth_sessions'::regclass
      AND confrelid = 'devices'::regclass
      AND contype = 'f'
    LIMIT 1;
    IF foreign_key_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE auth_sessions DROP CONSTRAINT %I', foreign_key_name);
    END IF;
END $$;

UPDATE auth_sessions
SET device_bound_at = created_at
WHERE device_id IS NOT NULL
  AND device_bound_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_auth_sessions_orphan_device_binding
    ON auth_sessions (device_bound_at, device_id)
    WHERE revoked_at IS NULL AND device_id IS NOT NULL AND device_bound_at IS NOT NULL;
