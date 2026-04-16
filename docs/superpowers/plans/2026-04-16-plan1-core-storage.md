# Plan 1: Core + Storage — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set up the Rust monorepo with the core library, SQLite storage layer (SQLCipher encryption, FTS5 search, WAL mode), and the foundational types that all other subsystems depend on.

**Architecture:** Cargo workspace with `core` as a library crate. Storage uses `rusqlite` with `bundled-sqlcipher` feature for encrypted SQLite. Core exposes public types (`Message`, `Contact`, `Channel`, `Thread`) and a `Store` struct with synchronous query/insert methods (rusqlite is sync; async wrapping happens at the Tauri/UniFFI layer). Migrations managed via embedded SQL files.

**Tech Stack:** Rust 1.78+, rusqlite (bundled-sqlcipher), tokio, uuid, chrono, serde, thiserror

---

## File Structure

```
messagehub/
├── Cargo.toml                      # Workspace root
├── core/
│   ├── Cargo.toml                  # Core library crate
│   ├── src/
│   │   ├── lib.rs                  # Public API re-exports
│   │   ├── types/
│   │   │   ├── mod.rs              # Type module re-exports
│   │   │   ├── message.rs          # Message, MessageContent, PriorityScore
│   │   │   ├── contact.rs          # Contact, ContactIdentity
│   │   │   ├── channel.rs          # Channel enum, ChannelConfig
│   │   │   └── thread.rs           # Thread struct
│   │   ├── store/
│   │   │   ├── mod.rs              # Store struct, public methods
│   │   │   ├── migrations.rs       # Migration runner
│   │   │   ├── messages.rs         # Message CRUD + search queries
│   │   │   ├── contacts.rs         # Contact CRUD + identity merging
│   │   │   └── channels.rs         # Channel config + sync state queries
│   │   └── error.rs                # Error types
│   ├── migrations/
│   │   └── 001_initial.sql         # Initial schema
│   └── tests/
│       ├── store_messages_test.rs   # Message storage integration tests
│       ├── store_contacts_test.rs   # Contact storage integration tests
│       ├── store_search_test.rs     # FTS5 search tests
│       └── store_encryption_test.rs # SQLCipher encryption tests
```

---

### Task 1: Initialize Cargo Workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
members = ["core"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "AGPL-3.0"
```

- [ ] **Step 2: Create core crate Cargo.toml**

```toml
[package]
name = "messagehub-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
rusqlite = { version = "0.31", features = ["bundled-sqlcipher", "vtab", "modern_sqlite"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "test-util"] }
```

- [ ] **Step 3: Create minimal lib.rs**

```rust
pub mod error;
pub mod types;
pub mod store;
```

- [ ] **Step 4: Verify workspace compiles**

Run: `cd /home/jocelyn/Applications/MessageHub && cargo check 2>&1`
Expected: Compilation errors (modules don't exist yet) — that's fine, workspace structure is valid.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml core/Cargo.toml core/src/lib.rs
git commit -m "chore: initialize cargo workspace with core crate"
```

---

### Task 2: Define Core Types

**Files:**
- Create: `core/src/error.rs`
- Create: `core/src/types/mod.rs`
- Create: `core/src/types/channel.rs`
- Create: `core/src/types/contact.rs`
- Create: `core/src/types/message.rs`
- Create: `core/src/types/thread.rs`

- [ ] **Step 1: Create error types**

```rust
// core/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("channel error: {0}")]
    Channel(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

- [ ] **Step 2: Create channel types**

```rust
// core/src/types/channel.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Channel {
    Email,
    Sms,
    WhatsApp,
    Teams,
    Telegram,
}

