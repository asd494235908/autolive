-- douyin-desktop 用户资产同步只保存显式白名单 JSON 事实与幂等回执。
-- user/product/device 作用域由现有规范化身份表提供，客户端请求体不能创建或替换作用域。

CREATE TABLE IF NOT EXISTS client_sync_workspaces (
    product TEXT NOT NULL REFERENCES products(code),
    user_id TEXT NOT NULL,
    current_revision BIGINT NOT NULL DEFAULT 0 CHECK (current_revision >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (product, user_id),
    CONSTRAINT client_sync_workspaces_user_product_fkey
        FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS client_sync_items (
    product TEXT NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    item_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    payload_jsonb JSONB NOT NULL DEFAULT '{}'::jsonb,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    updated_by_device_id TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (product, user_id, kind, item_id),
    CONSTRAINT client_sync_items_workspace_fkey
        FOREIGN KEY (product, user_id)
        REFERENCES client_sync_workspaces(product, user_id) ON DELETE CASCADE,
    CONSTRAINT client_sync_items_kind_check CHECK (
        kind IN (
            'persona_version', 'policy_version', 'model_config',
            'knowledge_source', 'knowledge_document', 'knowledge_chunk',
            'knowledge_rule', 'memory'
        )
    ),
    CONSTRAINT client_sync_items_payload_size_check
        CHECK (octet_length(payload_jsonb::text) <= 262144)
);

CREATE INDEX IF NOT EXISTS idx_client_sync_items_revision
    ON client_sync_items (product, user_id, revision);

CREATE TABLE IF NOT EXISTS client_sync_mutations (
    product TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    mutation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^[0-9a-f]{64}$'),
    response_jsonb JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (product, user_id, device_id, mutation_id),
    CONSTRAINT client_sync_mutations_user_product_fkey
        FOREIGN KEY (user_id, product) REFERENCES user_products(user_id, product) ON DELETE CASCADE,
    CONSTRAINT client_sync_mutations_response_size_check
        CHECK (octet_length(response_jsonb::text) <= 262144)
);

CREATE INDEX IF NOT EXISTS idx_client_sync_mutations_created_at
    ON client_sync_mutations (created_at);
