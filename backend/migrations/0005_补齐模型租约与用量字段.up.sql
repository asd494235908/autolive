-- State 中的租约和代理用量包含更多审计字段，补齐规范化影子表的表达能力。
ALTER TABLE model_leases
    ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS model TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS proxy_mode TEXT NOT NULL DEFAULT 'server_proxy',
    ADD COLUMN IF NOT EXISTS concurrency_limit INTEGER NOT NULL DEFAULT 1 CHECK (concurrency_limit > 0);

ALTER TABLE model_usage_records
    ADD COLUMN IF NOT EXISTS request_id TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS total_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_tokens >= 0),
    ADD COLUMN IF NOT EXISTS latency_ms BIGINT NOT NULL DEFAULT 0 CHECK (latency_ms >= 0);
