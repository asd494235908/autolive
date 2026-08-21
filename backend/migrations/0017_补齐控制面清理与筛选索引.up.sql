-- 为有界保留清理和管理端固定筛选补齐索引。
-- 仅新增可重复执行的索引，不改变历史事实或业务状态。
CREATE INDEX IF NOT EXISTS idx_auth_sessions_refresh_expiry_active
    ON auth_sessions (refresh_expires_at)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user_active
    ON auth_sessions (user_id, revoked_at);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_device_active
    ON auth_sessions (device_id, revoked_at)
    WHERE device_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_idempotency_records_created_at
    ON idempotency_records (created_at, scope, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_model_pool_test_results_created_at
    ON model_pool_test_results (created_at, id);

CREATE INDEX IF NOT EXISTS idx_model_leases_account_status_expiry
    ON model_leases (account_id, status, expires_at);

CREATE INDEX IF NOT EXISTS idx_model_accounts_provider_model_status
    ON model_accounts (provider, model, status);

CREATE INDEX IF NOT EXISTS idx_model_usage_account_created_at
    ON model_usage_records (account_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_logs_request_created_at
    ON audit_logs (request_id, created_at DESC)
    WHERE request_id IS NOT NULL;
