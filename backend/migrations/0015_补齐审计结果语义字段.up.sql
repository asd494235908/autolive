-- 审计结果字段不保存请求正文；失败细节只保留稳定错误码。
ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS outcome TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS status_code INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS error_code TEXT;

ALTER TABLE audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_outcome_check;

ALTER TABLE audit_logs
    ADD CONSTRAINT audit_logs_outcome_check CHECK (outcome IN ('success', 'failure', 'unknown'));

ALTER TABLE audit_logs
    ADD CONSTRAINT audit_logs_status_code_check CHECK (status_code BETWEEN 0 AND 599);

CREATE INDEX IF NOT EXISTS idx_audit_logs_outcome_created_at
    ON audit_logs(outcome, created_at);
