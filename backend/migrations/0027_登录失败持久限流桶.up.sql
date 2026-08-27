CREATE TABLE IF NOT EXISTS auth_login_throttles (
    bucket_type TEXT NOT NULL CHECK (bucket_type IN ('account', 'address')),
    bucket_hash CHAR(64) NOT NULL,
    failure_count INTEGER NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
    blocked_until TIMESTAMPTZ NOT NULL,
	last_failed_at TIMESTAMPTZ NOT NULL,
	updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (bucket_type, bucket_hash)
);

CREATE INDEX IF NOT EXISTS idx_auth_login_throttles_cleanup
    ON auth_login_throttles (updated_at, bucket_type, bucket_hash);
