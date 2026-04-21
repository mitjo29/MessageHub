-- B-003: Add external_id + unique partial index so ingest can dedup on
-- (channel_type, external_id) across polls. Legacy rows have external_id
-- NULL; the `WHERE external_id IS NOT NULL` clause exempts them from the
-- uniqueness constraint.
ALTER TABLE messages ADD COLUMN external_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_external_id
ON messages (channel_type, external_id)
WHERE external_id IS NOT NULL;