impl Channel {
    pub fn display_name(&self) -> &'static str {
        match self {
            Channel::Email => "Email",
            Channel::Sms => "SMS",
            Channel::WhatsApp => "WhatsApp",
            Channel::Teams => "Teams",
            Channel::Telegram => "Telegram",
        }
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for a connected channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub id: uuid::Uuid,
    pub channel: Channel,
    pub label: String,
    /// Reference to OS keychain entry (not the secret itself).
    pub keychain_ref: String,
    pub enabled: bool,
    pub poll_interval_secs: u32,
    pub last_sync_cursor: Option<String>,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

- [ ] **Step 3: Create contact types**

```rust
// core/src/types/contact.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: Uuid,
    pub display_name: String,
    pub identities: Vec<ContactIdentity>,
    pub vault_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactIdentity {
    pub channel: Channel,
    pub address: String,
}
```

- [ ] **Step 4: Create message types**

```rust
// core/src/types/message.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub channel: Channel,
    pub thread_id: Uuid,
    pub sender_id: Uuid,
    pub content: MessageContent,
    pub timestamp: DateTime<Utc>,
    pub metadata: std::collections::HashMap<String, String>,
    pub priority: Option<PriorityScore>,
    pub category: Option<String>,
    pub is_read: bool,
    pub is_archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContent {
    pub text: Option<String>,
    pub html: Option<String>,
    pub subject: Option<String>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PriorityScore(u8);

impl PriorityScore {
    pub fn new(score: u8) -> Option<Self> {
        if (1..=5).contains(&score) {
            Some(Self(score))
        } else {
            None
        }
    }

    pub fn value(&self) -> u8 {
        self.0
    }
}
```

- [ ] **Step 5: Create thread type**

```rust
// core/src/types/thread.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: Uuid,
    pub channel: Channel,
    pub subject: Option<String>,
    pub participant_ids: Vec<Uuid>,
    pub message_count: u32,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 6: Create types module re-export**

```rust
// core/src/types/mod.rs
pub mod channel;
pub mod contact;
pub mod message;
pub mod thread;

pub use channel::{Channel, ChannelConfig};
pub use contact::{Contact, ContactIdentity};
pub use message::{Attachment, Message, MessageContent, PriorityScore};
pub use thread::Thread;
```

- [ ] **Step 7: Verify everything compiles**

Run: `cd /home/jocelyn/Applications/MessageHub && cargo check`
Expected: Need to create `store` module stub first.

Create a stub:

```rust
// core/src/store/mod.rs
// Store implementation — see Tasks 3-6
```

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 8: Commit**

```bash
git add core/src/
git commit -m "feat: define core types (Message, Contact, Channel, Thread)"
```

---

### Task 3: Database Schema and Migration Runner

**Files:**
- Create: `core/migrations/001_initial.sql`
- Create: `core/src/store/migrations.rs`
- Test: `core/tests/store_encryption_test.rs`

- [ ] **Step 1: Write the initial migration SQL**

```sql
-- core/migrations/001_initial.sql

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

-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

- [ ] **Step 2: Write the migration runner**

```rust
// core/src/store/migrations.rs
use rusqlite::Connection;
use tracing::info;

use crate::error::Result;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../../migrations/001_initial.sql")),
];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );"
    )?;

    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?;

    for (i, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if version > current_version {
            info!(migration = name, version, "applying migration");
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )?;
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Write encryption test**

```rust
// core/tests/store_encryption_test.rs
use rusqlite::Connection;
use tempfile::NamedTempFile;

#[test]
fn test_sqlcipher_encryption_roundtrip() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();
    let password = "test-master-password";

    // Create encrypted database
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", password).unwrap();
        conn.execute_batch(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test VALUES (1, 'secret data');"
        ).unwrap();
    }

    // Read with correct password
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", password).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM test WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "secret data");
    }

    // Fail with wrong password
    {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", "wrong-password").unwrap();
        let result = conn.query_row("SELECT value FROM test WHERE id = 1", [], |r| r.get::<_, String>(0));
        assert!(result.is_err(), "should fail with wrong password");
    }
}

#[test]
fn test_wal_mode_enabled() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();

    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "key", "test-password").unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();

    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}
```

- [ ] **Step 4: Run tests**

Run: `cd /home/jocelyn/Applications/MessageHub && cargo test --test store_encryption_test -- --nocapture`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add core/migrations/ core/src/store/migrations.rs core/tests/store_encryption_test.rs
git commit -m "feat: add database schema, migration runner, and encryption tests"
```

---

### Task 4: Store — Open and Initialize

**Files:**
- Modify: `core/src/store/mod.rs`
- Test: Tested via existing encryption tests + Task 5 tests

- [ ] **Step 1: Implement Store struct**

