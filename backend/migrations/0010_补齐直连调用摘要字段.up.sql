-- 当前版本的 Go 控制面只记录 Rust 直连 OpenAI-compatible 调用摘要，不接收正文。
ALTER TABLE model_usage_records
    ADD COLUMN IF NOT EXISTS client_call_id TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS usage_source TEXT NOT NULL DEFAULT 'legacy';

CREATE INDEX IF NOT EXISTS idx_usage_client_call_id ON model_usage_records(client_call_id);
