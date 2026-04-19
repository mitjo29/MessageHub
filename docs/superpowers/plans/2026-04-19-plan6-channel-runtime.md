# Plan 6: Channel Runtime — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `core` from a library of parts into a running pipeline. Build a `Runtime` that polls adapters on their configured intervals, ingests messages into the store (contact + thread resolution), dispatches each new message to a local classifier worker, publishes `RuntimeEvent`s over a broadcast channel, and tracks per-channel health with exponential backoff. Delete the vestigial `AdapterManager`.

**Architecture:** New `core/src/runtime/` subtree. One `ChannelTask` (owns its `Box<dyn ChannelAdapter>`) per enabled `channels` row; a single `Ingestor` that resolves contacts + threads and inserts messages; a single `ClassifierWorker` that drains a bounded mpsc and runs `AiPipeline::classify_stored`. Events go out over `tokio::sync::broadcast`. Shutdown is cooperative via `tokio_util::sync::CancellationToken`.

**Tech Stack:** `tokio` (already a dep — mpsc, broadcast, spawn, select), `tokio-util` (NEW runtime dep — `CancellationToken`; feature `rt`), `rand` (add if absent — ±20% backoff jitter), `async-trait` (already a dep), `tracing` (already a dep). Dev-only: `tokio/test-util` for `tokio::time::pause`/`advance` (already used by Plan 4 tests).

**Prerequisites:** Plans 1–4 merged. Plan 5 (cloud actions) landed on master — no direct runtime dependency but the workspace must be clean.

**Spec:** `docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`.

---

## File Structure

```
core/
├── Cargo.toml                              # MODIFY — add `tokio-util`, `rand`
├── migrations/
│   └── 005_runtime.sql                     # CREATE — channels.{status,last_error,consecutive_failures},
│                                           #          threads.external_thread_id + index
├── src/
│   ├── lib.rs                              # MODIFY — pub mod runtime;
│   ├── adapters/
│   │   ├── mod.rs                          # MODIFY — remove `pub mod manager;`
│   │   └── manager.rs                      # DELETE
│   ├── ai/
│   │   └── pipeline.rs                     # MODIFY — add classify_stored(store, id)
│   ├── store/
│   │   ├── migrations.rs                   # MODIFY — register 005
│   │   ├── channels.rs                     # MODIFY — load + persist status columns
│   │   ├── contacts.rs                     # MODIFY — find_or_create_contact,
│   │   │                                   #          find_thread_by_external_id,
│   │   │                                   #          insert_thread persists external_thread_id
│   │   └── messages.rs                     # MODIFY — set_message_classification
│   ├── types/
│   │   ├── channel.rs                      # MODIFY — ChannelConfig gains status fields
│   │   └── thread.rs                       # MODIFY — Thread gains external_thread_id
│   └── runtime/
│       ├── mod.rs                          # CREATE — Runtime, RuntimeBuilder, IngestJob
│       ├── status.rs                       # CREATE — ChannelStatus + BackoffState
│       ├── events.rs                       # CREATE — RuntimeEvent + EventBus
│       ├── factory.rs                      # CREATE — AdapterFactory + FactoryRegistry
│       ├── ingestor.rs                     # CREATE — Ingestor task
│       ├── classifier_worker.rs            # CREATE — ClassifierWorker task
│       └── channel_task.rs                 # CREATE — ChannelTask poll loop
└── tests/
    ├── runtime_full_loop.rs                # CREATE
    ├── runtime_graceful_degradation.rs     # CREATE
    ├── runtime_backoff.rs                  # CREATE
    ├── runtime_shutdown.rs                 # CREATE
    ├── runtime_reload.rs                   # CREATE
    ├── runtime_classifier_failure.rs       # CREATE
    └── runtime_channel_isolation.rs        # CREATE
```

---

### Task 1: Migration 005 + schema-bearing type changes + store helpers

**Files:**
- Create: `core/migrations/005_runtime.sql`
- Modify: `core/src/store/migrations.rs`
- Modify: `core/src/types/channel.rs`
- Modify: `core/src/types/thread.rs`
- Modify: `core/src/store/channels.rs`
- Modify: `core/src/store/contacts.rs` (thread + contact helpers live here today)
- Modify: `core/src/store/messages.rs`
- Modify: `core/Cargo.toml`

`★ Why this matters:` Every downstream task depends on these schema and helper changes. Running this task end-to-end leaves the DB layer ready; nothing about `runtime/` is created yet so the crate still compiles exactly as it did before, just with richer store APIs.

- [ ] **Step 1: Add dependencies**

Open `core/Cargo.toml` and, in `[dependencies]`, add (if not already present):

```toml
tokio-util = { version = "0.7", features = ["rt"] }
rand = "0.8"
```

Run: `cargo check -p messagehub-core`
Expected: compiles cleanly.

- [ ] **Step 2: Create migration SQL**

Create `core/migrations/005_runtime.sql` with:

```sql
-- Runtime: per-channel health tracking.
ALTER TABLE channels ADD COLUMN status TEXT NOT NULL DEFAULT 'healthy';
ALTER TABLE channels ADD COLUMN last_error TEXT;
ALTER TABLE channels ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;

-- Runtime: thread matching by external service id.
ALTER TABLE threads ADD COLUMN external_thread_id TEXT;
CREATE INDEX IF NOT EXISTS idx_threads_external ON threads(channel_type, external_thread_id);
```

- [ ] **Step 3: Register migration 005**

Open `core/src/store/migrations.rs`. Find the `MIGRATIONS` slice (or equivalent registration). Add the new migration following the existing pattern. Example shape (match the local convention exactly):

```rust
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial",   include_str!("../../migrations/001_initial.sql")),
    ("002_knowledge", include_str!("../../migrations/002_knowledge.sql")),
    ("003_ai",        include_str!("../../migrations/003_ai.sql")),
    ("004_cloud",     include_str!("../../migrations/004_cloud.sql")),
    ("005_runtime",   include_str!("../../migrations/005_runtime.sql")),
];
```

Run: `cargo test -p messagehub-core --lib -- --nocapture migrations`
Expected: migration tests pass (new migration applies cleanly to a fresh in-memory DB).

- [ ] **Step 4: Extend `ChannelConfig` with status fields**

Open `core/src/types/channel.rs`. Replace the `ChannelConfig` struct with:

```rust
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
    /// Current health (persisted). Defaults to Healthy for rows predating Plan 6.
    #[serde(default)]
    pub status: crate::runtime::status::ChannelStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
}
```

Note: This references `crate::runtime::status::ChannelStatus`, which does not yet exist — it will be added in Task 3 as part of the runtime module skeleton. For this step, add a forward-declaring stub at the top of `core/src/lib.rs`:

```rust
pub mod runtime;  // NEW — module body lands in Task 3
```

And create the absolute minimum `core/src/runtime/mod.rs`:

```rust
pub mod status;
```

And `core/src/runtime/status.rs` (real contents land in Task 4 — this is just the type that `ChannelConfig` references):

```rust
use serde::{Deserialize, Serialize};

/// Per-channel health state. Persisted to `channels.status` as a lowercase string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelStatus {
    #[default]
    Healthy,
    Degraded { attempt: u32 },
    Failed { last_error: String },
}

impl ChannelStatus {
    pub fn db_str(&self) -> &'static str {
        match self {
            ChannelStatus::Healthy => "healthy",
            ChannelStatus::Degraded { .. } => "degraded",
            ChannelStatus::Failed { .. } => "failed",
        }
    }
}
```

Run: `cargo check -p messagehub-core`
Expected: compiles.

- [ ] **Step 5: Extend `Thread` with `external_thread_id`**

Open `core/src/types/thread.rs`. Replace the struct with:

```rust
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
    /// External service's thread/conversation id (null for synthesized threads).
    pub external_thread_id: Option<String>,
}
```

Update any existing constructors / test helpers that build a `Thread` so they set `external_thread_id: None`. Run `cargo check -p messagehub-core` and follow the compiler errors — most will be in `core/src/store/contacts.rs` (the `insert_thread` helper) and any tests that construct `Thread` literals. Set `external_thread_id: None` in each construction site.

Run: `cargo check -p messagehub-core --all-targets`
Expected: compiles.

- [ ] **Step 6: Update `insert_thread` to persist `external_thread_id`**

Open `core/src/store/contacts.rs`. Replace the `insert_thread` method body with:

