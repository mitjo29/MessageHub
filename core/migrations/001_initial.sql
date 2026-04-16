-- Channels (configured connections)
CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    channel_type TEXT NOT NULL,
    label TEXT NOT NULL,
    keychain_ref TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    poll_interval_secs INTEGER NOT NULL DEFAULT 30,
    last_sync_cursor TEXT,
    last_sync_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Contacts
CREATE TABLE IF NOT EXISTS contacts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    vault_ref TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Contact identities (many-to-one with contacts)
CREATE TABLE IF NOT EXISTS contact_identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    channel_type TEXT NOT NULL,
    address TEXT NOT NULL,
    UNIQUE(channel_type, address)
);
CREATE INDEX IF NOT EXISTS idx_contact_identities_address ON contact_identities(address);

-- Threads
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY,
    channel_type TEXT NOT NULL,
    subject TEXT,
    message_count INTEGER NOT NULL DEFAULT 0,
    last_message_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Thread participants (many-to-many)
CREATE TABLE IF NOT EXISTS thread_participants (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    contact_id TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (thread_id, contact_id)
);

-- Messages
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    channel_type TEXT NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    sender_id TEXT NOT NULL REFERENCES contacts(id),
    content_text TEXT,
    content_html TEXT,
    content_subject TEXT,
    attachments_json TEXT,
    timestamp TEXT NOT NULL,
    metadata_json TEXT,
    priority_score INTEGER,
    category TEXT,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages(channel_type);
CREATE INDEX IF NOT EXISTS idx_messages_priority ON messages(priority_score DESC);
CREATE INDEX IF NOT EXISTS idx_messages_unread ON messages(is_read) WHERE is_read = 0;

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content_text,
    content_subject,
    content=messages,
    content_rowid=rowid
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content_text, content_subject)
    VALUES (new.rowid, new.content_text, new.content_subject);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content_text, content_subject)
    VALUES ('delete', old.rowid, old.content_text, old.content_subject);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content_text, content_subject)
    VALUES ('delete', old.rowid, old.content_text, old.content_subject);
    INSERT INTO messages_fts(rowid, content_text, content_subject)
    VALUES (new.rowid, new.content_text, new.content_subject);
END;

-- Action log (future-proofing for AI audit trail)
CREATE TABLE IF NOT EXISTS action_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_type TEXT NOT NULL,
    entity_type TEXT,
    entity_id TEXT,
    reasoning TEXT,
    confidence_score REAL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
