-- 约束请求预占状态与结束字段的组合，避免规范化表出现无法恢复的半状态。
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'model_request_reservations_status_check'
    ) THEN
        ALTER TABLE model_request_reservations
            ADD CONSTRAINT model_request_reservations_status_check
            CHECK (status IN ('in_flight', 'succeeded', 'failed', 'unknown'));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'model_request_reservations_state_fields_check'
    ) THEN
        ALTER TABLE model_request_reservations
            ADD CONSTRAINT model_request_reservations_state_fields_check
            CHECK (
                (status = 'in_flight'
                    AND reserved_tokens > 0
                    AND finished_at IS NULL
                    AND error_code IS NULL
                    AND result IS NULL)
                OR (status = 'succeeded'
                    AND finished_at IS NOT NULL
                    AND error_code IS NULL
                    AND result IS NOT NULL
                    AND jsonb_typeof(result) = 'object')
                OR (status = 'failed'
                    AND finished_at IS NOT NULL
                    AND error_code IS NOT NULL
                    AND result IS NULL)
                OR (status = 'unknown'
                    AND finished_at IS NOT NULL
                    AND error_code = 'MODEL_REQUEST_UNKNOWN'
                    AND result IS NULL)
            );
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'model_request_reservations_time_check'
    ) THEN
        ALTER TABLE model_request_reservations
            ADD CONSTRAINT model_request_reservations_time_check
            CHECK (finished_at IS NULL OR finished_at >= started_at);
    END IF;
END $$;