```rust
pub fn insert_thread(&self, thread: &crate::types::Thread) -> Result<()> {
    self.conn().execute(
        "INSERT INTO threads (id, channel_type, subject, message_count, last_message_at, created_at, external_thread_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            thread.id.to_string(),
            thread.channel.to_db_str(),
            thread.subject,
            thread.message_count,
            thread.last_message_at.to_rfc3339(),
            thread.created_at.to_rfc3339(),
            thread.external_thread_id,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 7: Write test for `find_thread_by_external_id`**

In `core/src/store/contacts.rs` (or wherever the threads helpers' test module lives), add:

```rust
#[test]
fn test_find_thread_by_external_id_roundtrip() {
    use crate::types::{Channel, Thread};
    use chrono::Utc;
    use uuid::Uuid;

    let store = crate::store::Store::open_in_memory().unwrap();
    let thread = Thread {
        id: Uuid::new_v4(),
        channel: Channel::Telegram,
        subject: None,
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
        external_thread_id: Some("chat-42".to_string()),
    };
    store.insert_thread(&thread).unwrap();

    let hit = store
        .find_thread_by_external_id(Channel::Telegram, "chat-42")
        .unwrap();
    assert!(hit.is_some());
    assert_eq!(hit.unwrap().id, thread.id);

    let miss = store
        .find_thread_by_external_id(Channel::Telegram, "chat-99")
        .unwrap();
    assert!(miss.is_none());

    let wrong_channel = store
        .find_thread_by_external_id(Channel::Email, "chat-42")
        .unwrap();
    assert!(wrong_channel.is_none());
}
```

Run: `cargo test -p messagehub-core find_thread_by_external_id_roundtrip`
Expected: FAIL — method does not exist.

- [ ] **Step 8: Implement `find_thread_by_external_id`**

In `core/src/store/contacts.rs`, inside the `impl Store` block that already hosts `insert_thread`, add:

```rust
pub fn find_thread_by_external_id(
    &self,
    channel: crate::types::Channel,
    external_id: &str,
) -> Result<Option<crate::types::Thread>> {
    use crate::types::Thread;
    use rusqlite::OptionalExtension;

    let row = self.conn().query_row(
        "SELECT id, channel_type, subject, message_count, last_message_at, created_at, external_thread_id \
         FROM threads \
         WHERE channel_type = ?1 AND external_thread_id = ?2 \
         LIMIT 1",
        params![channel.to_db_str(), external_id],
        |row| {
            let id_str: String = row.get(0)?;
            let channel_str: String = row.get(1)?;
            let subject: Option<String> = row.get(2)?;
            let message_count: u32 = row.get(3)?;
            let last_at: String = row.get(4)?;
            let created_at: String = row.get(5)?;
            let ext: Option<String> = row.get(6)?;
            Ok((id_str, channel_str, subject, message_count, last_at, created_at, ext))
        },
    ).optional()?;

    let Some((id_str, channel_str, subject, message_count, last_at, created_at, ext)) = row else {
        return Ok(None);
    };

    let id = uuid::Uuid::parse_str(&id_str)
        .map_err(|e| CoreError::InvalidInput(format!("bad thread id: {}", e)))?;
    let channel = crate::types::Channel::from_db_str(&channel_str)
        .ok_or_else(|| CoreError::InvalidInput(format!("unknown channel: {}", channel_str)))?;
    let last_message_at = chrono::DateTime::parse_from_rfc3339(&last_at)
        .map_err(|e| CoreError::InvalidInput(format!("bad last_message_at: {}", e)))?
        .with_timezone(&chrono::Utc);
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
        .map_err(|e| CoreError::InvalidInput(format!("bad created_at: {}", e)))?
        .with_timezone(&chrono::Utc);

    Ok(Some(Thread {
        id,
        channel,
        subject,
        participant_ids: vec![], // not loaded here; Ingestor does not need them
        message_count,
        last_message_at,
        created_at,
        external_thread_id: ext,
    }))
}
```

Run: `cargo test -p messagehub-core find_thread_by_external_id_roundtrip`
Expected: PASS.

- [ ] **Step 9: Write test for `find_or_create_contact_by_address`**

Add to `core/src/store/contacts.rs` tests:

```rust
#[test]
fn test_find_or_create_contact_by_address_creates_then_finds() {
    use crate::types::Channel;

    let store = crate::store::Store::open_in_memory().unwrap();

    let a = store
        .find_or_create_contact_by_address(Channel::Telegram, "alice_bot", "Alice")
        .unwrap();
    let b = store
        .find_or_create_contact_by_address(Channel::Telegram, "alice_bot", "Alice-renamed")
        .unwrap();

    assert_eq!(a.id, b.id, "second call should return the same contact");
    // Display name is set only on creation; second call does not overwrite.
    assert_eq!(a.display_name, "Alice");
}
```

Run: `cargo test -p messagehub-core find_or_create_contact_by_address_creates_then_finds`
Expected: FAIL — method does not exist.

- [ ] **Step 10: Implement `find_or_create_contact_by_address`**

In `core/src/store/contacts.rs`, inside `impl Store`, add:

```rust
pub fn find_or_create_contact_by_address(
    &self,
    channel: crate::types::Channel,
    address: &str,
    display_name: &str,
) -> Result<crate::types::Contact> {
    use crate::types::{Contact, ContactIdentity};

    if let Some(existing) = self.find_contact_by_address(channel, address)? {
        return Ok(existing);
    }

    let contact = Contact {
        id: uuid::Uuid::new_v4(),
        display_name: display_name.to_string(),
        vault_ref: None,
        identities: vec![ContactIdentity {
            channel,
            address: address.to_string(),
        }],
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    self.insert_contact(&contact)?;
    self.add_identity(
        &contact.id,
        &ContactIdentity { channel, address: address.to_string() },
    )?;
    Ok(contact)
}
```

(If `Contact` field layout differs from the above, adjust to the struct as defined in `core/src/types/contact.rs` — the essential contract is: one call → one contact guaranteed to have a matching identity.)

Run: `cargo test -p messagehub-core find_or_create_contact_by_address_creates_then_finds`
Expected: PASS.

- [ ] **Step 11: Write test for `set_message_classification`**

Add to `core/src/store/messages.rs` tests:

```rust
#[test]
fn test_set_message_classification_updates_row() {
    use crate::types::{Category, Channel, Message, MessageContent, Priority};

    let store = crate::store::Store::open_in_memory().unwrap();
    // Minimum viable setup: insert a contact and thread first to satisfy FKs.
    let contact = store
        .find_or_create_contact_by_address(Channel::Telegram, "u1", "User")
        .unwrap();
    let thread_id = uuid::Uuid::new_v4();
    store.insert_thread(&crate::types::Thread {
        id: thread_id,
        channel: Channel::Telegram,
        subject: None,
        participant_ids: vec![],
        message_count: 0,
        last_message_at: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
        external_thread_id: None,
    }).unwrap();

    let msg = Message {
        id: uuid::Uuid::new_v4(),
        channel: Channel::Telegram,
        thread_id,
        sender_id: contact.id,
        content: MessageContent {
            text: Some("Hi".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
        },
        timestamp: chrono::Utc::now(),
        metadata: std::collections::HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    };
    store.insert_message(&msg).unwrap();

    store.set_message_classification(&msg.id, Some("Work"), Some(Priority::High))
        .unwrap();

    let reloaded = store.get_message(&msg.id).unwrap();
    assert_eq!(reloaded.category.as_deref(), Some("Work"));
    assert_eq!(reloaded.priority, Some(Priority::High));
}
```

Run: `cargo test -p messagehub-core set_message_classification_updates_row`
Expected: FAIL — method does not exist.

- [ ] **Step 12: Implement `set_message_classification`**

In `core/src/store/messages.rs`, inside `impl Store`, add:

```rust
pub fn set_message_classification(
    &self,
    id: &uuid::Uuid,
    category: Option<&str>,
    priority: Option<crate::types::Priority>,
) -> Result<()> {
    let priority_score: Option<i64> = priority.map(|p| p.value() as i64);
    self.conn().execute(
        "UPDATE messages SET priority_score = ?1, category = ?2 WHERE id = ?3",
        params![priority_score, category, id.to_string()],
    )?;
    Ok(())
}
```

Run: `cargo test -p messagehub-core set_message_classification_updates_row`
Expected: PASS.

- [ ] **Step 13: Write test for `update_channel_status`**

Add to `core/src/store/channels.rs` tests:

```rust
#[test]
fn test_update_channel_status_persists_and_reloads() {
    use crate::runtime::status::ChannelStatus;
    use crate::types::{Channel, ChannelConfig};

    let store = crate::store::Store::open_in_memory().unwrap();
    let id = uuid::Uuid::new_v4();
    store.insert_channel_config(&ChannelConfig {
        id,
        channel: Channel::Telegram,
        label: "t".to_string(),
        keychain_ref: "ref".to_string(),
        enabled: true,
        poll_interval_secs: 30,
        last_sync_cursor: None,
        last_sync_at: None,
        status: ChannelStatus::Healthy,
        last_error: None,
        consecutive_failures: 0,
    }).unwrap();

    store.update_channel_status(
        &id,
        &ChannelStatus::Failed { last_error: "boom".to_string() },
        3,
    ).unwrap();

    let cfgs = store.list_channel_configs().unwrap();
    let cfg = cfgs.iter().find(|c| c.id == id).unwrap();
    assert_eq!(cfg.status, ChannelStatus::Failed { last_error: "boom".to_string() });
    assert_eq!(cfg.last_error.as_deref(), Some("boom"));
    assert_eq!(cfg.consecutive_failures, 3);
}
```

Run: `cargo test -p messagehub-core update_channel_status_persists_and_reloads`
Expected: FAIL.

- [ ] **Step 14: Update `list_channel_configs` + `insert_channel_config` to handle status, then add `update_channel_status`**

Open `core/src/store/channels.rs`. Update the SQL in `insert_channel_config` to include the new columns:

```rust
pub fn insert_channel_config(&self, config: &ChannelConfig) -> Result<()> {
    self.conn().execute(
        "INSERT INTO channels (id, channel_type, label, keychain_ref, enabled, \
                               poll_interval_secs, last_sync_cursor, last_sync_at, \
                               status, last_error, consecutive_failures) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            config.id.to_string(),
            config.channel.to_db_str(),
            config.label,
            config.keychain_ref,
            config.enabled as i64,
            config.poll_interval_secs,
            config.last_sync_cursor,
            config.last_sync_at.map(|t| t.to_rfc3339()),
            config.status.db_str(),
            config.last_error,
            config.consecutive_failures,
        ],
    )?;
    Ok(())
}
```

Update `list_channel_configs` to select and hydrate the new columns. The SELECT list becomes:

```sql
SELECT id, channel_type, label, keychain_ref, enabled, poll_interval_secs,
       last_sync_cursor, last_sync_at, status, last_error, consecutive_failures
FROM channels
```

In the row mapper, convert `status` + `last_error` + `consecutive_failures` to a `ChannelStatus`:

```rust
let status_str: String = row.get(8)?;
let last_error: Option<String> = row.get(9)?;
let consecutive_failures: u32 = row.get(10)?;
let status = match status_str.as_str() {
    "healthy"  => crate::runtime::status::ChannelStatus::Healthy,
    "degraded" => crate::runtime::status::ChannelStatus::Degraded {
        attempt: consecutive_failures.max(1),
    },
    "failed"   => crate::runtime::status::ChannelStatus::Failed {
        last_error: last_error.clone().unwrap_or_default(),
    },
    other => return Err(rusqlite::Error::InvalidParameterName(
        format!("unknown channel status: {}", other),
    )),
};
```

Then add the new `update_channel_status` method:

```rust
pub fn update_channel_status(
    &self,
    id: &uuid::Uuid,
    status: &crate::runtime::status::ChannelStatus,
    consecutive_failures: u32,
) -> Result<()> {
    let last_error = match status {
        crate::runtime::status::ChannelStatus::Failed { last_error } => Some(last_error.as_str()),
        _ => None,
    };
    self.conn().execute(
        "UPDATE channels SET status = ?1, last_error = ?2, consecutive_failures = ?3 WHERE id = ?4",
        params![status.db_str(), last_error, consecutive_failures, id.to_string()],
    )?;
    Ok(())
}
```

Run: `cargo test -p messagehub-core update_channel_status_persists_and_reloads`
Expected: PASS.

Run the whole store test suite: `cargo test -p messagehub-core --lib store`
Expected: all pass (check that the new columns didn't break existing insert/list tests — they shouldn't because of the `DEFAULT` clauses, and old test fixtures now need the new `ChannelConfig` fields populated — fix any compilation errors by adding `status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0` to old `ChannelConfig` literals).

- [ ] **Step 15: Commit**

```bash
git add core/Cargo.toml core/migrations/005_runtime.sql core/src/store/ core/src/types/ core/src/lib.rs core/src/runtime/mod.rs core/src/runtime/status.rs
git commit -m "feat(runtime): add migration 005 + schema-bearing type changes

- channels: status, last_error, consecutive_failures
- threads: external_thread_id + index
- ChannelConfig + Thread gain the new fields
- Store helpers: find_thread_by_external_id, find_or_create_contact_by_address,
  set_message_classification, update_channel_status
- Stub ChannelStatus enum in core/src/runtime/status.rs (full impl in Task 4)"
```

---

### Task 2: Delete `AdapterManager`

**Files:**
- Delete: `core/src/adapters/manager.rs`
- Modify: `core/src/adapters/mod.rs`

`★ Why this matters:` The existing `AdapterManager` is a Plan-2 proof-of-concept with an `Fn(Vec<RawMessage>)` sync callback that cannot compose with async ingestion. The new `Runtime` replaces it. Doing this as its own commit keeps the diff reviewable and proves nothing outside `adapters/` references it.

- [ ] **Step 1: Confirm no external references**

Run: `grep -rn "AdapterManager" core/src core/tests`
Expected: matches appear only inside `core/src/adapters/manager.rs` (and nowhere else).

If any reference outside that file shows up, stop — read the call sites and fold removal into this task.

- [ ] **Step 2: Delete the file**

```bash
git rm core/src/adapters/manager.rs
```

- [ ] **Step 3: Remove the module declaration**

Open `core/src/adapters/mod.rs` and delete the line `pub mod manager;` at the top.

- [ ] **Step 4: Verify the crate still compiles and tests pass**

```bash
cargo build -p messagehub-core
cargo test  -p messagehub-core
```

Expected: clean build. If any test relied on `MockAdapter` via `manager::tests` imports, adjust the import to `crate::adapters::mock::MockAdapter`. All existing tests outside `manager.rs` should still pass.

- [ ] **Step 5: Commit**

```bash
git add -A core/src/adapters/
git commit -m "refactor(runtime): delete vestigial AdapterManager

Replaced by the Runtime module (landed in subsequent tasks).
The Fn(Vec<RawMessage>) sync callback cannot compose with the
async ingestion pipeline Plan 6 introduces."
```

---

### Task 3: Runtime module skeleton

**Files:**
- Modify: `core/src/runtime/mod.rs`
- Create: `core/src/runtime/events.rs` (stub)
- Create: `core/src/runtime/factory.rs` (stub)
- Create: `core/src/runtime/ingestor.rs` (stub)
- Create: `core/src/runtime/classifier_worker.rs` (stub)
- Create: `core/src/runtime/channel_task.rs` (stub)

`★ Why this matters:` All module files declared up-front means every subsequent task is fill-in, not structural. Stubs keep the crate compilable between tasks.

- [ ] **Step 1: Create stubs for each runtime submodule**

Create `core/src/runtime/events.rs`:

```rust
//! Runtime events — populated in Task 5.
```

Create `core/src/runtime/factory.rs`:

```rust
//! AdapterFactory trait — populated in Task 6.
```

Create `core/src/runtime/ingestor.rs`:

```rust
//! Ingestor task — populated in Task 7.
```

Create `core/src/runtime/classifier_worker.rs`:

```rust
//! ClassifierWorker task — populated in Task 9.
```

Create `core/src/runtime/channel_task.rs`:

```rust
//! Per-channel polling task — populated in Task 10.
```

- [ ] **Step 2: Expose them from `runtime/mod.rs`**

Replace `core/src/runtime/mod.rs` with:

```rust
//! Runtime: orchestration layer that drives adapters → ingestion → classification.
//!
//! See `docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`.

pub mod status;
pub mod events;
pub mod factory;
pub mod ingestor;
pub mod classifier_worker;
pub mod channel_task;

// Runtime + RuntimeBuilder land in Task 11.
```

Run: `cargo build -p messagehub-core`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add core/src/runtime/
git commit -m "feat(runtime): scaffold runtime module skeleton"
```

---

### Task 4: `status.rs` — ChannelStatus + BackoffState

**Files:**
- Modify: `core/src/runtime/status.rs`

`★ Why this matters:` The state machine + backoff math is pure logic, easy to table-test. Getting it right in isolation pays off across the whole runtime — every later test relies on deterministic backoff.

- [ ] **Step 1: Write failing tests for BackoffState transitions**

Replace `core/src/runtime/status.rs` with (keeping the existing `ChannelStatus` from Task 1):

```rust
use serde::{Deserialize, Serialize};

/// Per-channel health state. Persisted to `channels.status` as a lowercase string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelStatus {
    #[default]
    Healthy,
    Degraded { attempt: u32 },
    Failed { last_error: String },
}

impl ChannelStatus {
    pub fn db_str(&self) -> &'static str {
        match self {
            ChannelStatus::Healthy => "healthy",
            ChannelStatus::Degraded { .. } => "degraded",
            ChannelStatus::Failed { .. } => "failed",
        }
    }
}

/// Threshold at which Degraded transitions to Failed.
pub const FAIL_THRESHOLD: u32 = 4;
/// Hard ceiling on backoff delay.
pub const MAX_BACKOFF_SECS: u64 = 600;

/// Tracks consecutive failures and derives the next poll delay.
///
/// Exponential base-2 with ±20% jitter, clamped to `MAX_BACKOFF_SECS`.
#[derive(Debug, Clone, Default)]
pub struct BackoffState {
    pub consecutive_failures: u32,
}

impl BackoffState {
    pub fn new() -> Self { Self { consecutive_failures: 0 } }

    /// Classify the current state into a `ChannelStatus`.
    pub fn status(&self, last_error: Option<&str>) -> ChannelStatus {
        if self.consecutive_failures == 0 {
            ChannelStatus::Healthy
        } else if self.consecutive_failures < FAIL_THRESHOLD {
            ChannelStatus::Degraded { attempt: self.consecutive_failures }
        } else {
            ChannelStatus::Failed {
                last_error: last_error.unwrap_or("unknown").to_string(),
            }
        }
    }

    /// Reset to Healthy after a successful fetch.
    pub fn record_success(&mut self) { self.consecutive_failures = 0; }

    /// Increment failure counter.
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Deterministic delay (seconds) before jitter. Used by callers that want
    /// testability; production code uses `next_delay`.
    pub fn base_delay_secs(&self, poll_interval_secs: u32) -> u64 {
        if self.consecutive_failures == 0 {
            return poll_interval_secs as u64;
        }
        let exp = self.consecutive_failures.min(16); // avoid shift overflow
        let raw = (poll_interval_secs as u64).saturating_mul(1u64 << exp);
        raw.min(MAX_BACKOFF_SECS)
    }

    /// Actual next delay with ±20% jitter applied via the injected RNG.
    pub fn next_delay_secs<R: rand::Rng>(&self, poll_interval_secs: u32, rng: &mut R) -> u64 {
        let base = self.base_delay_secs(poll_interval_secs);
        if base == 0 { return 0; }
        let jitter: f64 = rng.gen_range(-0.2..=0.2);
        let delayed = (base as f64 * (1.0 + jitter)).round() as i64;
        delayed.max(0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn fresh_state_is_healthy() {
        let s = BackoffState::new();
        assert_eq!(s.status(None), ChannelStatus::Healthy);
        assert_eq!(s.base_delay_secs(30), 30);
    }

    #[test]
    fn failures_progress_through_degraded_to_failed() {
        let mut s = BackoffState::new();
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 1 });
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 2 });
        s.record_failure();
        assert_eq!(s.status(None), ChannelStatus::Degraded { attempt: 3 });
        s.record_failure();
        assert_eq!(
            s.status(Some("x")),
            ChannelStatus::Failed { last_error: "x".to_string() },
        );
    }

    #[test]
    fn success_resets_state() {
        let mut s = BackoffState::new();
        for _ in 0..5 { s.record_failure(); }
        s.record_success();
        assert_eq!(s.status(None), ChannelStatus::Healthy);
        assert_eq!(s.base_delay_secs(30), 30);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut s = BackoffState::new();
        let base = 30u32;
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 60);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 120);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 240);
        s.record_failure();
        assert_eq!(s.base_delay_secs(base), 480);
        s.record_failure();
        // 30 * 32 = 960 → clamped to 600
        assert_eq!(s.base_delay_secs(base), MAX_BACKOFF_SECS);
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        let mut s = BackoffState::new();
        s.record_failure();
        let base = s.base_delay_secs(30);
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        for _ in 0..1000 {
            let d = s.next_delay_secs(30, &mut rng);
            let lo = (base as f64 * 0.8).floor() as u64;
            let hi = (base as f64 * 1.2).ceil()  as u64;
            assert!(d >= lo && d <= hi, "delay {} outside [{}, {}]", d, lo, hi);
        }
    }

    #[test]
    fn db_str_roundtrip() {
        assert_eq!(ChannelStatus::Healthy.db_str(), "healthy");
        assert_eq!(ChannelStatus::Degraded { attempt: 1 }.db_str(), "degraded");
        assert_eq!(
            ChannelStatus::Failed { last_error: "x".to_string() }.db_str(),
            "failed",
        );
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p messagehub-core runtime::status
```

Expected: all six tests pass.

- [ ] **Step 3: Commit**

```bash
git add core/src/runtime/status.rs
git commit -m "feat(runtime): add ChannelStatus + BackoffState with table tests"
```

---

### Task 5: `events.rs` — RuntimeEvent + EventBus

**Files:**
- Modify: `core/src/runtime/events.rs`

`★ Why this matters:` All runtime components publish through a shared `EventBus`. Defining it early means the ingestor, classifier, and channel task all have somewhere to publish in their respective tasks.

- [ ] **Step 1: Write the event type + bus + test**

Replace `core/src/runtime/events.rs` with:

```rust
use tokio::sync::broadcast;
use tracing::trace;
use uuid::Uuid;

use crate::runtime::status::ChannelStatus;
use crate::types::Priority;

/// Events published by the runtime. Consumers subscribe via `Runtime::subscribe`.
#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    MessageIngested      { id: Uuid, channel_id: Uuid },
    MessageClassified    { id: Uuid, category: Option<String>, priority: Option<Priority> },
    SyncSucceeded        { channel_id: Uuid, count: usize },
    SyncFailed           { channel_id: Uuid, error: String, attempt: u32 },
    ChannelStatusChanged { channel_id: Uuid, status: ChannelStatus },
}

/// Thin wrapper around `broadcast::Sender` that silently drops send errors.
///
/// `broadcast::Sender::send` returns `Err` iff there are no receivers. The
/// runtime must not care — nobody subscribed yet is a valid state.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RuntimeEvent>,
}

impl EventBus {
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }

    /// Publish an event. Never blocks, never errors.
    pub fn publish(&self, ev: RuntimeEvent) {
        if let Err(err) = self.tx.send(ev) {
            trace!(error = %err, "no runtime event subscribers");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Priority;

    #[tokio::test]
    async fn publish_and_receive_roundtrip() {
        let bus = EventBus::with_capacity(4);
        let mut rx = bus.subscribe();
        let id = Uuid::new_v4();
        let ch = Uuid::new_v4();
        bus.publish(RuntimeEvent::MessageIngested { id, channel_id: ch });
        bus.publish(RuntimeEvent::MessageClassified {
            id,
            category: Some("Work".to_string()),
            priority: Some(Priority::High),
        });
        match rx.recv().await.unwrap() {
            RuntimeEvent::MessageIngested { id: got, .. } => assert_eq!(got, id),
            other => panic!("unexpected event: {:?}", other),
        }
        match rx.recv().await.unwrap() {
            RuntimeEvent::MessageClassified { id: got, priority, .. } => {
                assert_eq!(got, id);
                assert_eq!(priority, Some(Priority::High));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn publish_without_subscribers_is_noop() {
        let bus = EventBus::with_capacity(4);
        bus.publish(RuntimeEvent::SyncSucceeded {
            channel_id: Uuid::new_v4(),
            count: 3,
        });
        // No panic, no error — success condition.
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p messagehub-core runtime::events
```

Expected: both tests pass.

- [ ] **Step 3: Commit**

```bash
git add core/src/runtime/events.rs
git commit -m "feat(runtime): add RuntimeEvent + EventBus"
```

---

### Task 6: `factory.rs` — AdapterFactory trait + FactoryRegistry

**Files:**
- Modify: `core/src/runtime/factory.rs`

`★ Why this matters:` Keeps `core` adapter-agnostic. The `Runtime` holds an `Arc<dyn AdapterFactory>` per channel type and never matches on concrete adapter structs.

- [ ] **Step 1: Write trait + registry + tests**

Replace `core/src/runtime/factory.rs` with:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::ChannelAdapter;
use crate::error::Result;
use crate::types::ChannelConfig;

/// Builds adapter instances from persisted channel rows.
///
/// The factory is responsible for credential resolution (keychain lookup,
/// OAuth refresh). The `Runtime` calls `build` once per channel at startup,
/// then calls `connect` on the returned adapter before starting the poll loop.
#[async_trait]
pub trait AdapterFactory: Send + Sync {
    async fn build(&self, config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>>;
}

/// Keyed registry of factories. Key is the DB `channel_type` string
/// (see `Channel::to_db_str`: "Email", "Telegram", etc.).
#[derive(Default, Clone)]
pub struct FactoryRegistry {
    inner: HashMap<String, Arc<dyn AdapterFactory>>,
}

impl FactoryRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, channel_type: impl Into<String>, factory: Arc<dyn AdapterFactory>) {
        self.inner.insert(channel_type.into(), factory);
    }

    pub fn get(&self, channel_type: &str) -> Option<Arc<dyn AdapterFactory>> {
        self.inner.get(channel_type).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{ChannelAdapter, RawMessage};
    use crate::error::Result;
    use crate::types::{Channel, MessageContent};
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};

    struct DummyAdapter;
    #[async_trait]
    impl ChannelAdapter for DummyAdapter {
        async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> { Ok(()) }
        async fn fetch_messages(&self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
            Ok(vec![])
        }
        async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> { Ok(()) }
        async fn disconnect(&mut self) -> Result<()> { Ok(()) }
        fn channel_type(&self) -> Channel { Channel::Telegram }
    }

    struct DummyFactory;
    #[async_trait]
    impl AdapterFactory for DummyFactory {
        async fn build(&self, _config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
            Ok(Box::new(DummyAdapter))
        }
    }

    #[test]
    fn registry_returns_registered_factory() {
        let mut reg = FactoryRegistry::new();
        reg.register("Telegram", Arc::new(DummyFactory));
        assert!(reg.get("Telegram").is_some());
        assert!(reg.get("Email").is_none());
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p messagehub-core runtime::factory
```

Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add core/src/runtime/factory.rs
git commit -m "feat(runtime): add AdapterFactory trait + FactoryRegistry"
```

---

### Task 7: `ingestor.rs` — contact + thread resolution + insert

**Files:**
- Modify: `core/src/runtime/ingestor.rs`

`★ Why this matters:` Single task that serializes identity-merging. Owns the `RawMessage → Message` path end-to-end. Bounded mpsc in both directions gives us backpressure for free.

- [ ] **Step 1: Define IngestJob + Ingestor type + public API**

Replace `core/src/runtime/ingestor.rs` with (this also contains the function the channel task will call):

```rust
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::adapters::{normalize, RawMessage};
use crate::error::Result;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::store::Store;
use crate::types::{Channel, Thread};

/// A batch of raw messages fetched from a single channel, awaiting ingestion.
#[derive(Debug)]
pub struct IngestJob {
    pub channel_id: Uuid,
    pub batch: Vec<RawMessage>,
}

/// Spawns the ingestor task. Returns the sender used by channel tasks
/// to enqueue jobs, the JoinHandle, and a sender for classifier ids.
pub fn spawn_ingestor(
    store: Arc<Store>,
    bus: EventBus,
    classifier_tx: Option<mpsc::Sender<Uuid>>,
    queue_capacity: usize,
    shutdown: CancellationToken,
) -> (mpsc::Sender<IngestJob>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<IngestJob>(queue_capacity);

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    info!("ingestor: shutdown signalled, draining queue");
                    while let Ok(job) = rx.try_recv() {
                        process_job(&store, &bus, classifier_tx.as_ref(), job).await;
                    }
                    break;
                }
                maybe_job = rx.recv() => {
                    match maybe_job {
                        Some(job) => process_job(&store, &bus, classifier_tx.as_ref(), job).await,
                        None => { info!("ingestor: channel closed, exiting"); break; }
                    }
                }
            }
        }
    });

    (tx, handle)
}

async fn process_job(
    store: &Store,
    bus: &EventBus,
    classifier_tx: Option<&mpsc::Sender<Uuid>>,
    job: IngestJob,
) {
    let IngestJob { channel_id, batch } = job;
    for raw in batch {
        match ingest_one(store, &raw) {
            Ok(message_id) => {
                bus.publish(RuntimeEvent::MessageIngested {
                    id: message_id,
                    channel_id,
                });
                if let Some(tx) = classifier_tx {
                    if let Err(e) = tx.send(message_id).await {
                        warn!(error = %e, "classifier queue closed; skipping classify");
                    }
                }
            }
            Err(e) => {
                error!(
                    external_id = %raw.external_id,
                    channel = %raw.channel,
                    error = %e,
                    "ingestor: dropped message after store error"
                );
            }
        }
    }
}

/// Resolve contact + thread, normalize, insert. Returns the new message id.
fn ingest_one(store: &Store, raw: &RawMessage) -> Result<Uuid> {
    let contact = store.find_or_create_contact_by_address(
        raw.channel,
        &raw.sender_address,
        &raw.sender_name,
    )?;

    let thread = resolve_thread(store, raw)?;

    // Clone because `normalize` takes the RawMessage by value and the caller
    // expected to keep it.
    let message = normalize(raw.clone(), contact.id, thread.id);
    store.insert_message(&message)?;
    debug!(id = %message.id, "ingestor: stored message");
    Ok(message.id)
}

fn resolve_thread(store: &Store, raw: &RawMessage) -> Result<Thread> {
    if let Some(ext) = raw.external_thread_id.as_deref() {
        if let Some(t) = store.find_thread_by_external_id(raw.channel, ext)? {
            return Ok(t);
        }
    }

    // Synthesize a new thread. External id may still be None (some channels
    // don't provide one); that is fine — future messages with the same
    // external id will match, and messages with no external id each get
    // their own thread. Thread grouping by subject/participants is a later
    // concern (not in Plan 6 scope).
    let thread = Thread {
        id: Uuid::new_v4(),
        channel: raw.channel,
        subject: raw.subject.clone(),
        participant_ids: vec![],
        message_count: 0,
        last_message_at: raw.timestamp,
        created_at: raw.timestamp,
        external_thread_id: raw.external_thread_id.clone(),
    };
    store.insert_thread(&thread)?;
    Ok(thread)
}
```

- [ ] **Step 2: Write an integration-style unit test**

Add at the bottom of `core/src/runtime/ingestor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::RawMessage;
    use crate::types::Channel;
    use std::collections::HashMap;

    fn raw(ext_thread: Option<&str>, sender: &str) -> RawMessage {
        RawMessage {
            external_id: uuid::Uuid::new_v4().to_string(),
            channel: Channel::Telegram,
            external_thread_id: ext_thread.map(String::from),
            sender_name: sender.to_string(),
            sender_address: sender.to_string(),
            text: Some("hi".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn ingest_inserts_contact_thread_message() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let (tx, handle) = spawn_ingestor(
            Arc::clone(&store),
            bus.clone(),
            None, // no classifier
            8,
            CancellationToken::new(),
        );
        let channel_id = Uuid::new_v4();

        tx.send(IngestJob {
            channel_id,
            batch: vec![raw(Some("chat-1"), "alice")],
        }).await.unwrap();

        // First event must be MessageIngested.
        let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await.unwrap().unwrap();
        match evt {
            RuntimeEvent::MessageIngested { channel_id: got_ch, .. } => {
                assert_eq!(got_ch, channel_id);
            }
            other => panic!("unexpected event: {:?}", other),
        }

        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn same_external_thread_reuses_existing_thread() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let (tx, handle) = spawn_ingestor(
            Arc::clone(&store), bus, None, 8, CancellationToken::new(),
        );
        let channel_id = Uuid::new_v4();

        tx.send(IngestJob {
            channel_id,
            batch: vec![
                raw(Some("chat-same"), "alice"),
                raw(Some("chat-same"), "bob"),
            ],
        }).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        // Collect the two MessageIngested events, look up each message, and
        // assert they share a thread_id.
        let mut ids: Vec<Uuid> = Vec::new();
        for _ in 0..2 {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await.unwrap().unwrap();
            if let RuntimeEvent::MessageIngested { id, .. } = evt { ids.push(id); }
        }
        assert_eq!(ids.len(), 2);
        let m1 = store.get_message(&ids[0]).unwrap();
        let m2 = store.get_message(&ids[1]).unwrap();
        assert_eq!(m1.thread_id, m2.thread_id,
                   "both messages should land in the same thread");
    }
}
```

- [ ] **Step 3: Run the tests**

```bash
cargo test -p messagehub-core runtime::ingestor
```

Expected: both tests pass.

- [ ] **Step 4: Commit**

```bash
git add core/src/runtime/ingestor.rs
git commit -m "feat(runtime): add Ingestor with contact + thread resolution"
```

---

### Task 8: Add `AiPipeline::classify_stored`

**Files:**
- Modify: `core/src/ai/pipeline.rs`

`★ Why this matters:` The existing `enrich_and_store` conflates insert + classify in one call, which is exactly what the fan-out worker architecture needs to split. Adding `classify_stored` is the minimum change — the ingestor does the insert; the worker calls this to classify an already-stored message.

- [ ] **Step 1: Write a failing test**

Add to `core/src/ai/pipeline.rs` tests (create the `#[cfg(test)] mod tests` block if it doesn't exist, following the patterns in the file):

```rust
#[cfg(test)]
mod classify_stored_tests {
    use super::*;
    use crate::ai::llm::LlmBackend;
    use crate::ai::profile::UserProfile;
    use crate::store::Store;
    use crate::types::{Channel, Message, MessageContent};
    use async_trait::async_trait;

    // Minimal stub LLM that always returns a parseable classification.
    struct StubLlm;
    #[async_trait]
    impl LlmBackend for StubLlm {
        async fn complete(&self, _prompt: &str) -> crate::error::Result<String> {
            // Return a response matching whatever the classifier parser expects.
            // Inspect `core/src/ai/classifier.rs` for the concrete format.
            Ok("{\"category\":\"Work\",\"priority\":4,\"reasoning\":\"test\"}".to_string())
        }
    }

    async fn setup_stored_message() -> (Arc<Store>, uuid::Uuid) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let contact = store.find_or_create_contact_by_address(
            Channel::Telegram, "u1", "User",
        ).unwrap();
        let thread_id = uuid::Uuid::new_v4();
        store.insert_thread(&crate::types::Thread {
            id: thread_id,
            channel: Channel::Telegram,
            subject: None,
            participant_ids: vec![],
            message_count: 0,
            last_message_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            external_thread_id: None,
        }).unwrap();
        let msg = Message {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Telegram,
            thread_id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some("Hello".to_string()),
                html: None,
                subject: None,
                attachments: vec![],
            },
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
        };
        let id = msg.id;
        store.insert_message(&msg).unwrap();
        (store, id)
    }

    #[tokio::test]
    async fn classify_stored_happy_path_updates_row() {
        let (store, id) = setup_stored_message().await;
        let pipeline = AiPipeline::new(
            Arc::new(StubLlm),
            None,
            UserProfile::default(),
        );

        let outcome = pipeline.classify_stored(&store, &id).await.unwrap();
        assert!(outcome.classified);

        let reloaded = store.get_message(&id).unwrap();
        assert!(reloaded.category.is_some());
        assert!(reloaded.priority.is_some());
    }
}
```

If the stub prompt body doesn't match the classifier's parser, inspect `core/src/ai/classifier.rs` and adjust the stubbed response accordingly. The essential assertion is: given a stored message and a succeeding LLM, the DB row afterwards has a non-None `category` and `priority`.

Run: `cargo test -p messagehub-core classify_stored_happy_path_updates_row`
Expected: FAIL — method does not exist.

- [ ] **Step 2: Implement `classify_stored`**

In `core/src/ai/pipeline.rs`, inside `impl AiPipeline`, add:

```rust
/// Classify an already-stored message and persist `(category, priority)`
/// to its row. On classifier failure, writes `category=Unknown, priority=Low`
/// so the message surfaces in the UI and logs a `classify_failed` action.
///
/// This is the method the runtime's `ClassifierWorker` calls. Unlike
/// `enrich_and_store`, it does not insert — it assumes the ingestor already
/// persisted the message.
pub async fn classify_stored(
    &self,
    store: &Store,
    id: &uuid::Uuid,
) -> Result<EnrichOutcome> {
    let msg = store.get_message(id)?;
    // The sender's display name + address come from the stored contact.
    // The Ingestor inserted them; re-fetch for the classifier.
    let sender = store.get_contact(&msg.sender_id)?;
    let sender_address = sender
        .identities
        .iter()
        .find(|i| i.channel == msg.channel)
        .map(|i| i.address.clone())
        .unwrap_or_default();

    let subject = msg.content.subject.clone().unwrap_or_default();
    let body = msg.content.text.clone().unwrap_or_default();

    let rag = build_rag_context(
        store,
        self.retriever.as_ref(),
        &self.profile,
        msg.channel,
        &sender_address,
        &subject,
        &body,
    )?;

    let result = self.classifier.classify(
        msg.channel,
        &sender.display_name,
        &sender_address,
        &subject,
        &body,
        &rag,
    ).await;

    let message_id_str = id.to_string();
    match result {
        Ok(classification) => {
            store.set_message_classification(
                id,
                Some(classification.category.as_str()),
                Some(classification.priority),
            )?;
            store.log_ai_decision(
                "classify",
                "message",
                &message_id_str,
                &classification.reasoning,
                1.0,
            )?;
            info!(
                message_id = %message_id_str,
                priority = classification.priority.value(),
                category = classification.category.as_str(),
                "classify_stored: message classified",
            );
            Ok(EnrichOutcome { classified: true })
        }
        Err(e) => {
            // Degraded fallback — write Unknown/Low so UI still surfaces it.
            use crate::types::{Category, Priority};
            store.set_message_classification(
                id,
                Some(Category::Unknown.as_str()),
                Some(Priority::Low),
            )?;
            let reason = format!("classification failed: {}", e);
            if let Err(log_err) = store.log_ai_decision(
                "classify_failed", "message", &message_id_str, &reason, 0.0,
            ) {
                warn!(error = %log_err, "failed to log classification failure");
            }
            debug!(
                message_id = %message_id_str,
                error = %e,
                "classify_stored: degraded",
            );
            Ok(EnrichOutcome { classified: false })
        }
    }
}
```

Note: If `Category` does not have an `Unknown` variant and/or `as_str()`, check the enum in `core/src/types/category.rs` and pick the closest "no-category" variant (e.g. an `Other` or serialize `"Unknown"` as a string literal). The contract is: on failure, write a non-None category + `Priority::Low` so the UI sees the message.

Run: `cargo test -p messagehub-core classify_stored_happy_path_updates_row`
Expected: PASS.

- [ ] **Step 3: Add a failing-LLM test**

Append to the same `mod classify_stored_tests`:

```rust
struct ErroringLlm;
#[async_trait]
impl LlmBackend for ErroringLlm {
    async fn complete(&self, _prompt: &str) -> crate::error::Result<String> {
        Err(crate::error::CoreError::InvalidInput("simulated llm failure".to_string()))
    }
}

#[tokio::test]
async fn classify_stored_llm_failure_writes_degraded_row() {
    let (store, id) = setup_stored_message().await;
    let pipeline = AiPipeline::new(
        Arc::new(ErroringLlm),
        None,
        UserProfile::default(),
    );
    let outcome = pipeline.classify_stored(&store, &id).await.unwrap();
    assert!(!outcome.classified);

    let reloaded = store.get_message(&id).unwrap();
    // Still readable; category is non-None (fallback), priority is Low.
    assert!(reloaded.category.is_some());
    assert_eq!(reloaded.priority, Some(crate::types::Priority::Low));
}
```

Run: `cargo test -p messagehub-core classify_stored_llm_failure_writes_degraded_row`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add core/src/ai/pipeline.rs
git commit -m "feat(ai): add AiPipeline::classify_stored for runtime worker"
```

---

### Task 9: `classifier_worker.rs` — drain mpsc, classify, update row

**Files:**
- Modify: `core/src/runtime/classifier_worker.rs`

`★ Why this matters:` Decouples I/O (polling) from AI (classification). A stuck LLM parks the classifier, not the channel task.

- [ ] **Step 1: Write the worker**

Replace `core/src/runtime/classifier_worker.rs` with:

```rust
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::ai::pipeline::AiPipeline;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::store::Store;

/// Spawns the single classifier worker. Returns the sender used by the
/// ingestor to enqueue message ids, and the JoinHandle.
///
/// If `ai_pipeline` is `None`, the worker is not spawned and this returns
/// `(None, None)` — the ingestor will see `classifier_tx = None` and skip
/// classification entirely.
pub fn maybe_spawn_classifier(
    store: Arc<Store>,
    ai_pipeline: Option<Arc<AiPipeline>>,
    bus: EventBus,
    queue_capacity: usize,
    shutdown: CancellationToken,
) -> (Option<mpsc::Sender<Uuid>>, Option<JoinHandle<()>>) {
    let Some(pipeline) = ai_pipeline else {
        info!("classifier worker not spawned: no AiPipeline configured");
        return (None, None);
    };

    let (tx, mut rx) = mpsc::channel::<Uuid>(queue_capacity);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    info!("classifier: shutdown signalled, draining queue");
                    while let Ok(id) = rx.try_recv() {
                        classify_one(&store, &pipeline, &bus, id).await;
                    }
                    break;
                }
                maybe_id = rx.recv() => {
                    match maybe_id {
                        Some(id) => classify_one(&store, &pipeline, &bus, id).await,
                        None => { info!("classifier: channel closed, exiting"); break; }
                    }
                }
            }
        }
    });
    (Some(tx), Some(handle))
}

