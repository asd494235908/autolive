-- 阶段运行中与成功事件可以共享同一个 operation_id，事件本身使用 event_id 唯一。
ALTER TABLE variant_task_events
    DROP CONSTRAINT IF EXISTS variant_task_events_task_id_variant_id_stage_operation_id_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'variant_task_events_task_id_variant_id_event_id_key'
    ) THEN
        ALTER TABLE variant_task_events
            ADD CONSTRAINT variant_task_events_task_id_variant_id_event_id_key
            UNIQUE (task_id, variant_id, event_id);
    END IF;
END $$;
