-- 将 0002 的初始化占位快照升级为显式空 State，避免应用把损坏快照误判为新系统。
UPDATE control_plane_state
SET state = '{"version":1,"state":{}}'::jsonb,
    updated_at = CURRENT_TIMESTAMP
WHERE id = TRUE
  AND state = '{"version":1}'::jsonb;
