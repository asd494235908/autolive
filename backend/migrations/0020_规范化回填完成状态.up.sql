-- 规范化读源的显式准入标记。
-- 迁移只创建 pending 状态；必须由 autolive-backfill-normalized 在同一事务完成
-- 快照复制和逐表校验后切换为 completed。历史快照表继续保留，不在本迁移删除。
CREATE TABLE IF NOT EXISTS normalized_backfill_state (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed')),
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);

INSERT INTO normalized_backfill_state (id, status, completed_at, updated_at)
VALUES (TRUE, 'pending', NULL, CURRENT_TIMESTAMP)
ON CONFLICT (id) DO NOTHING;

-- 专用写事务先取得现有控制面锁，再由数据库拒绝未完成回填的 normalized 写入。
-- 快照模式继续允许影子同步；只有调用 normalized 写入口时才执行此门禁。
CREATE OR REPLACE FUNCTION autolive_require_normalized_backfill_completed()
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    current_status TEXT;
BEGIN
    SELECT status INTO current_status
    FROM normalized_backfill_state
    WHERE id = TRUE;
    IF current_status IS DISTINCT FROM 'completed' THEN
        RAISE EXCEPTION 'normalized backfill is not complete (status %)', COALESCE(current_status, 'missing');
    END IF;
    RETURN 1;
END;
$$;
