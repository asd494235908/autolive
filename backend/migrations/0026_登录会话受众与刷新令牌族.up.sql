-- 登录会话按管理端/桌面端隔离，并保留已消费 Refresh Token 以检测重放。
ALTER TABLE auth_sessions
    ADD COLUMN IF NOT EXISTS audience TEXT,
    ADD COLUMN IF NOT EXISTS token_family_id TEXT,
    ADD COLUMN IF NOT EXISTS generation INTEGER,
    ADD COLUMN IF NOT EXISTS consumed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS revoked_reason TEXT,
    ADD COLUMN IF NOT EXISTS rotated_to_session_id TEXT,
    ADD COLUMN IF NOT EXISTS last_used_at TIMESTAMPTZ;

UPDATE auth_sessions
SET audience = COALESCE(audience, 'legacy'),
    token_family_id = COALESCE(token_family_id, id),
    generation = COALESCE(generation, 0),
    revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP),
    revoked_reason = COALESCE(revoked_reason, 'legacy_migration');

ALTER TABLE auth_sessions
    ALTER COLUMN audience SET NOT NULL,
    ALTER COLUMN token_family_id SET NOT NULL,
    ALTER COLUMN generation SET NOT NULL,
    ADD CONSTRAINT auth_sessions_audience_check
        CHECK (audience IN ('admin', 'desktop', 'legacy')),
    ADD CONSTRAINT auth_sessions_generation_check
        CHECK (generation >= 0),
    ADD CONSTRAINT auth_sessions_rotation_state_check
        CHECK (
            (consumed_at IS NULL AND rotated_to_session_id IS NULL)
            OR
            (consumed_at IS NOT NULL AND revoked_at IS NOT NULL AND revoked_reason = 'rotated' AND rotated_to_session_id IS NOT NULL)
        );

CREATE UNIQUE INDEX IF NOT EXISTS uq_auth_sessions_token_family_generation
    ON auth_sessions (token_family_id, generation);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_refresh_replay_lookup
    ON auth_sessions (refresh_token_hash, revoked_at, consumed_at);
