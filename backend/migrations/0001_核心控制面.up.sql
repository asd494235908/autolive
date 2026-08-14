-- 迁移边界：本文件只定义生产控制面元数据，不保存模型 API Key 或明文激活码。
-- 执行前请使用受控迁移工具/发布流程，并在目标 PostgreSQL 上完成备份与回滚演练。

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    disabled_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id),
    device_key TEXT NOT NULL UNIQUE,
    client_version TEXT NOT NULL,
    status TEXT NOT NULL,
    disk_free_bytes BIGINT NOT NULL DEFAULT 0 CHECK (disk_free_bytes >= 0),
    memory_total_bytes BIGINT NOT NULL DEFAULT 0 CHECK (memory_total_bytes >= 0),
    memory_available_bytes BIGINT NOT NULL DEFAULT 0 CHECK (memory_available_bytes >= 0 AND memory_available_bytes <= memory_total_bytes),
    cpu_logical_cores INTEGER NOT NULL DEFAULT 0 CHECK (cpu_logical_cores BETWEEN 0 AND 4096),
    runtime_os_name TEXT,
    runtime_os_version TEXT,
    kernel_version TEXT,
    last_heartbeat_at TIMESTAMPTZ,
    current_media_name TEXT,
    playback_state TEXT
);

CREATE TABLE IF NOT EXISTS activation_codes (
    id TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL UNIQUE,
    code_prefix TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    used_at TIMESTAMPTZ,
    used_by_device_id TEXT REFERENCES devices(id)
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    device_id TEXT REFERENCES devices(id),
    access_token_hash TEXT NOT NULL UNIQUE,
    refresh_token_hash TEXT NOT NULL UNIQUE,
    access_expires_at TIMESTAMPTZ NOT NULL,
    refresh_expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS model_accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    base_url TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    status TEXT NOT NULL,
    concurrency_limit INTEGER NOT NULL CHECK (concurrency_limit > 0),
    daily_token_limit INTEGER NOT NULL DEFAULT 0 CHECK (daily_token_limit >= 0),
    active_requests INTEGER NOT NULL DEFAULT 0 CHECK (active_requests >= 0),
    daily_reserved_tokens INTEGER NOT NULL DEFAULT 0 CHECK (daily_reserved_tokens >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

-- 只保存密文；加密主密钥由部署环境注入，不进入数据库。
CREATE TABLE IF NOT EXISTS model_account_secrets (
    secret_ref TEXT PRIMARY KEY,
    ciphertext BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS model_leases (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES model_accounts(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    purpose TEXT NOT NULL,
    status TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    released_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS model_usage_records (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES model_accounts(id),
    lease_id TEXT REFERENCES model_leases(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0 CHECK (prompt_tokens >= 0),
    completion_tokens INTEGER NOT NULL DEFAULT 0 CHECK (completion_tokens >= 0),
    status TEXT NOT NULL,
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS variant_tasks (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    source_sha256 TEXT NOT NULL,
    source_duration_ms BIGINT NOT NULL CHECK (source_duration_ms > 0),
    target_duration_ms BIGINT NOT NULL CHECK (target_duration_ms > 0),
    variant_count INTEGER NOT NULL CHECK (variant_count > 0),
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS variant_task_events (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES variant_tasks(id),
    variant_id TEXT NOT NULL DEFAULT '',
    stage TEXT NOT NULL,
    status TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (task_id, variant_id, stage, operation_id)
);

CREATE TABLE IF NOT EXISTS idempotency_records (
    scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (scope, idempotency_key)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT REFERENCES users(id),
    device_id TEXT REFERENCES devices(id),
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    request_id TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_devices_user_id ON devices(user_id);
CREATE INDEX IF NOT EXISTS idx_devices_last_heartbeat_at ON devices(last_heartbeat_at);
CREATE INDEX IF NOT EXISTS idx_model_leases_active_expiry ON model_leases(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_usage_user_created_at ON model_usage_records(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_variant_tasks_user_updated_at ON variant_tasks(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at);