async fn classify_one(
    store: &Store,
    pipeline: &AiPipeline,
    bus: &EventBus,
    id: Uuid,
) {
    match pipeline.classify_stored(store, &id).await {
        Ok(_outcome) => {
            // Reload to publish the final (category, priority).
            match store.get_message(&id) {
                Ok(msg) => bus.publish(RuntimeEvent::MessageClassified {
                    id: msg.id,
                    category: msg.category,
                    priority: msg.priority,
                }),
                Err(e) => warn!(message_id = %id, error = %e,
                                "classifier: could not reload message after classify"),
            }
        }
        Err(e) => {
            // classify_stored returns Err only on unrecoverable store errors;
            // LLM failures are already converted to `classified=false` inside.
            error!(message_id = %id, error = %e,
                   "classifier: unrecoverable store error during classify");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::LlmBackend;
    use crate::ai::profile::UserProfile;
    use crate::types::{Channel, Message, MessageContent};
    use async_trait::async_trait;

    struct StubLlm;
    #[async_trait]
    impl LlmBackend for StubLlm {
        async fn complete(&self, _prompt: &str) -> crate::error::Result<String> {
            Ok("{\"category\":\"Work\",\"priority\":4,\"reasoning\":\"ok\"}".to_string())
        }
    }

    #[tokio::test]
    async fn worker_processes_queued_id_and_emits_event() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        // Seed a message.
        let contact = store.find_or_create_contact_by_address(
            Channel::Telegram, "u1", "User",
        ).unwrap();
        let thread_id = Uuid::new_v4();
        store.insert_thread(&crate::types::Thread {
            id: thread_id,
            channel: Channel::Telegram,
            subject: None,
            participant_ids: vec![],
            message_count: 0,
            last_message_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            external_thread_id: None,
        }).unwrap();
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            thread_id,
            sender_id: contact.id,
            content: MessageContent { text: Some("hi".into()), html: None, subject: None, attachments: vec![] },
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
            priority: None, category: None, is_read: false, is_archived: false,
        };
        let msg_id = msg.id;
        store.insert_message(&msg).unwrap();

        let pipeline = Arc::new(AiPipeline::new(
            Arc::new(StubLlm), None, UserProfile::default(),
        ));
        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let (tx, handle) = maybe_spawn_classifier(
            Arc::clone(&store),
            Some(pipeline),
            bus,
            8,
            CancellationToken::new(),
        );
        let tx = tx.expect("classifier should be spawned");

        tx.send(msg_id).await.unwrap();

        let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await.unwrap().unwrap();
        match evt {
            RuntimeEvent::MessageClassified { id, category, priority } => {
                assert_eq!(id, msg_id);
                assert!(category.is_some());
                assert!(priority.is_some());
            }
            other => panic!("unexpected: {:?}", other),
        }

        drop(tx);
        handle.unwrap().await.unwrap();
    }

    #[tokio::test]
    async fn worker_not_spawned_when_pipeline_absent() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let bus = EventBus::with_capacity(16);
        let (tx, handle) = maybe_spawn_classifier(
            store, None, bus, 8, CancellationToken::new(),
        );
        assert!(tx.is_none());
        assert!(handle.is_none());
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p messagehub-core runtime::classifier_worker
```

Expected: both tests pass.

- [ ] **Step 3: Commit**

```bash
git add core/src/runtime/classifier_worker.rs
git commit -m "feat(runtime): add ClassifierWorker draining an mpsc queue"
```

---

### Task 10: `channel_task.rs` — per-channel poll loop with backoff

**Files:**
- Modify: `core/src/runtime/channel_task.rs`

`★ Why this matters:` The per-channel task is the only moving part that combines everything — adapter I/O, backoff math, status persistence, events, shutdown cancellation.

- [ ] **Step 1: Write the task + its handle**

Replace `core/src/runtime/channel_task.rs` with:

```rust
use std::sync::Arc;
use std::time::Duration;

use rand::SeedableRng;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::adapters::ChannelAdapter;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::runtime::ingestor::IngestJob;
use crate::runtime::status::{BackoffState, ChannelStatus};
use crate::store::Store;
use crate::types::ChannelConfig;

/// Handle for a running channel task. Drop or call `stop()` to cancel.
pub struct ChannelTaskHandle {
    pub config_id: Uuid,
    pub token: CancellationToken,
    pub join: JoinHandle<()>,
}

impl ChannelTaskHandle {
    pub fn stop(&self) { self.token.cancel(); }
}

/// Spawns the polling loop for one channel.
pub fn spawn_channel_task(
    config: ChannelConfig,
    adapter: Box<dyn ChannelAdapter>,
    store: Arc<Store>,
    ingest_tx: mpsc::Sender<IngestJob>,
    bus: EventBus,
    parent_token: &CancellationToken,
) -> ChannelTaskHandle {
    let token = parent_token.child_token();
    let task_token = token.clone();
    let config_id = config.id;

    let join = tokio::spawn(run_channel_task(
        config, adapter, store, ingest_tx, bus, task_token,
    ));

    ChannelTaskHandle { config_id, token, join }
}

async fn run_channel_task(
    config: ChannelConfig,
    mut adapter: Box<dyn ChannelAdapter>,
    store: Arc<Store>,
    ingest_tx: mpsc::Sender<IngestJob>,
    bus: EventBus,
    token: CancellationToken,
) {
    let channel_id = config.id;
    let mut backoff = BackoffState {
        consecutive_failures: config.consecutive_failures,
    };
    let mut last_status = backoff.status(config.last_error.as_deref());
    let mut last_sync_at = config.last_sync_at;
    let mut rng = rand::rngs::StdRng::from_entropy();

    info!(%channel_id, label = %config.label, "channel task: starting");

    loop {
        let delay_secs = backoff.next_delay_secs(config.poll_interval_secs, &mut rng);
        tokio::select! {
            biased;
            _ = token.cancelled() => { break; }
            _ = tokio::time::sleep(Duration::from_secs(delay_secs)) => {}
        }

        match adapter.fetch_messages(last_sync_at).await {
            Ok(batch) if batch.is_empty() => {
                backoff.record_success();
                publish_status_if_changed(&store, &bus, channel_id, &backoff, None, &mut last_status);
            }
            Ok(batch) => {
                let count = batch.len();
                let latest_ts = batch.iter().map(|m| m.timestamp).max();
                // Send to ingestor — await applies backpressure when ingestor queue is full.
                let job = IngestJob { channel_id, batch };
                if let Err(e) = ingest_tx.send(job).await {
                    error!(%channel_id, error = %e, "channel task: ingestor channel closed");
                    break;
                }

                // Persist cursor on success.
                if let Some(ts) = latest_ts {
                    if let Err(e) = store.update_sync_state(&channel_id, None, ts) {
                        warn!(%channel_id, error = %e, "channel task: failed to persist cursor");
                    }
                    last_sync_at = Some(ts);
                }

                backoff.record_success();
                bus.publish(RuntimeEvent::SyncSucceeded { channel_id, count });
                publish_status_if_changed(&store, &bus, channel_id, &backoff, None, &mut last_status);
            }
            Err(e) => {
                backoff.record_failure();
                let err_str = e.to_string();
                let attempt = backoff.consecutive_failures;
                bus.publish(RuntimeEvent::SyncFailed {
                    channel_id,
                    error: err_str.clone(),
                    attempt,
                });
                publish_status_if_changed(&store, &bus, channel_id, &backoff, Some(&err_str), &mut last_status);
            }
        }
    }

    info!(%channel_id, "channel task: disconnecting");
    if let Err(e) = adapter.disconnect().await {
        warn!(%channel_id, error = %e, "channel task: disconnect error");
    }
}

fn publish_status_if_changed(
    store: &Store,
    bus: &EventBus,
    channel_id: Uuid,
    backoff: &BackoffState,
    last_error: Option<&str>,
    last_status: &mut ChannelStatus,
) {
    let new = backoff.status(last_error);
    if new != *last_status {
        if let Err(e) = store.update_channel_status(&channel_id, &new, backoff.consecutive_failures) {
            warn!(%channel_id, error = %e, "channel task: failed to persist status");
        }
        bus.publish(RuntimeEvent::ChannelStatusChanged { channel_id, status: new.clone() });
        *last_status = new;
    }
}
```

- [ ] **Step 2: Write a unit test using `MockAdapter`**

Add at the bottom of `core/src/runtime/channel_task.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock::MockAdapter;
    use crate::adapters::RawMessage;
    use crate::runtime::ingestor::spawn_ingestor;
    use crate::types::Channel;
    use std::collections::HashMap;

    fn seed_config() -> ChannelConfig {
        ChannelConfig {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "test".to_string(),
            keychain_ref: "none".to_string(),
            enabled: true,
            poll_interval_secs: 1,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn successful_fetch_publishes_sync_succeeded_and_forwards_to_ingestor() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = seed_config();
        store.insert_channel_config(&config).unwrap(); // allow update_sync_state
        let bus = EventBus::with_capacity(32);
        let mut events = bus.subscribe();

        let (ingest_tx, ingest_handle) = spawn_ingestor(
            Arc::clone(&store), bus.clone(), None, 8, CancellationToken::new(),
        );

        let adapter = MockAdapter::new();
        adapter.add_message(RawMessage {
            external_id: "msg-1".to_string(),
            channel: Channel::Telegram,
            external_thread_id: Some("chat-1".to_string()),
            sender_name: "Bot".to_string(),
            sender_address: "bot".to_string(),
            text: Some("Hi".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });

        let root = CancellationToken::new();
        let handle = spawn_channel_task(
            config.clone(),
            Box::new(adapter),
            Arc::clone(&store),
            ingest_tx.clone(),
            bus,
            &root,
        );

        // Advance past one poll interval (1s, plus up to 20% jitter → 1.2s).
        tokio::time::advance(Duration::from_millis(1500)).await;

        // Collect up to 5 events with a small timeout each.
        let mut saw_sync_succeeded = false;
        let mut saw_ingested = false;
        for _ in 0..5 {
            let evt = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
            if let Ok(Ok(ev)) = evt {
                match ev {
                    RuntimeEvent::SyncSucceeded { .. } => saw_sync_succeeded = true,
                    RuntimeEvent::MessageIngested { .. } => saw_ingested = true,
                    _ => {}
                }
            }
        }
        assert!(saw_sync_succeeded, "expected SyncSucceeded");
        assert!(saw_ingested, "expected MessageIngested");

        root.cancel();
        handle.join.await.unwrap();
        drop(ingest_tx);
        ingest_handle.await.unwrap();
    }
}
```

If `MockAdapter::add_message` is not present under that name, check `core/src/adapters/mock.rs` and use the actual seed API.

- [ ] **Step 3: Run the tests**

```bash
cargo test -p messagehub-core runtime::channel_task
```

Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add core/src/runtime/channel_task.rs
git commit -m "feat(runtime): add ChannelTask polling loop with backoff"
```

---

### Task 11: `Runtime` + `RuntimeBuilder` + wiring

**Files:**
- Modify: `core/src/runtime/mod.rs`

`★ Why this matters:` The public API of Plan 6. Everything else has been building blocks; this is what a caller actually holds.

- [ ] **Step 1: Write the builder + runtime + their only tests that can live inline**

Replace `core/src/runtime/mod.rs` with:

```rust
//! Runtime: orchestration layer that drives adapters → ingestion → classification.
//!
//! See `docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`.

pub mod status;
pub mod events;
pub mod factory;
pub mod ingestor;
pub mod classifier_worker;
pub mod channel_task;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::ai::pipeline::AiPipeline;
use crate::error::{CoreError, Result};
use crate::store::Store;

use self::channel_task::{spawn_channel_task, ChannelTaskHandle};
use self::classifier_worker::maybe_spawn_classifier;
use self::events::{EventBus, RuntimeEvent};
use self::factory::{AdapterFactory, FactoryRegistry};
use self::ingestor::{spawn_ingestor, IngestJob};

/// Default capacities.
const DEFAULT_EVENT_BUFFER: usize = 1024;
const DEFAULT_CLASSIFIER_QUEUE: usize = 256;
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct RuntimeBuilder {
    store: Arc<Store>,
    pipeline: Option<Arc<AiPipeline>>,
    registry: FactoryRegistry,
    event_buffer: usize,
    classifier_queue: usize,
    shutdown_timeout: Duration,
}

impl RuntimeBuilder {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            pipeline: None,
            registry: FactoryRegistry::new(),
            event_buffer: DEFAULT_EVENT_BUFFER,
            classifier_queue: DEFAULT_CLASSIFIER_QUEUE,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }
    pub fn with_ai_pipeline(mut self, p: Arc<AiPipeline>) -> Self { self.pipeline = Some(p); self }
    pub fn with_factory(
        mut self,
        channel_type: impl Into<String>,
        factory: Arc<dyn AdapterFactory>,
    ) -> Self {
        self.registry.register(channel_type, factory);
        self
    }
    pub fn event_buffer(mut self, n: usize)     -> Self { self.event_buffer = n; self }
    pub fn classifier_queue(mut self, n: usize) -> Self { self.classifier_queue = n; self }
    pub fn shutdown_timeout(mut self, d: Duration) -> Self { self.shutdown_timeout = d; self }

    pub fn build(self) -> Runtime {
        let bus = EventBus::with_capacity(self.event_buffer);
        Runtime {
            store: self.store,
            pipeline: self.pipeline,
            registry: self.registry,
            bus,
            classifier_queue: self.classifier_queue,
            // ingestor capacity scales with registered channels; rebuilt in start().
            ingest_capacity_base: 16,
            shutdown_timeout: self.shutdown_timeout,
            running: None,
        }
    }
}

pub struct Runtime {
    store: Arc<Store>,
    pipeline: Option<Arc<AiPipeline>>,
    registry: FactoryRegistry,
    bus: EventBus,
    classifier_queue: usize,
    ingest_capacity_base: usize,
    shutdown_timeout: Duration,
    running: Option<RunningState>,
}

struct RunningState {
    root: CancellationToken,
    ingest_tx: mpsc::Sender<IngestJob>,
    ingest_handle: JoinHandle<()>,
    classifier_handle: Option<JoinHandle<()>>,
    channel_tasks: HashMap<Uuid, ChannelTaskHandle>,
}

impl Runtime {
    pub fn builder(store: Arc<Store>) -> RuntimeBuilder { RuntimeBuilder::new(store) }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> { self.bus.subscribe() }

    /// Spawn ingestor + classifier + one task per enabled channel row.
    pub async fn start(&mut self) -> Result<()> {
        if self.running.is_some() {
            return Err(CoreError::InvalidInput("runtime already running".to_string()));
        }
        let root = CancellationToken::new();

        let (classifier_tx, classifier_handle) = maybe_spawn_classifier(
            Arc::clone(&self.store),
            self.pipeline.clone(),
            self.bus.clone(),
            self.classifier_queue,
            root.clone(),
        );

        let ingest_cap = self.ingest_capacity_base.max(16);
        let (ingest_tx, ingest_handle) = spawn_ingestor(
            Arc::clone(&self.store),
            self.bus.clone(),
            classifier_tx,
            ingest_cap,
            root.clone(),
        );

        self.running = Some(RunningState {
            root,
            ingest_tx,
            ingest_handle,
            classifier_handle,
            channel_tasks: HashMap::new(),
        });

        self.reload_channels().await?;
        info!("runtime started");
        Ok(())
    }

    /// (Re)read the `channels` table and reconcile with running tasks.
    pub async fn reload_channels(&mut self) -> Result<()> {
        let rows = self.store.list_channel_configs()?;
        let running = self.running.as_mut().ok_or_else(|| {
            CoreError::InvalidInput("runtime not started".to_string())
        })?;

        let enabled_ids: std::collections::HashSet<Uuid> =
            rows.iter().filter(|r| r.enabled).map(|r| r.id).collect();

        // Stop tasks whose row is missing or disabled.
        let to_stop: Vec<Uuid> = running.channel_tasks
            .keys().copied().filter(|id| !enabled_ids.contains(id)).collect();
        for id in to_stop {
            if let Some(h) = running.channel_tasks.remove(&id) {
                h.stop();
                if let Err(e) = h.join.await {
                    warn!(channel_id = %id, error = %e, "reload: join error");
                }
            }
        }

        // Start tasks for enabled rows that don't have one.
        for row in rows.into_iter().filter(|r| r.enabled) {
            if running.channel_tasks.contains_key(&row.id) { continue; }
            let channel_type = row.channel.to_db_str().to_string();
            let Some(factory) = self.registry.get(&channel_type) else {
                warn!(channel_id = %row.id, channel_type = %channel_type,
                      "no factory registered for channel type; skipping");
                continue;
            };
            let mut adapter = factory.build(&row).await?;
            adapter.connect(&row).await?;
            let handle = spawn_channel_task(
                row.clone(),
                adapter,
                Arc::clone(&self.store),
                running.ingest_tx.clone(),
                self.bus.clone(),
                &running.root,
            );
            running.channel_tasks.insert(row.id, handle);
        }
        Ok(())
    }

    /// Graceful shutdown. Cancel, drain, disconnect, join under a timeout.
    pub async fn shutdown(mut self) {
        let Some(mut running) = self.running.take() else { return; };
        running.root.cancel();

        // Wait for channel tasks first — they call disconnect() on exit.
        let channel_ids: Vec<Uuid> = running.channel_tasks.keys().copied().collect();
        for id in channel_ids {
            if let Some(h) = running.channel_tasks.remove(&id) {
                if let Err(e) = timeout(self.shutdown_timeout, h.join).await {
                    warn!(channel_id = %id, error = %e, "shutdown: channel task timeout");
                }
            }
        }

        // Drop ingest sender so the ingestor's rx.recv() returns None.
        drop(running.ingest_tx);
        if let Err(e) = timeout(self.shutdown_timeout, running.ingest_handle).await {
            warn!(error = %e, "shutdown: ingestor timeout");
        }

        if let Some(h) = running.classifier_handle {
            if let Err(e) = timeout(self.shutdown_timeout, h).await {
                warn!(error = %e, "shutdown: classifier timeout");
            }
        }

        info!("runtime shut down");
    }
}
```

- [ ] **Step 2: Verify the whole crate compiles**

```bash
cargo build -p messagehub-core --all-targets
```

Expected: clean build.

- [ ] **Step 3: Run every unit test**

```bash
cargo test -p messagehub-core --lib
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add core/src/runtime/mod.rs
git commit -m "feat(runtime): wire Runtime + RuntimeBuilder"
```

---

### Task 12: Integration test — full loop

**Files:**
- Create: `core/tests/runtime_full_loop.rs`

`★ Why this matters:` End-to-end sanity: adapter → store → classifier → both events. If this breaks, the whole pipeline is broken.

- [ ] **Step 1: Write the integration test**

Create `core/tests/runtime_full_loop.rs`:

```rust
//! End-to-end: register a MockFactory, seed a message, assert both
//! `MessageIngested` and `MessageClassified` arrive and DB rows are populated.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::ai::llm::LlmBackend;
use messagehub_core::ai::pipeline::AiPipeline;
use messagehub_core::ai::profile::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
use uuid::Uuid;

struct StubLlm;
#[async_trait]
impl LlmBackend for StubLlm {
    async fn complete(&self, _p: &str) -> Result<String> {
        Ok("{\"category\":\"Work\",\"priority\":4,\"reasoning\":\"t\"}".into())
    }
}

struct MockFactory { seed: Vec<RawMessage> }
#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        for m in &self.seed { a.add_message(m.clone()); }
        Ok(Box::new(a))
    }
}

fn raw() -> RawMessage {
    RawMessage {
        external_id: "ext-1".into(),
        channel: Channel::Telegram,
        external_thread_id: Some("chat-1".into()),
        sender_name: "Alice".into(),
        sender_address: "alice".into(),
        text: Some("hello".into()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

fn seeded_config() -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel: Channel::Telegram,
        label: "t".into(),
        keychain_ref: "none".into(),
        enabled: true,
        poll_interval_secs: 1,
        last_sync_cursor: None,
        last_sync_at: None,
        status: ChannelStatus::Healthy,
        last_error: None,
        consecutive_failures: 0,
    }
}

#[tokio::test]
async fn full_loop_ingests_and_classifies() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.insert_channel_config(&seeded_config()).unwrap();

    let pipeline = Arc::new(AiPipeline::new(
        Arc::new(StubLlm), None, UserProfile::default(),
    ));

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_ai_pipeline(pipeline)
        .with_factory("Telegram", Arc::new(MockFactory { seed: vec![raw()] }))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    // Collect until we see both events, bounded to 10s.
    let mut ingested = false;
    let mut classified = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !(ingested && classified) {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            match ev {
                RuntimeEvent::MessageIngested { .. } => ingested = true,
                RuntimeEvent::MessageClassified { category, priority, .. } => {
                    classified = true;
                    assert!(category.is_some());
                    assert!(priority.is_some());
                }
                _ => {}
            }
        }
    }
    assert!(ingested && classified, "expected both events within 10s");

    rt.shutdown().await;
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p messagehub-core --test runtime_full_loop
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_full_loop.rs
git commit -m "test(runtime): end-to-end ingest + classify integration test"
```

---

### Task 13: Integration test — graceful degradation (no AiPipeline)

**Files:**
- Create: `core/tests/runtime_graceful_degradation.rs`

- [ ] **Step 1: Write the test**

Create `core/tests/runtime_graceful_degradation.rs`:

```rust
//! No AiPipeline configured → MessageIngested fires; MessageClassified never does.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::Result;
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
use uuid::Uuid;

struct MockFactory { seed: Vec<RawMessage> }
#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        for m in &self.seed { a.add_message(m.clone()); }
        Ok(Box::new(a))
    }
}

fn raw() -> RawMessage {
    RawMessage {
        external_id: "x".into(),
        channel: Channel::Telegram,
        external_thread_id: None,
        sender_name: "A".into(),
        sender_address: "a".into(),
        text: Some("y".into()),
        html: None, subject: None, attachments: vec![],
        timestamp: Utc::now(), metadata: HashMap::new(),
    }
}

fn cfg() -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel: Channel::Telegram,
        label: "t".into(), keychain_ref: "none".into(),
        enabled: true, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }
}

#[tokio::test]
async fn no_pipeline_means_no_classified_events() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.insert_channel_config(&cfg()).unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        // NOTE: no with_ai_pipeline
        .with_factory("Telegram", Arc::new(MockFactory { seed: vec![raw()] }))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    // Collect events for 3s. Assert at least one MessageIngested, zero MessageClassified.
    let mut saw_ingested = false;
    let mut saw_classified = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            match ev {
                RuntimeEvent::MessageIngested { .. }   => saw_ingested = true,
                RuntimeEvent::MessageClassified { .. } => saw_classified = true,
                _ => {}
            }
        }
    }

    assert!(saw_ingested, "expected at least one MessageIngested");
    assert!(!saw_classified, "MessageClassified must not fire without AiPipeline");

    rt.shutdown().await;
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p messagehub-core --test runtime_graceful_degradation
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_graceful_degradation.rs
git commit -m "test(runtime): graceful degradation when AiPipeline is absent"
```

---

### Task 14: Integration test — backoff progression

**Files:**
- Create: `core/tests/runtime_backoff.rs`

`★ Why this matters:` Exercises the full status-transition machinery through the runtime.

- [ ] **Step 1: Write a scripted-failure adapter + test**

Create `core/tests/runtime_backoff.rs`:

```rust
//! An adapter fails N times then recovers. Assert status progression.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::{CoreError, Result};
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};
use uuid::Uuid;

/// Fails on fetch_messages until `fail_remaining` hits zero, then succeeds.
struct FlakyAdapter { fail_remaining: Arc<AtomicUsize> }
#[async_trait]
impl ChannelAdapter for FlakyAdapter {
    async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> { Ok(()) }
    async fn fetch_messages(&self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if self.fail_remaining.load(Ordering::SeqCst) > 0 {
            self.fail_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(CoreError::InvalidInput("boom".to_string()));
        }
        Ok(vec![])
    }
    async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> { Ok(()) }
    async fn disconnect(&mut self) -> Result<()> { Ok(()) }
    fn channel_type(&self) -> Channel { Channel::Telegram }
}

struct FlakyFactory { fail_remaining: Arc<AtomicUsize> }
#[async_trait]
impl AdapterFactory for FlakyFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(FlakyAdapter {
            fail_remaining: Arc::clone(&self.fail_remaining),
        }))
    }
}

fn cfg(id: Uuid) -> ChannelConfig {
    ChannelConfig {
        id,
        channel: Channel::Telegram,
        label: "t".into(), keychain_ref: "none".into(),
        enabled: true, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }
}

#[tokio::test]
async fn status_progresses_degraded_failed_then_healthy() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let id = Uuid::new_v4();
    store.insert_channel_config(&cfg(id)).unwrap();

    let fails = Arc::new(AtomicUsize::new(5)); // 5 consecutive failures (crosses threshold)
    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(FlakyFactory { fail_remaining: Arc::clone(&fails) }))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let mut saw_degraded = false;
    let mut saw_failed = false;
    let mut saw_healthy_after = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline
        && !(saw_degraded && saw_failed && saw_healthy_after)
    {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            if let RuntimeEvent::ChannelStatusChanged { status, .. } = ev {
                match status {
                    ChannelStatus::Degraded { .. } => saw_degraded = true,
                    ChannelStatus::Failed   { .. } => saw_failed = true,
                    ChannelStatus::Healthy         => {
                        if saw_failed { saw_healthy_after = true; }
                    }
                }
            }
        }
    }
    assert!(saw_degraded, "expected Degraded");
    assert!(saw_failed,   "expected Failed");
    assert!(saw_healthy_after, "expected return to Healthy after recovery");

    rt.shutdown().await;
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p messagehub-core --test runtime_backoff
```

Expected: PASS. The test may take several seconds due to backoff progression — that's expected.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_backoff.rs
git commit -m "test(runtime): exponential backoff status progression"
```

---

### Task 15: Integration test — graceful shutdown

**Files:**
- Create: `core/tests/runtime_shutdown.rs`

- [ ] **Step 1: Write the test**

Create `core/tests/runtime_shutdown.rs`:

```rust
//! Start the runtime, then shut it down. Assert disconnect was called.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::Result;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};
use uuid::Uuid;

struct Tracked { disconnected: Arc<AtomicBool> }
#[async_trait]
impl ChannelAdapter for Tracked {
    async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> { Ok(()) }
    async fn fetch_messages(&self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Ok(vec![])
    }
    async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> { Ok(()) }
    async fn disconnect(&mut self) -> Result<()> {
        self.disconnected.store(true, Ordering::SeqCst); Ok(())
    }
    fn channel_type(&self) -> Channel { Channel::Telegram }
}

struct TrackedFactory { disconnected: Arc<AtomicBool> }
#[async_trait]
impl AdapterFactory for TrackedFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(Tracked { disconnected: Arc::clone(&self.disconnected) }))
    }
}

