-- 冷却时间由服务端状态机写入；NULL 表示没有自动冷却窗口。
ALTER TABLE model_accounts
    ADD COLUMN IF NOT EXISTS cooldown_until TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS model_accounts_cooldown_until_idx
    ON model_accounts (cooldown_until)
    WHERE cooldown_until IS NOT NULL;
