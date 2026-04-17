-- Per-entity lookup: "give me every AI decision about this message"
CREATE INDEX IF NOT EXISTS idx_action_log_entity
    ON action_log(entity_type, entity_id);

-- Secondary index for audit-by-type queries ("show me all priority_score actions from last week")
CREATE INDEX IF NOT EXISTS idx_action_log_type_time
    ON action_log(action_type, created_at DESC);