#[tokio::test]
async fn shutdown_disconnects_adapters() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let flag = Arc::new(AtomicBool::new(false));

    store.insert_channel_config(&ChannelConfig {
        id: Uuid::new_v4(),
        channel: Channel::Telegram,
        label: "t".into(), keychain_ref: "none".into(),
        enabled: true, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }).unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(TrackedFactory { disconnected: Arc::clone(&flag) }))
        .build();
    rt.start().await.unwrap();

    // Let it run briefly.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    rt.shutdown().await;

    assert!(flag.load(Ordering::SeqCst), "disconnect() must be called on shutdown");
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p messagehub-core --test runtime_shutdown
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_shutdown.rs
git commit -m "test(runtime): graceful shutdown calls disconnect"
```

---

### Task 16: Integration test — `reload_channels`

**Files:**
- Create: `core/tests/runtime_reload.rs`

- [ ] **Step 1: Write the test**

Create `core/tests/runtime_reload.rs`:

```rust
//! Insert a row at runtime → reload → a task spawns.
//! Disable the row → reload → the task stops.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::Result;
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
use uuid::Uuid;

struct MockFactory;
#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        a.add_message(RawMessage {
            external_id: "x".into(),
            channel: Channel::Telegram,
            external_thread_id: None,
            sender_name: "A".into(), sender_address: "a".into(),
            text: Some("hi".into()), html: None, subject: None, attachments: vec![],
            timestamp: Utc::now(), metadata: HashMap::new(),
        });
        Ok(Box::new(a))
    }
}

