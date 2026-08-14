-- State 中已有优先级字段；补齐规范化模型账号表，保持影子写入字段可表达。
ALTER TABLE model_accounts
    ADD COLUMN IF NOT EXISTS priority INTEGER NOT NULL DEFAULT 0 CHECK (priority >= 0);
