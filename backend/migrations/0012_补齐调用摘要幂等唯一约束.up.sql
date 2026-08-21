-- 客户端调用摘要的幂等事实由租约和客户端调用 ID 共同确定。
-- 先拒绝历史重复数据，避免迁移过程中静默删除或合并账单摘要。
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM model_usage_records
        WHERE client_call_id <> ''
        GROUP BY lease_id, client_call_id
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION 'duplicate model usage client_call_id facts exist; reconcile before migration 0012';
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS uq_model_usage_lease_client_call
    ON model_usage_records (lease_id, client_call_id)
    WHERE client_call_id <> '';