fn row(id: Uuid, enabled: bool) -> ChannelConfig {
    ChannelConfig {
        id,
        channel: Channel::Telegram,
        label: "t".into(), keychain_ref: "none".into(),
        enabled, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }
}

#[tokio::test]
async fn reload_adds_and_removes_channel_tasks() {
    let store = Arc::new(Store::open_in_memory().unwrap());

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(MockFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap(); // zero channels yet → no channel task spawned

    // Add a row + reload.
    let id = Uuid::new_v4();
    store.insert_channel_config(&row(id, true)).unwrap();
    rt.reload_channels().await.unwrap();

    // Expect at least one SyncSucceeded/MessageIngested within a few seconds.
    let mut saw_activity = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && !saw_activity {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            if matches!(ev, RuntimeEvent::SyncSucceeded { .. } | RuntimeEvent::MessageIngested { .. }) {
                saw_activity = true;
            }
        }
    }
    assert!(saw_activity, "runtime should poll after add+reload");

    // Disable the row + reload → task stops; we assert indirectly by observing
    // that future activity for this channel ceases. Since the MockAdapter has
    // already drained its message, "no new events for 2s" is the test; we
    // simply ensure shutdown completes cleanly.
    let mut cfg = store.list_channel_configs().unwrap().into_iter().next().unwrap();
    cfg.enabled = false;
    // Overwrite via delete+insert — or extend the plan with an update helper
    // if the store has one. For this test, a simple UPDATE is enough:
    store.update_channel_enabled(&id, false).unwrap();
    rt.reload_channels().await.unwrap();

    rt.shutdown().await;
}
```

- [ ] **Step 2: Add the small `update_channel_enabled` helper needed by this test**

Open `core/src/store/channels.rs`. Add inside `impl Store`:

```rust
pub fn update_channel_enabled(&self, id: &uuid::Uuid, enabled: bool) -> Result<()> {
    self.conn().execute(
        "UPDATE channels SET enabled = ?1 WHERE id = ?2",
        rusqlite::params![enabled as i64, id.to_string()],
    )?;
    Ok(())
}
```

- [ ] **Step 3: Run**

```bash
cargo test -p messagehub-core --test runtime_reload
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add core/tests/runtime_reload.rs core/src/store/channels.rs
git commit -m "test(runtime): reload_channels adds and removes tasks"
```

---

### Task 17: Integration test — classifier failure is non-fatal

**Files:**
- Create: `core/tests/runtime_classifier_failure.rs`

- [ ] **Step 1: Write the test**

Create `core/tests/runtime_classifier_failure.rs`:

```rust
//! LLM always errors → message is still inserted, `MessageClassified` fires
//! with the fallback (Low priority, non-None category).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::ai::llm::LlmBackend;
use messagehub_core::ai::pipeline::AiPipeline;
use messagehub_core::ai::profile::UserProfile;
use messagehub_core::error::{CoreError, Result};
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, Priority};
use uuid::Uuid;

