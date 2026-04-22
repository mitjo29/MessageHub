-- Plan 7b.3: reply_drafts stores work-in-progress compose state. One row per
-- thread; autosave UPSERTs, successful send DELETEs. Separate from ai_drafts
-- (which is an append-only log of AI generations).
CREATE TABLE IF NOT EXISTS reply_drafts (
    thread_id                TEXT PRIMARY KEY,
    in_reply_to_message_id   TEXT NOT NULL,
    body                     TEXT NOT NULL DEFAULT '',
    subject                  TEXT,
    updated_at               TEXT NOT NULL
                             DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
