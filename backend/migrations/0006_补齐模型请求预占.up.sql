-- 模型请求预占独立保存幂等、额度预占和未知状态恢复所需的元数据。
-- 结果内容只作为服务端审计/重放数据保存，不保存模型 API Key。
CREATE TABLE IF NOT EXISTS model_request_reservations (
    scope TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    status TEXT NOT NULL,
    usage_id TEXT NOT NULL REFERENCES model_usage_records(id),
    task_id TEXT NOT NULL,
    variant_id TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL REFERENCES users(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    lease_id TEXT NOT NULL REFERENCES model_leases(id),
    account_id TEXT NOT NULL REFERENCES model_accounts(id),
    request_id TEXT NOT NULL UNIQUE,
    reserved_tokens INTEGER NOT NULL CHECK (reserved_tokens >= 0),
    daily_key TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    error_code TEXT,
    error_message TEXT,
    error_status INTEGER NOT NULL DEFAULT 0 CHECK (error_status >= 0),
    result JSONB
);

CREATE INDEX IF NOT EXISTS idx_model_request_reservations_status
    ON model_request_reservations(status, started_at);