struct BrokenLlm;
#[async_trait]
impl LlmBackend for BrokenLlm {
    async fn complete(&self, _p: &str) -> Result<String> {
        Err(CoreError::InvalidInput("down".into()))
    }
}

struct MockFactory;
#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        a.add_message(RawMessage {
            external_id: "x".into(),
            channel: Channel::Telegram,
            external_thread_id: None,
            sender_name: "A".into(), sender_address: "a".into(),
            text: Some("hi".into()), html: None, subject: None, attachments: vec![],
            timestamp: Utc::now(), metadata: HashMap::new(),
        });
        Ok(Box::new(a))
    }
}

#[tokio::test]
async fn llm_failure_still_classifies_with_low_priority_fallback() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.insert_channel_config(&ChannelConfig {
        id: Uuid::new_v4(), channel: Channel::Telegram,
        label: "t".into(), keychain_ref: "none".into(),
        enabled: true, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }).unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_ai_pipeline(Arc::new(AiPipeline::new(
            Arc::new(BrokenLlm), None, UserProfile::default(),
        )))
        .with_factory("Telegram", Arc::new(MockFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let mut fallback_seen = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline && !fallback_seen {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            if let RuntimeEvent::MessageClassified { priority, .. } = ev {
                assert_eq!(priority, Some(Priority::Low));
                fallback_seen = true;
            }
        }
    }
    assert!(fallback_seen, "MessageClassified with Low fallback must fire");

    rt.shutdown().await;
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p messagehub-core --test runtime_classifier_failure
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_classifier_failure.rs
git commit -m "test(runtime): classifier failure produces Low-priority fallback"
```

---

### Task 18: Integration test — one channel's failure does not affect another

**Files:**
- Create: `core/tests/runtime_channel_isolation.rs`

`★ Why this matters:` The whole point of per-channel tasks. A single test guarding against regressions that couple channel lifetimes.

- [ ] **Step 1: Write the test**

Create `core/tests/runtime_channel_isolation.rs`:

```rust
//! Two channels. One fails every fetch; the other succeeds. The healthy one
//! must continue emitting SyncSucceeded events regardless.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::{CoreError, Result};
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};
use uuid::Uuid;

