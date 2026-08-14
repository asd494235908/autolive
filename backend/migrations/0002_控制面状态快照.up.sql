CREATE TABLE IF NOT EXISTS control_plane_state (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id = TRUE),
    state JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

INSERT INTO control_plane_state (id, state, updated_at)
VALUES (TRUE, '{"version":1}'::jsonb, CURRENT_TIMESTAMP)
ON CONFLICT (id) DO NOTHING;
