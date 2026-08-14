-- 为任务和阶段事件保留完整结构化载荷，同时保留已有查询字段。
-- 当前仍由控制面快照事务影子写入，后续再切换任务领域的规范化读写事实源。
ALTER TABLE variant_tasks
    ADD COLUMN IF NOT EXISTS task_payload JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE variant_task_events
    ADD COLUMN IF NOT EXISTS event_id TEXT NOT NULL DEFAULT '';

UPDATE variant_task_events
SET event_id = COALESCE(NULLIF(payload->>'event_id', ''), id)
WHERE event_id = '';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'variant_tasks_task_payload_object_check'
    ) THEN
        ALTER TABLE variant_tasks
            ADD CONSTRAINT variant_tasks_task_payload_object_check
            CHECK (jsonb_typeof(task_payload) = 'object');
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'variant_task_events_payload_object_check'
    ) THEN
        ALTER TABLE variant_task_events
            ADD CONSTRAINT variant_task_events_payload_object_check
            CHECK (jsonb_typeof(payload) = 'object');
    END IF;
END $$;