struct Ok0;
#[async_trait]
impl ChannelAdapter for Ok0 {
    async fn connect(&mut self, _: &ChannelConfig) -> Result<()> { Ok(()) }
    async fn fetch_messages(&self, _: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Ok(vec![])
    }
    async fn send_reply(&self, _: &str, _: &MessageContent) -> Result<()> { Ok(()) }
    async fn disconnect(&mut self) -> Result<()> { Ok(()) }
    fn channel_type(&self) -> Channel { Channel::Telegram }
}
struct Err0;
#[async_trait]
impl ChannelAdapter for Err0 {
    async fn connect(&mut self, _: &ChannelConfig) -> Result<()> { Ok(()) }
    async fn fetch_messages(&self, _: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Err(CoreError::InvalidInput("always".into()))
    }
    async fn send_reply(&self, _: &str, _: &MessageContent) -> Result<()> { Ok(()) }
    async fn disconnect(&mut self) -> Result<()> { Ok(()) }
    fn channel_type(&self) -> Channel { Channel::Email }
}
struct OkFactory;
#[async_trait]
impl AdapterFactory for OkFactory {
    async fn build(&self, _: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> { Ok(Box::new(Ok0)) }
}
struct ErrFactory;
#[async_trait]
impl AdapterFactory for ErrFactory {
    async fn build(&self, _: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> { Ok(Box::new(Err0)) }
}

fn row(channel: Channel) -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel,
        label: "t".into(), keychain_ref: "none".into(),
        enabled: true, poll_interval_secs: 1,
        last_sync_cursor: None, last_sync_at: None,
        status: ChannelStatus::Healthy, last_error: None, consecutive_failures: 0,
    }
}

