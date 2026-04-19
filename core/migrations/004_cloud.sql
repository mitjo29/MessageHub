-- Cloud action outputs (drafts, summaries, search answers).
-- message_id is NULL for smart_search (no anchor message) and
-- NON-NULL for summarize_thread / draft_reply.
CREATE TABLE IF NOT EXISTS ai_drafts (
    id                 TEXT PRIMARY KEY,
    message_id         TEXT,
    action_type        TEXT NOT NULL CHECK (action_type IN
                          ('summarize_thread', 'draft_reply', 'smart_search')),
    input_redacted     TEXT NOT NULL,
    output             TEXT NOT NULL,
    user_edited_output TEXT,
    confidence         REAL NOT NULL,
    provider           TEXT NOT NULL,
    model              TEXT NOT NULL,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_ai_drafts_message ON ai_drafts(message_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_ai_drafts_action  ON ai_drafts(action_type, created_at DESC);
