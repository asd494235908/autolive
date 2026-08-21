-- 审计事件先持久化为可重试 Outbox，再投递到 audit_logs。
-- dedupe_key 由请求 ID 驱动，避免 fail-closed 重试产生重复审计事实。
CREATE TABLE IF NOT EXISTS audit_outbox (
    id TEXT PRIMARY KEY,
    dedupe_key TEXT NOT NULL UNIQUE,
    payload JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'processing', 'sent')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    delivered_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_audit_outbox_dispatch
    ON audit_outbox (status, next_attempt_at, created_at, id);