```rust
// core/src/store/mod.rs
pub mod channels;
pub mod contacts;
pub mod messages;
mod migrations;

use std::path::Path;
use rusqlite::Connection;
use tracing::info;

use crate::error::Result;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) an encrypted database at the given path.
    pub fn open(path: &Path, password: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", password)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", "5000")?;

        migrations::run_migrations(&conn)?;

        info!(path = %path.display(), "database opened");
        Ok(Self { conn })
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::run_migrations(&conn)?;

        Ok(Self { conn })
    }

    /// Access the raw connection (for advanced queries).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: Needs stub files for `channels.rs`, `contacts.rs`, `messages.rs`.

Create stubs:

```rust
// core/src/store/messages.rs
use crate::store::Store;

impl Store {
    // Message queries — see Task 5
}
```

```rust
// core/src/store/contacts.rs
use crate::store::Store;

impl Store {
    // Contact queries — see Task 6
}
```

```rust
// core/src/store/channels.rs
use crate::store::Store;

impl Store {
    // Channel queries — see Task 7
}
```

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add core/src/store/
git commit -m "feat: add Store struct with encrypted open and migration"
```

---

### Task 5: Store — Message CRUD and Search

**Files:**
- Modify: `core/src/store/messages.rs`
- Test: `core/tests/store_messages_test.rs`

- [ ] **Step 1: Write failing tests for message insert and query**

```rust
// core/tests/store_messages_test.rs
use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn make_contact(store: &Store) -> Contact {
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Test User".into(),
        identities: vec![ContactIdentity {
            channel: Channel::Email,
            address: "test@example.com".into(),
        }],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();
    contact
}

fn make_thread(store: &Store) -> Thread {
    let thread = Thread {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        subject: Some("Test thread".into()),
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
    };
    store.insert_thread(&thread).unwrap();
    thread
}

fn make_message(sender_id: Uuid, thread_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id,
        sender_id,
        content: MessageContent {
            text: Some("Hello, this is a test message about contracts".into()),
            html: None,
            subject: Some("Contract Review".into()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: PriorityScore::new(3),
        category: Some("work".into()),
        is_read: false,
        is_archived: false,
    }
}

#[test]
fn test_insert_and_get_message() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);

    store.insert_message(&msg).unwrap();

    let retrieved = store.get_message(&msg.id).unwrap();
    assert_eq!(retrieved.id, msg.id);
    assert_eq!(retrieved.content.subject.as_deref(), Some("Contract Review"));
    assert_eq!(retrieved.is_read, false);
}

#[test]
fn test_list_messages_by_channel() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..3 {
        store.insert_message(&make_message(contact.id, thread.id)).unwrap();
    }

    let messages = store.list_messages(Some(Channel::Email), false, 10, 0).unwrap();
    assert_eq!(messages.len(), 3);
}

#[test]
fn test_mark_message_read() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);
    store.insert_message(&msg).unwrap();

    store.mark_read(&msg.id, true).unwrap();

    let retrieved = store.get_message(&msg.id).unwrap();
    assert!(retrieved.is_read);
}

#[test]
fn test_search_messages_fts() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);
    store.insert_message(&msg).unwrap();

    let results = store.search_messages("contracts", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, msg.id);

    let no_results = store.search_messages("nonexistent", 10).unwrap();
    assert!(no_results.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test store_messages_test 2>&1 | head -30`
Expected: FAIL — methods don't exist yet.

- [ ] **Step 3: Implement message CRUD in store**

