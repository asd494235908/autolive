-- 为管理端 normalized 查询的常用等值筛选补齐复合索引。
-- 索引只改善查询路径，不改变控制面事实；具体执行计划仍需在发布前
-- 使用真实数据量运行 EXPLAIN (ANALYZE, BUFFERS) 复核。
CREATE INDEX IF NOT EXISTS idx_model_leases_user_status_expiry
    ON model_leases (user_id, status, expires_at, id);

CREATE INDEX IF NOT EXISTS idx_model_leases_device_status_expiry
    ON model_leases (device_id, status, expires_at, id);

CREATE INDEX IF NOT EXISTS idx_model_leases_provider_model_status_expiry
    ON model_leases (provider, model, status, expires_at, id);

CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_created_at
    ON audit_logs (actor_user_id, created_at DESC, id DESC)
    WHERE actor_user_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_audit_logs_device_created_at
    ON audit_logs (device_id, created_at DESC, id DESC)
    WHERE device_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_audit_logs_action_created_at
    ON audit_logs (action, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_created_at
    ON audit_logs (resource_type, resource_id, created_at DESC, id DESC);
