-- 规范化读源需要保留设备展示字段和最近一次模型测试结果，避免回退到 JSONB 快照。
ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS device_name TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS platform TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS model_pool_test_results (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES model_accounts(id),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_model_pool_test_results_account_created_at
    ON model_pool_test_results (account_id, created_at DESC);
