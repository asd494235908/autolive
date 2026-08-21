-- 迁移边界：保留激活码首次核销时的用户归属，避免设备解绑后详情丢失历史主体。
ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS used_by_user_id TEXT REFERENCES users(id);

UPDATE activation_codes AS ac
SET used_by_user_id = d.user_id
FROM devices AS d
WHERE ac.used_by_device_id = d.id
  AND ac.used_by_user_id IS NULL
  AND d.user_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_activation_codes_used_by_user_id
    ON activation_codes (used_by_user_id);