```rust
// core/src/store/messages.rs
use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_message(&self, msg: &Message) -> Result<()> {
        let attachments_json = serde_json::to_string(&msg.content.attachments)?;
        let metadata_json = serde_json::to_string(&msg.metadata)?;

        self.conn().execute(
            "INSERT INTO messages (id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                msg.id.to_string(),
                format!("{:?}", msg.channel),
                msg.thread_id.to_string(),
                msg.sender_id.to_string(),
                msg.content.text,
                msg.content.html,
                msg.content.subject,
                attachments_json,
                msg.timestamp.to_rfc3339(),
                metadata_json,
                msg.priority.map(|p| p.value() as i32),
                msg.category,
                msg.is_read as i32,
                msg.is_archived as i32,
            ],
        )?;
        Ok(())
    }

    pub fn get_message(&self, id: &Uuid) -> Result<Message> {
        let id_str = id.to_string();
        self.conn()
            .query_row(
                "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived FROM messages WHERE id = ?1",
                [&id_str],
                |row| Ok(row_to_message(row)),
            )?
            .map_err(|_| CoreError::NotFound {
                entity: "message".into(),
                id: id_str,
            })
    }

    pub fn list_messages(
        &self,
        channel: Option<Channel>,
        archived: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        let mut sql = String::from(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived FROM messages WHERE is_archived = ?1"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(archived as i32)];

        if let Some(ch) = channel {
            sql.push_str(" AND channel_type = ?2");
            params_vec.push(Box::new(format!("{:?}", ch)));
        }

        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");
        let limit_idx = params_vec.len() + 1;
        sql = sql.replace(
            "LIMIT ?",
            &format!("LIMIT ?{} OFFSET ?{}", limit_idx, limit_idx + 1),
        );
        params_vec.push(Box::new(limit));
        params_vec.push(Box::new(offset));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn().prepare(&sql)?;
        let messages = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_message(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(messages)
    }

    pub fn mark_read(&self, id: &Uuid, read: bool) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE messages SET is_read = ?1 WHERE id = ?2",
            params![read as i32, id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "message".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    pub fn search_messages(&self, query: &str, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT m.id, m.channel_type, m.thread_id, m.sender_id, m.content_text, m.content_html, m.content_subject, m.attachments_json, m.timestamp, m.metadata_json, m.priority_score, m.category, m.is_read, m.is_archived
             FROM messages_fts fts
             JOIN messages m ON m.rowid = fts.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2"
        )?;
        let messages = stmt
            .query_map(params![query, limit], |row| Ok(row_to_message(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(messages)
    }
}

fn row_to_message(row: &rusqlite::Row) -> std::result::Result<Message, CoreError> {
    let id_str: String = row.get(0).map_err(rusqlite::Error::from)?;
    let channel_str: String = row.get(1).map_err(rusqlite::Error::from)?;
    let thread_str: String = row.get(2).map_err(rusqlite::Error::from)?;
    let sender_str: String = row.get(3).map_err(rusqlite::Error::from)?;
    let content_text: Option<String> = row.get(4).map_err(rusqlite::Error::from)?;
    let content_html: Option<String> = row.get(5).map_err(rusqlite::Error::from)?;
    let content_subject: Option<String> = row.get(6).map_err(rusqlite::Error::from)?;
    let attachments_json: Option<String> = row.get(7).map_err(rusqlite::Error::from)?;
    let timestamp_str: String = row.get(8).map_err(rusqlite::Error::from)?;
    let metadata_json: Option<String> = row.get(9).map_err(rusqlite::Error::from)?;
    let priority_val: Option<i32> = row.get(10).map_err(rusqlite::Error::from)?;
    let category: Option<String> = row.get(11).map_err(rusqlite::Error::from)?;
    let is_read: i32 = row.get(12).map_err(rusqlite::Error::from)?;
    let is_archived: i32 = row.get(13).map_err(rusqlite::Error::from)?;

    let channel = match channel_str.as_str() {
        "Email" => Channel::Email,
        "Sms" => Channel::Sms,
        "WhatsApp" => Channel::WhatsApp,
        "Teams" => Channel::Teams,
        "Telegram" => Channel::Telegram,
        _ => return Err(CoreError::InvalidInput(format!("unknown channel: {}", channel_str))),
    };

    let attachments: Vec<Attachment> = attachments_json
        .map(|j| serde_json::from_str(&j).unwrap_or_default())
        .unwrap_or_default();

    let metadata: std::collections::HashMap<String, String> = metadata_json
        .map(|j| serde_json::from_str(&j).unwrap_or_default())
        .unwrap_or_default();

    Ok(Message {
        id: Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        channel,
        thread_id: Uuid::parse_str(&thread_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        sender_id: Uuid::parse_str(&sender_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        content: MessageContent {
            text: content_text,
            html: content_html,
            subject: content_subject,
            attachments,
        },
        timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
            .with_timezone(&chrono::Utc),
        metadata,
        priority: priority_val.and_then(|v| PriorityScore::new(v as u8)),
        category,
        is_read: is_read != 0,
        is_archived: is_archived != 0,
    })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test store_messages_test -- --nocapture`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/store/messages.rs core/tests/store_messages_test.rs
