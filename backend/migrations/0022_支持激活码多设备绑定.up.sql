-- 为激活码补齐可绑定设备上限和当前绑定数量，保留首次核销归属字段兼容旧数据。
ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS max_devices INTEGER NOT NULL DEFAULT 1;

ALTER TABLE activation_codes
    ADD COLUMN IF NOT EXISTS bound_devices INTEGER NOT NULL DEFAULT 0;

UPDATE activation_codes
SET bound_devices = 1
WHERE bound_devices = 0
  AND used_by_device_id IS NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'activation_codes_max_devices_range'
    ) THEN
        ALTER TABLE activation_codes
            ADD CONSTRAINT activation_codes_max_devices_range
                CHECK (max_devices BETWEEN 1 AND 100);
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'activation_codes_bound_devices_range'
    ) THEN
        ALTER TABLE activation_codes
            ADD CONSTRAINT activation_codes_bound_devices_range
                CHECK (bound_devices BETWEEN 0 AND max_devices);
    END IF;
END
$$;
