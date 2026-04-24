-- B-004: Persist which configured channel a message was received on, so the
-- reply path can route through the same account it arrived on. Multi-account
-- users (e.g. work@ + personal@ Email) previously had replies fall back to
-- whichever channel sorted first.
--
-- Existing rows have NULL — `send_email_reply` falls back to first-match for
-- legacy data, matching prior behavior.
ALTER TABLE messages ADD COLUMN received_on_channel_id TEXT REFERENCES channels(id);

CREATE INDEX IF NOT EXISTS idx_messages_received_on_channel
ON messages(received_on_channel_id)
WHERE received_on_channel_id IS NOT NULL;