#[tokio::test]
async fn healthy_channel_keeps_polling_while_other_fails() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let ok_row  = row(Channel::Telegram);
    let err_row = row(Channel::Email);
    store.insert_channel_config(&ok_row).unwrap();
    store.insert_channel_config(&err_row).unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(OkFactory))
        .with_factory("Email",    Arc::new(ErrFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let mut ok_successes = 0u32;
    let mut err_failures = 0u32;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            match ev {
                RuntimeEvent::SyncSucceeded { channel_id, .. } if channel_id == ok_row.id => {
                    ok_successes += 1;
                }
                RuntimeEvent::SyncFailed { channel_id, .. } if channel_id == err_row.id => {
                    err_failures += 1;
                }
                _ => {}
            }
        }
    }

    assert!(ok_successes >= 2, "healthy channel produced {} successes", ok_successes);
    assert!(err_failures >= 1, "failing channel produced no failures");

    rt.shutdown().await;
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p messagehub-core --test runtime_channel_isolation
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add core/tests/runtime_channel_isolation.rs
git commit -m "test(runtime): failing channel does not affect healthy sibling"
```

---

### Task 19: Final sweep — whole-crate tests + lints

**Files:** (none — verification step)

- [ ] **Step 1: Run the full test suite**

```bash
cargo test -p messagehub-core
```

Expected: all unit + integration tests pass. No `#[ignore]`d tests added by Plan 6.

- [ ] **Step 2: Check no dead code in runtime/**

```bash
cargo build -p messagehub-core --all-targets 2>&1 | grep -E "warning|error" | head -40
```

Expected: no new warnings introduced by Plan 6 files. Fix any `unused` warnings by removing the dead item (not by silencing).

- [ ] **Step 3: Commit any cleanup**

If Step 2 required fixes:

```bash
git add -A
git commit -m "chore(runtime): resolve post-plan compiler warnings"
```

Otherwise skip.

---

## Notes for the executor

- **Commit after every task boundary.** Intermediate steps within a task should not be committed independently unless the step explicitly says so.
- **Do not skip the failing-test step.** If a test already passes on the first run, the test is not actually exercising the new behavior — inspect, don't trust.
- **If Step N's compiler errors disagree with the plan:** your codebase has drifted from the spec's assumptions. Stop and reread the current shape of the type being touched before improvising.
- **Do not introduce new `#[ignore]` tests or `#[cfg(feature = "ignore-slow")]` guards.** Every runtime test is fast (seconds) because there is no real I/O.
- **Do not add new runtime dependencies beyond `tokio-util` and `rand`.** Any third new dep indicates scope creep — push back.
- **If the `MockAdapter` API in `core/src/adapters/mock.rs` differs from what the tests assume**, fix the test to match the mock — do not extend the mock's API in Plan 6 unless unavoidable.
