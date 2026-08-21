-- 用户模型授权策略。allowed_models 为空数组表示不限制已登记模型。
-- daily_token_limit 只约束服务端已接收的 usage 记录，不是供应商权威计费或预占。
CREATE TABLE IF NOT EXISTS user_authorization_policies (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    allowed_models JSONB NOT NULL DEFAULT '[]'::jsonb,
    daily_token_limit BIGINT NOT NULL DEFAULT 0 CHECK (daily_token_limit >= 0 AND daily_token_limit <= 1000000000),
    updated_at TIMESTAMPTZ NOT NULL
);
