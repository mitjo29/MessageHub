-- Runtime: per-channel health tracking.
ALTER TABLE channels ADD COLUMN status TEXT NOT NULL DEFAULT 'healthy';
ALTER TABLE channels ADD COLUMN last_error TEXT;
ALTER TABLE channels ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;

-- Runtime: thread matching by external service id.
ALTER TABLE threads ADD COLUMN external_thread_id TEXT;
CREATE INDEX IF NOT EXISTS idx_threads_external ON threads(channel_type, external_thread_id);