git commit -m "feat: add message CRUD and FTS5 search"
```

---

### Task 6: Store — Contact CRUD and Identity Merging

**Files:**
- Modify: `core/src/store/contacts.rs`
- Test: `core/tests/store_contacts_test.rs`

- [ ] **Step 1: Write failing tests**

```rust
// core/tests/store_contacts_test.rs
use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

#[test]
fn test_insert_and_get_contact() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
            ContactIdentity { channel: Channel::Telegram, address: "@sarachen".into() },
        ],
        vault_ref: Some("05-People/Sarah Chen.md".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    store.insert_contact(&contact).unwrap();

    let retrieved = store.get_contact(&contact.id).unwrap();
    assert_eq!(retrieved.display_name, "Sarah Chen");
    assert_eq!(retrieved.identities.len(), 2);
    assert_eq!(retrieved.vault_ref.as_deref(), Some("05-People/Sarah Chen.md"));
}

#[test]
fn test_find_contact_by_address() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
        ],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let found = store.find_contact_by_address(Channel::Email, "sarah@example.com").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, contact.id);

    let not_found = store.find_contact_by_address(Channel::Email, "nobody@example.com").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_merge_contact_identities() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
        ],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let new_identity = ContactIdentity { channel: Channel::WhatsApp, address: "+491234567".into() };
    store.add_identity(&contact.id, &new_identity).unwrap();

    let retrieved = store.get_contact(&contact.id).unwrap();
    assert_eq!(retrieved.identities.len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test store_contacts_test 2>&1 | head -20`
Expected: FAIL — methods don't exist.

- [ ] **Step 3: Implement contact CRUD**

```rust
// core/src/store/contacts.rs
use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_contact(&self, contact: &Contact) -> Result<()> {
        self.conn().execute(
            "INSERT INTO contacts (id, display_name, vault_ref, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                contact.id.to_string(),
                contact.display_name,
                contact.vault_ref,
                contact.created_at.to_rfc3339(),
                contact.updated_at.to_rfc3339(),
            ],
        )?;

        for identity in &contact.identities {
            self.add_identity(&contact.id, identity)?;
        }

        Ok(())
    }

    pub fn get_contact(&self, id: &Uuid) -> Result<Contact> {
        let id_str = id.to_string();
        let (display_name, vault_ref, created_at_str, updated_at_str): (String, Option<String>, String, String) =
            self.conn().query_row(
                "SELECT display_name, vault_ref, created_at, updated_at FROM contacts WHERE id = ?1",
                [&id_str],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|_| CoreError::NotFound { entity: "contact".into(), id: id_str.clone() })?;

        let identities = self.get_identities(id)?;

        Ok(Contact {
            id: *id,
            display_name,
            identities,
            vault_ref,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                .with_timezone(&chrono::Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                .with_timezone(&chrono::Utc),
        })
    }

    pub fn find_contact_by_address(&self, channel: Channel, address: &str) -> Result<Option<Contact>> {
        let channel_str = format!("{:?}", channel);
        let result = self.conn().query_row(
            "SELECT contact_id FROM contact_identities WHERE channel_type = ?1 AND address = ?2",
            params![channel_str, address],
            |row| {
                let id_str: String = row.get(0)?;
                Ok(id_str)
            },
        );

        match result {
            Ok(id_str) => {
                let id = Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
                Ok(Some(self.get_contact(&id)?))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    pub fn add_identity(&self, contact_id: &Uuid, identity: &ContactIdentity) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO contact_identities (contact_id, channel_type, address) VALUES (?1, ?2, ?3)",
            params![
                contact_id.to_string(),
                format!("{:?}", identity.channel),
                identity.address,
            ],
        )?;
        Ok(())
    }

    fn get_identities(&self, contact_id: &Uuid) -> Result<Vec<ContactIdentity>> {
        let mut stmt = self.conn().prepare(
            "SELECT channel_type, address FROM contact_identities WHERE contact_id = ?1"
        )?;
        let identities = stmt
            .query_map([contact_id.to_string()], |row| {
                let channel_str: String = row.get(0)?;
                let address: String = row.get(1)?;
                Ok((channel_str, address))
            })?
            .filter_map(|r| r.ok())
            .map(|(ch, addr)| {
                let channel = match ch.as_str() {
                    "Email" => Channel::Email,
                    "Sms" => Channel::Sms,
                    "WhatsApp" => Channel::WhatsApp,
                    "Teams" => Channel::Teams,
                    "Telegram" => Channel::Telegram,
                    _ => Channel::Email,
                };
                ContactIdentity { channel, address: addr }
            })
            .collect();
        Ok(identities)
    }

    pub fn insert_thread(&self, thread: &crate::types::Thread) -> Result<()> {
        self.conn().execute(
            "INSERT INTO threads (id, channel_type, subject, message_count, last_message_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                thread.id.to_string(),
                format!("{:?}", thread.channel),
                thread.subject,
                thread.message_count,
                thread.last_message_at.to_rfc3339(),
                thread.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test store_contacts_test -- --nocapture`
Expected: 3 tests pass.

- [ ] **Step 5: Run all tests to ensure no regressions**

Run: `cargo test`
Expected: All tests pass (encryption + messages + contacts).

- [ ] **Step 6: Commit**

```bash
git add core/src/store/contacts.rs core/tests/store_contacts_test.rs
git commit -m "feat: add contact CRUD with identity merging and address lookup"
```

---

### Task 7: Store — Channel Config CRUD

**Files:**
- Modify: `core/src/store/channels.rs`
- No separate test file — this is small enough to test inline with message/contact tests

- [ ] **Step 1: Implement channel config methods**

```rust
// core/src/store/channels.rs
use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_channel_config(&self, config: &ChannelConfig) -> Result<()> {
        self.conn().execute(
            "INSERT INTO channels (id, channel_type, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                config.id.to_string(),
                format!("{:?}", config.channel),
                config.label,
                config.keychain_ref,
                config.enabled as i32,
                config.poll_interval_secs,
                config.last_sync_cursor,
                config.last_sync_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn list_channel_configs(&self) -> Result<Vec<ChannelConfig>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, channel_type, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at FROM channels"
        )?;
        let configs = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let channel_str: String = row.get(1)?;
                let label: String = row.get(2)?;
                let keychain_ref: String = row.get(3)?;
                let enabled: i32 = row.get(4)?;
                let poll_interval_secs: u32 = row.get(5)?;
                let last_sync_cursor: Option<String> = row.get(6)?;
                let last_sync_at_str: Option<String> = row.get(7)?;

                Ok((id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at_str)| {
                let channel = match channel_str.as_str() {
                    "Email" => Channel::Email,
                    "Sms" => Channel::Sms,
                    "WhatsApp" => Channel::WhatsApp,
                    "Teams" => Channel::Teams,
                    "Telegram" => Channel::Telegram,
                    _ => return None,
                };
                let last_sync_at = last_sync_at_str
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc));

                Some(ChannelConfig {
                    id: Uuid::parse_str(&id_str).ok()?,
                    channel,
                    label,
                    keychain_ref,
                    enabled: enabled != 0,
                    poll_interval_secs,
                    last_sync_cursor,
                    last_sync_at,
                })
            })
            .collect();
        Ok(configs)
    }

    pub fn update_sync_state(&self, channel_id: &Uuid, cursor: Option<&str>, synced_at: chrono::DateTime<chrono::Utc>) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE channels SET last_sync_cursor = ?1, last_sync_at = ?2, updated_at = ?3 WHERE id = ?4",
            params![cursor, synced_at.to_rfc3339(), synced_at.to_rfc3339(), channel_id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound { entity: "channel".into(), id: channel_id.to_string() });
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All existing tests still pass. Channel methods compile.

- [ ] **Step 3: Commit**

```bash
git add core/src/store/channels.rs
git commit -m "feat: add channel config CRUD and sync state tracking"
```

---

### Task 8: Store — Full-Text Search Tests

**Files:**
- Test: `core/tests/store_search_test.rs`

- [ ] **Step 1: Write comprehensive search tests**

```rust
// core/tests/store_search_test.rs
use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn seed_messages(store: &Store) -> (Uuid, Uuid) {
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Test Sender".into(),
        identities: vec![ContactIdentity { channel: Channel::Email, address: "test@example.com".into() }],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let thread = crate::messagehub_core::types::Thread {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        subject: None,
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
    };
    store.insert_thread(&thread).unwrap();

    let messages = vec![
        ("Contract review for Q2 delivery", "Please review the attached contract for the helicopter parts delivery"),
        ("Sprint planning meeting", "Let's discuss the next sprint goals and assign tasks"),
        ("Invoice #2024-089", "Attached is the invoice for consulting services rendered in March"),
        ("Dinner reservation confirmed", "Your table for 4 is confirmed at Restaurant Le Jardin for Saturday"),
    ];

    for (subject, body) in messages {
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Email,
            thread_id: thread.id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some(body.into()),
                html: None,
                subject: Some(subject.into()),
                attachments: vec![],
            },
            timestamp: Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
        };
        store.insert_message(&msg).unwrap();
    }

    (contact.id, thread.id)
}

#[test]
fn test_search_by_subject() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("contract", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.subject.as_ref().unwrap().contains("Contract"));
}

#[test]
fn test_search_by_body() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("helicopter", 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_search_multiple_results() {
    let store = test_store();
    seed_messages(&store);

    // Both "contract" and "consulting" messages mention professional services
    let results = store.search_messages("review OR invoice", 10).unwrap();
    assert!(results.len() >= 2);
}

#[test]
fn test_search_no_results() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("blockchain", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_respects_limit() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("the", 2).unwrap();
    assert!(results.len() <= 2);
}
```

Wait — the `seed_messages` function references `crate::messagehub_core` which is wrong for an integration test. Let me fix that:

```rust
// core/tests/store_search_test.rs
use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn seed_messages(store: &Store) {
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Test Sender".into(),
        identities: vec![ContactIdentity { channel: Channel::Email, address: "test@example.com".into() }],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let thread = Thread {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        subject: None,
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
    };
    store.insert_thread(&thread).unwrap();

    let messages = vec![
        ("Contract review for Q2 delivery", "Please review the attached contract for the helicopter parts delivery"),
        ("Sprint planning meeting", "Let's discuss the next sprint goals and assign tasks"),
        ("Invoice #2024-089", "Attached is the invoice for consulting services rendered in March"),
        ("Dinner reservation confirmed", "Your table for 4 is confirmed at Restaurant Le Jardin for Saturday"),
    ];

    for (subject, body) in messages {
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Email,
            thread_id: thread.id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some(body.into()),
                html: None,
                subject: Some(subject.into()),
                attachments: vec![],
            },
            timestamp: Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
        };
        store.insert_message(&msg).unwrap();
    }
}

#[test]
fn test_search_by_subject() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("contract", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.subject.as_ref().unwrap().contains("Contract"));
}

#[test]
fn test_search_by_body() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("helicopter", 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_search_multiple_results() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("review OR invoice", 10).unwrap();
    assert!(results.len() >= 2);
}

#[test]
fn test_search_no_results() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("blockchain", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_respects_limit() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("the", 2).unwrap();
    assert!(results.len() <= 2);
}
```

- [ ] **Step 2: Run search tests**

Run: `cargo test --test store_search_test -- --nocapture`
Expected: 5 tests pass.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass (encryption: 2, messages: 4, contacts: 3, search: 5 = 14 total).

- [ ] **Step 4: Commit**

```bash
git add core/tests/store_search_test.rs
git commit -m "test: add comprehensive FTS5 search tests"
```

---

## Summary

After completing all 8 tasks, you have:

- **Cargo workspace** with `core` library crate
- **Core types**: `Message`, `Contact`, `Channel`, `Thread`, `ChannelConfig`, `PriorityScore` — shared by all future subsystems
- **Encrypted SQLite store** (SQLCipher + WAL mode) with migration runner
- **Message CRUD** with insert, get, list (by channel, archived status), mark read
- **Contact CRUD** with identity merging and cross-channel address lookup
- **Channel config** CRUD with sync state tracking
- **FTS5 full-text search** with automatic sync triggers
- **Action log table** ready for AI audit trail
- **14 integration tests** covering encryption, CRUD, search, and identity merging

**Next plan:** Plan 2 (Channel Adapters) builds on this foundation — adapters will use the `Store` to persist messages and the `ChannelAdapter` trait from `types/channel.rs`.
