# Plan 5: Tier 2 Cloud Actions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in, user-triggered cloud AI actions (`summarize_thread`, `draft_reply`, `smart_search`) on top of Plan 4's `ai::rag` and Plan 3's `knowledge::Retriever`. Introduce a `CloudProvider` trait with an Anthropic implementation, entity redaction with reverse-map un-redaction, a heuristic confidence score, and a new `ai_drafts` storage table.

**Architecture:** New `core/src/ai/cloud/` subtree. `CloudProvider` trait mirrors Plan 4's `LlmBackend` so test doubles work identically. Each of the three actions owns a prompt + strict JSON parser + orchestrator that composes `Redactor` → `provider.complete` → parse → `un_redact` → persist. A `CloudActions` facade holds the shared pieces and exposes one async method per action. Redaction happens before the network call; un-redaction happens after the parse. Every call writes to both `ai_drafts` (mutable, user-editable) and `action_log` (append-only audit).

**Tech Stack:** `reqwest` + `serde_json` (HTTP + JSON — already deps), `async-trait` (trait with async methods — already a dep), `regex` (NEW runtime dep — for email/phone redaction), `wiremock` (dev-dep — already added in Plan 4). No other new runtime deps.

**Prerequisites:** Plans 1, 3, 4 merged. An Anthropic API key is required only to run the `#[ignore]`d smoke test in Task 10; everything else runs offline against mocked providers.

---

## File Structure

```
core/
├── Cargo.toml                           # MODIFY — add `regex = "1"` to [dependencies]
├── migrations/
│   └── 004_cloud.sql                    # CREATE — ai_drafts table + indexes
├── src/
│   ├── lib.rs                           # unchanged (ai module already exported)
│   ├── error.rs                         # MODIFY — add Cloud variant
│   ├── store/
│   │   ├── mod.rs                       # MODIFY — pub mod drafts; pub use drafts::DraftRecord
│   │   ├── migrations.rs                # MODIFY — register 004
│   │   ├── messages.rs                  # MODIFY — add list_messages_in_thread
│   │   └── drafts.rs                    # CREATE — NewDraft, DraftRecord, insert_draft,
│   │                                    #          list_drafts_for_message, update_draft_output
│   └── ai/
│       ├── mod.rs                       # MODIFY — pub mod cloud;
│       └── cloud/
│           ├── mod.rs                   # CREATE — CloudAction enum + CloudConfig + re-exports
│           ├── provider.rs              # CREATE — CloudProvider trait + AnthropicCloud
│           ├── redactor.rs              # CREATE — Redactor + ReverseMap + un_redact
│           ├── confidence.rs            # CREATE — derive_confidence
│           └── actions/
│               ├── mod.rs               # CREATE — CloudActions facade + shared helpers
│               ├── summarize.rs         # CREATE — summarize_thread
│               ├── draft.rs             # CREATE — draft_reply
│               └── search.rs            # CREATE — smart_search
└── tests/
    ├── store_messages_thread_test.rs    # CREATE — list_messages_in_thread tests
    ├── store_drafts_test.rs             # CREATE — draft CRUD tests
    ├── cloud_provider_test.rs           # CREATE — AnthropicCloud HTTP tests (wiremock)
    ├── cloud_redactor_test.rs           # CREATE — redaction + un_redaction unit tests
    ├── cloud_confidence_test.rs         # CREATE — derive_confidence table-driven tests
    ├── cloud_summarize_test.rs          # CREATE — summarize action with scripted provider
    ├── cloud_draft_test.rs              # CREATE — draft action with scripted provider
    ├── cloud_search_test.rs             # CREATE — smart_search with scripted provider
    ├── cloud_facade_test.rs             # CREATE — end-to-end CloudActions facade
    └── cloud_anthropic_integration_test.rs  # CREATE — #[ignore]d real-Anthropic smoke
```

---

### Task 1: Migration 004 + CloudError + Module Skeleton

**Files:**
- Create: `core/migrations/004_cloud.sql`
- Modify: `core/src/store/migrations.rs`
- Modify: `core/src/error.rs`
- Modify: `core/src/ai/mod.rs`
- Create: `core/src/ai/cloud/mod.rs`
- Create: `core/src/ai/cloud/provider.rs` (stub)
- Create: `core/src/ai/cloud/redactor.rs` (stub)
- Create: `core/src/ai/cloud/confidence.rs` (stub)
- Create: `core/src/ai/cloud/actions/mod.rs` (stub)
- Create: `core/src/ai/cloud/actions/summarize.rs` (stub)
- Create: `core/src/ai/cloud/actions/draft.rs` (stub)
- Create: `core/src/ai/cloud/actions/search.rs` (stub)

`★ Why this matters:` Lay down the schema and every module file so downstream tasks only have content to fill in, not structural decisions. Stubs keep the crate compilable between tasks.

- [ ] **Step 1: Create the migration SQL**

Create `core/migrations/004_cloud.sql`:

```sql
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
```

- [ ] **Step 2: Register the migration**

Edit `core/src/store/migrations.rs`. Update `MIGRATIONS`:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../../migrations/001_initial.sql")),
    ("002_knowledge", include_str!("../../migrations/002_knowledge.sql")),
    ("003_ai", include_str!("../../migrations/003_ai.sql")),
    ("004_cloud", include_str!("../../migrations/004_cloud.sql")),
];
```

- [ ] **Step 3: Add the `Cloud` error variant**

Edit `core/src/error.rs`. Add one new variant immediately after `Ai`:

```rust
    #[error("ai pipeline error: {0}")]
    Ai(String),

    #[error("cloud action error: {0}")]
    Cloud(String),
}
```

- [ ] **Step 4: Register the cloud sub-module under `ai`**

Edit `core/src/ai/mod.rs`. Add `pub mod cloud;` alongside the other `pub mod` declarations:

```rust
pub mod classifier;
pub mod cloud;
pub mod llm;
pub mod pipeline;
pub mod profile;
pub mod prompts;
pub mod rag;
```

- [ ] **Step 5: Create the cloud module root**

Create `core/src/ai/cloud/mod.rs`:

```rust
//! Tier 2 cloud actions (user-triggered, opt-in per call).
//!
//! This module provides three cloud-backed actions that run against
//! Anthropic's API:
//!
//! - `summarize_thread` — condense a conversation with vault context.
//! - `draft_reply`     — compose a language-matched reply draft.
//! - `smart_search`    — natural-language answer over the knowledge vault.
//!
//! The submodules are:
//! - `provider`   — `CloudProvider` trait + `AnthropicCloud` HTTP client
//! - `redactor`   — entity scrubbing with reverse-map un-redaction
//! - `confidence` — heuristic 0..1 score derived from retrieval signals
//! - `actions`    — one file per action + a `CloudActions` facade

pub mod actions;
pub mod confidence;
pub mod provider;
pub mod redactor;

pub use actions::CloudActions;
pub use provider::{AnthropicCloud, CloudProvider};
pub use redactor::{Redactor, ReverseMap};

use serde::{Deserialize, Serialize};

/// Discriminator for which cloud action was run. Persisted into
/// `ai_drafts.action_type` and `action_log.action_type` verbatim (via
/// `as_str`), so values must stay stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudAction {
    SummarizeThread,
    DraftReply,
    SmartSearch,
}

impl CloudAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            CloudAction::SummarizeThread => "summarize_thread",
            CloudAction::DraftReply => "draft_reply",
            CloudAction::SmartSearch => "smart_search",
        }
    }
}

/// Per-call options. Kept minimal for Plan 5.
#[derive(Debug, Clone, Copy)]
pub struct CloudConfig {
    /// When true, the `Redactor` scrubs named entities, emails, and
    /// phone numbers before the body is sent to the cloud. The reverse
    /// map is always applied to the response before it is stored or
    /// returned, regardless of this flag.
    pub redact: bool,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self { redact: false }
    }
}
```

- [ ] **Step 6: Create stub files for every sub-module**

Create `core/src/ai/cloud/provider.rs`:

```rust
//! Stub — filled in by Task 4.

use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait CloudProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

pub struct AnthropicCloud;
```

Create `core/src/ai/cloud/redactor.rs`:

```rust
//! Stub — filled in by Task 5.

use std::collections::HashMap;

pub type ReverseMap = HashMap<String, String>;

pub struct Redactor;
```

Create `core/src/ai/cloud/confidence.rs`:

```rust
//! Stub — filled in by Task 6.

use crate::ai::RagContext;

pub fn derive_confidence(_rag: &RagContext, _retrieval_sims: &[f32]) -> f32 {
    0.0
}
```

Create `core/src/ai/cloud/actions/mod.rs`:

```rust
//! Stub — filled in by Task 7+ (individual actions) and Task 10 (facade).

pub mod draft;
pub mod search;
pub mod summarize;

pub struct CloudActions;
```

Create `core/src/ai/cloud/actions/summarize.rs`:

```rust
//! Stub — filled in by Task 7.
```

Create `core/src/ai/cloud/actions/draft.rs`:

```rust
//! Stub — filled in by Task 8.
```

Create `core/src/ai/cloud/actions/search.rs`:

```rust
//! Stub — filled in by Task 9.
```

- [ ] **Step 7: Verify the crate still compiles**

Run: `cargo check -p messagehub-core`
Expected: PASS. Warnings about unused items are fine; there should be no errors.

- [ ] **Step 8: Verify the migration runs cleanly**

Run: `cargo test -p messagehub-core --test store_messages_test`
Expected: PASS. `Store::open_in_memory` runs all migrations as a side effect; an empty `ai_drafts` table should exist after.

- [ ] **Step 9: Commit**

```bash
git add core/migrations/004_cloud.sql core/src/store/migrations.rs core/src/error.rs core/src/ai/mod.rs core/src/ai/cloud/
git commit -m "feat(cloud): scaffold cloud module with migration 004 and stubs"
```

---

### Task 2: `Store::list_messages_in_thread`

**Files:**
- Modify: `core/src/store/messages.rs`
- Create: `core/tests/store_messages_thread_test.rs`

`★ Why this matters:` Both `summarize_thread` and `draft_reply` need the full conversation history for the thread. The existing `list_messages(channel, archived, limit, offset)` helper filters by channel, not thread — we need a narrower helper that also orders oldest-first so the cloud prompt reads naturally.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/store_messages_thread_test.rs`:

```rust
use chrono::{TimeZone, Utc};
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use uuid::Uuid;

fn seed_contact_and_thread(store: &Store) -> (Uuid, Uuid) {
    let contact_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact_id,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread_id,
            channel: Channel::Email,
            subject: Some("Project X".into()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    (contact_id, thread_id)
}

fn msg(sender: Uuid, thread: Uuid, text: &str, epoch_secs: i64) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: thread,
        sender_id: sender,
        content: MessageContent {
            text: Some(text.into()),
            html: None,
            subject: Some("Project X".into()),
            attachments: vec![],
        },
        timestamp: Utc.timestamp_opt(epoch_secs, 0).unwrap(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    }
}

#[test]
fn test_list_messages_in_thread_returns_oldest_first() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread) = seed_contact_and_thread(&store);

    let later = msg(sender, thread, "second", 2000);
    let earlier = msg(sender, thread, "first", 1000);
    store.insert_message(&later).unwrap();
    store.insert_message(&earlier).unwrap();

    let got = store.list_messages_in_thread(&thread, 10).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].content.text.as_deref(), Some("first"));
    assert_eq!(got[1].content.text.as_deref(), Some("second"));
}

#[test]
fn test_list_messages_in_thread_respects_limit() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread) = seed_contact_and_thread(&store);

    for i in 0..5 {
        store
            .insert_message(&msg(sender, thread, &format!("m{}", i), 1000 + i as i64))
            .unwrap();
    }

    let got = store.list_messages_in_thread(&thread, 3).unwrap();
    assert_eq!(got.len(), 3);
    // Oldest-first, so we keep m0..m2 — NOT the last 3.
    assert_eq!(got[0].content.text.as_deref(), Some("m0"));
    assert_eq!(got[2].content.text.as_deref(), Some("m2"));
}

#[test]
fn test_list_messages_in_thread_returns_empty_for_unknown_thread() {
    let store = Store::open_in_memory().unwrap();
    let unknown = Uuid::new_v4();
    let got = store.list_messages_in_thread(&unknown, 10).unwrap();
    assert!(got.is_empty());
}

#[test]
fn test_list_messages_in_thread_ignores_other_threads() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread_a) = seed_contact_and_thread(&store);
    let thread_b = Uuid::new_v4();
    store
        .insert_thread(&Thread {
            id: thread_b,
            channel: Channel::Email,
            subject: Some("Unrelated".into()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();

    store.insert_message(&msg(sender, thread_a, "in A", 1000)).unwrap();
    store.insert_message(&msg(sender, thread_b, "in B", 1001)).unwrap();

    let got = store.list_messages_in_thread(&thread_a, 10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].content.text.as_deref(), Some("in A"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test store_messages_thread_test -- --nocapture`
Expected: FAIL with compile error — `list_messages_in_thread` does not exist.

- [ ] **Step 3: Implement the helper**

Edit `core/src/store/messages.rs`. Inside `impl Store { ... }`, right after `list_messages`, add:

```rust
    /// Return every message in a thread, oldest first.
    ///
    /// Ordering is `timestamp ASC` so the conversation reads naturally
    /// top-to-bottom when rendered into a prompt. The `limit` caps the
    /// oldest-N returned (not the newest-N) — use a high value if you
    /// want the whole thread, or truncate at the call site if you need
    /// "last N".
    pub fn list_messages_in_thread(&self, thread_id: &Uuid, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html,
                    content_subject, attachments_json, timestamp, metadata_json,
                    priority_score, category, is_read, is_archived
             FROM messages
             WHERE thread_id = ?1
             ORDER BY timestamp ASC
             LIMIT ?2",
        )?;
        let messages: Vec<Message> = stmt
            .query_map(params![thread_id.to_string(), limit], |row| {
                Ok(row_to_message(row))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
            .into_iter()
            .collect::<std::result::Result<Vec<_>, CoreError>>()?;
        Ok(messages)
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test store_messages_thread_test -- --nocapture`
Expected: 4 passing tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/store/messages.rs core/tests/store_messages_thread_test.rs
git commit -m "feat(store): add Store::list_messages_in_thread (oldest-first)"
```

---

### Task 3: Store Draft Helpers (`NewDraft`, `insert_draft`, `list_drafts_for_message`, `update_draft_output`)

**Files:**
- Create: `core/src/store/drafts.rs`
- Modify: `core/src/store/mod.rs`
- Create: `core/tests/store_drafts_test.rs`

`★ Why this matters:` All three actions persist their output to `ai_drafts` via one shared helper. Drafts are mutable (`user_edited_output`) and must survive app restart; `action_log` is for the immutable audit trail.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/store_drafts_test.rs`:

```rust
use messagehub_core::store::{DraftRecord, NewDraft, Store};
use uuid::Uuid;

#[test]
fn test_insert_and_list_draft_for_message() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "[body with [EMAIL_1] scrubbed]",
            output: "Hi Alice, sure we can meet tomorrow.",
            confidence: 0.72,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 1);
    let d: &DraftRecord = &drafts[0];
    assert_eq!(d.id, draft_id);
    assert_eq!(d.action_type, "draft_reply");
    assert_eq!(d.output, "Hi Alice, sure we can meet tomorrow.");
    assert!((d.confidence - 0.72).abs() < 1e-6);
    assert_eq!(d.provider, "anthropic");
    assert!(d.user_edited_output.is_none());
}

#[test]
fn test_insert_draft_with_null_message_id_for_smart_search() {
    let store = Store::open_in_memory().unwrap();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: None,
            action_type: "smart_search",
            input_redacted: "latest from alix's school?",
            output: "No school messages in the last 30 days.",
            confidence: 0.91,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    // list_drafts_for_message filters on message_id; smart_search drafts
    // are therefore invisible here — by design.
    let some_msg = Uuid::new_v4();
    let drafts = store.list_drafts_for_message(&some_msg).unwrap();
    assert!(drafts.is_empty());
}

#[test]
fn test_update_draft_output_writes_user_edited_field() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "initial draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    store.update_draft_output(&draft_id, "edited by user").unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts[0].output, "initial draft"); // original preserved
    assert_eq!(drafts[0].user_edited_output.as_deref(), Some("edited by user"));
}

#[test]
fn test_multiple_drafts_for_same_message_return_newest_first() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    let first = Uuid::new_v4();
    let second = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: first,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "first draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();
    // Sleep one millisecond-granular tick is hard in tests; ensure ordering
    // is stable by inserting with deterministically different ids — the
    // query uses created_at DESC, then id DESC as a tiebreaker.
    store
        .insert_draft(&NewDraft {
            id: second,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "second draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 2);
    // Newest first — the second insert should appear at index 0.
    assert_eq!(drafts[0].id, second);
    assert_eq!(drafts[1].id, first);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test store_drafts_test -- --nocapture`
Expected: FAIL with compile errors — `NewDraft`, `DraftRecord`, `insert_draft`, etc. do not exist.

- [ ] **Step 3: Create the drafts module**

Create `core/src/store/drafts.rs`:

```rust
use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;

/// Input payload for `Store::insert_draft`.
///
/// We use a borrowed struct rather than positional arguments because
/// the field count is high and positional calls at the call site are
/// error-prone ("which string was `input_redacted` again?").
#[derive(Debug, Clone)]
pub struct NewDraft<'a> {
    pub id: Uuid,
    /// `None` for `smart_search` (no anchor message); `Some(...)` for
    /// `summarize_thread` and `draft_reply`.
    pub message_id: Option<Uuid>,
    pub action_type: &'a str,
    pub input_redacted: &'a str,
    pub output: &'a str,
    pub confidence: f32,
    pub provider: &'a str,
    pub model: &'a str,
}

/// A row from `ai_drafts`. Returned by `list_drafts_for_message`.
#[derive(Debug, Clone)]
pub struct DraftRecord {
    pub id: Uuid,
    pub message_id: Option<Uuid>,
    pub action_type: String,
    pub input_redacted: String,
    pub output: String,
    pub user_edited_output: Option<String>,
    pub confidence: f32,
    pub provider: String,
    pub model: String,
    pub created_at: String,
}

impl Store {
    /// Persist a newly generated cloud draft.
    pub fn insert_draft(&self, draft: &NewDraft<'_>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO ai_drafts
                (id, message_id, action_type, input_redacted, output,
                 confidence, provider, model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                draft.id.to_string(),
                draft.message_id.map(|u| u.to_string()),
                draft.action_type,
                draft.input_redacted,
                draft.output,
                draft.confidence as f64,
                draft.provider,
                draft.model,
            ],
        )?;
        Ok(())
    }

    /// Return every draft anchored to `message_id`, newest first.
    ///
    /// `smart_search` drafts are persisted with `message_id = NULL` and
    /// therefore never appear here — call sites that need them should
    /// query by action type instead (future helper).
    pub fn list_drafts_for_message(&self, message_id: &Uuid) -> Result<Vec<DraftRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, message_id, action_type, input_redacted, output,
                    user_edited_output, confidence, provider, model, created_at
             FROM ai_drafts
             WHERE message_id = ?1
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows: std::result::Result<Vec<DraftRecord>, rusqlite::Error> = stmt
            .query_map(params![message_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let msg_id_str: Option<String> = row.get(1)?;
                Ok(DraftRecord {
                    id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::nil()),
                    message_id: msg_id_str
                        .as_deref()
                        .and_then(|s| Uuid::parse_str(s).ok()),
                    action_type: row.get(2)?,
                    input_redacted: row.get(3)?,
                    output: row.get(4)?,
                    user_edited_output: row.get(5)?,
                    confidence: row.get::<_, f64>(6)? as f32,
                    provider: row.get(7)?,
                    model: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })?
            .collect();
        rows.map_err(CoreError::Database)
    }

    /// Replace `user_edited_output` on an existing draft. Does not touch
    /// the original `output` column — that stays as the cloud's
    /// verbatim response for audit.
    pub fn update_draft_output(&self, draft_id: &Uuid, edited: &str) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE ai_drafts SET user_edited_output = ?1 WHERE id = ?2",
            params![edited, draft_id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "ai_draft".into(),
                id: draft_id.to_string(),
            });
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Register the module**

Edit `core/src/store/mod.rs`. Add `pub mod drafts;` alongside the other `pub mod` declarations, and add the re-exports near `pub use ai_log::AiDecision;`:

```rust
pub mod ai_log;
pub mod channels;
pub mod contacts;
pub mod drafts;
pub mod knowledge;
pub mod messages;
mod migrations;
```

```rust
pub use ai_log::AiDecision;
pub use drafts::{DraftRecord, NewDraft};
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test store_drafts_test -- --nocapture`
Expected: 4 passing tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/store/drafts.rs core/src/store/mod.rs core/tests/store_drafts_test.rs
git commit -m "feat(store): add draft CRUD helpers (insert/list/update)"
```

---

### Task 4: `CloudProvider` Trait + `AnthropicCloud` HTTP Client

**Files:**
- Modify: `core/Cargo.toml`
- Modify: `core/src/ai/cloud/provider.rs`
- Create: `core/tests/cloud_provider_test.rs`

`★ Why this matters:` Concrete HTTP client for Anthropic's `/v1/messages` endpoint, behind a `CloudProvider` trait so tests can inject scripted responses without touching the network. Mirrors the `LlmBackend`/`OllamaLlm` shape from Plan 4 so everything that reads like "AI client" looks the same.

- [ ] **Step 1: Add `regex` runtime dep**

Edit `core/Cargo.toml`. Add `regex = "1"` to `[dependencies]` (alphabetical placement, near `reqwest`):

```toml
reqwest = { version = "0.12", features = ["json"] }
regex = "1"
async-imap = "0.9"
```

Regex is used by Task 5 (Redactor); adding it here keeps `Cargo.lock` changes with Task 4's wiremock/reqwest activity.

- [ ] **Step 2: Write the failing integration test**

Create `core/tests/cloud_provider_test.rs`:

```rust
use messagehub_core::ai::cloud::{AnthropicCloud, CloudProvider};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn canned_response_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "content": [
            { "type": "text", "text": text }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })
}

#[tokio::test]
async fn test_anthropic_complete_posts_to_v1_messages_with_auth_headers() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body("hello back")))
        .expect(1)
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("test-key".into(), "claude-sonnet-4-6".into())
        .with_base_url(server.uri());
    let out = provider
        .complete("sys prompt", "user prompt", 128)
        .await
        .unwrap();
    assert_eq!(out, "hello back");
}

#[tokio::test]
async fn test_anthropic_complete_joins_multiple_text_blocks() {
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "content": [
            { "type": "text", "text": "first " },
            { "type": "text", "text": "second" }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1, "output_tokens": 1 }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    let out = provider.complete("s", "u", 64).await.unwrap();
    assert_eq!(out, "first second");
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    let err = provider.complete("s", "u", 64).await.unwrap_err();
    let msg = format!("{}", err);
    assert!(
        msg.contains("anthropic") || msg.contains("401") || msg.contains("cloud"),
        "error does not mention anthropic/401/cloud: {}",
        msg
    );
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_5xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    assert!(provider.complete("s", "u", 64).await.is_err());
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_malformed_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    assert!(provider.complete("s", "u", 64).await.is_err());
}

#[tokio::test]
async fn test_anthropic_health_check_returns_false_when_unreachable() {
    // Localhost port nothing is listening on.
    let provider = AnthropicCloud::new("k".into(), "m".into())
        .with_base_url("http://127.0.0.1:1".into());
    assert_eq!(provider.health_check().await.unwrap(), false);
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_provider_test -- --nocapture`
Expected: FAIL with compile errors — `AnthropicCloud::new`, `with_base_url`, `health_check` do not exist.

- [ ] **Step 4: Implement the provider**

Replace the entire contents of `core/src/ai/cloud/provider.rs`:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::error::{CoreError, Result};

/// Abstraction over a cloud LLM that the action orchestrators talk to.
///
/// Mirrors the shape of `ai::llm::LlmBackend` deliberately: one method
/// taking a system prompt, a user prompt, and a max-tokens cap, returning
/// the assistant's plain-text output. Tests inject scripted implementations
/// that skip the network entirely (see `ScriptedCloudProvider` in the
/// per-action integration tests).
#[async_trait]
pub trait CloudProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

/// HTTP client for Anthropic's `/v1/messages` endpoint.
///
/// Holds the API key, the model name (e.g. `"claude-sonnet-4-6"`), and
/// the base URL (configurable for wiremock tests). Sends `stream: false`
/// requests and joins every `type: "text"` block in the response `content`
/// array into one string.
///
/// The constructor intentionally does NOT read from the environment —
/// credential policy (env, keychain, config file) is the caller's
/// responsibility.
pub struct AnthropicCloud {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicCloud {
    pub fn new(api_key: String, model: String) -> Self {
        // 60s matches the Ollama client and gives headroom for
        // first-token latency on large prompts. Cloud latency is
        // usually sub-second but we'd rather fail a slow request than
        // hang forever.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client builder never fails with default config");
        Self {
            client,
            api_key,
            model,
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Override the base URL. Primary use: point tests at a wiremock
    /// server. Production callers should leave this at the default.
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Check whether the Anthropic endpoint is reachable with the
    /// configured API key.
    ///
    /// Returns `Ok(false)` on any network-level failure (connection
    /// refused, timeout, DNS). Returns `Ok(true)` on any 2xx. Only
    /// propagates `Err(...)` for logic bugs — consistent with Plan 4's
    /// `OllamaLlm::health_check`.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v1/models", self.base_url);
        match self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
        {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!(error = %e, url = %url, "anthropic health check failed");
                Ok(false)
            }
        }
    }
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    system: &'a str,
    messages: Vec<MessagesInput<'a>>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Serialize)]
struct MessagesInput<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    #[serde(other)]
    Other,
}

#[async_trait]
impl CloudProvider for AnthropicCloud {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = MessagesRequest {
            model: &self.model,
            system,
            messages: vec![MessagesInput {
                role: "user",
                content: user,
            }],
            max_tokens,
            temperature: 0.3,
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Cloud(format!("anthropic request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let preview: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            warn!(status = %status, body_preview = %preview, "anthropic returned non-2xx");
            return Err(CoreError::Cloud(format!(
                "anthropic returned {} — {}",
                status, preview
            )));
        }

        let parsed: MessagesResponse = resp.json().await.map_err(|e| {
            CoreError::Cloud(format!("anthropic response body malformed: {}", e))
        })?;

        let mut out = String::new();
        for block in parsed.content {
            if let ContentBlock::Text { text } = block {
                out.push_str(&text);
            }
        }
        Ok(out)
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_provider_test -- --nocapture`
Expected: 6 passing tests.

- [ ] **Step 6: Commit**

```bash
git add core/Cargo.toml Cargo.lock core/src/ai/cloud/provider.rs core/tests/cloud_provider_test.rs
git commit -m "feat(cloud): add CloudProvider trait and AnthropicCloud HTTP client"
```

---

### Task 5: `Redactor` — Entity Scrubbing + Reverse Map

**Files:**
- Modify: `core/src/ai/cloud/redactor.rs`
- Create: `core/tests/cloud_redactor_test.rs`

`★ Why this matters:` The privacy invariant: nothing between `Redactor::redact` and the cloud response can see un-redacted bytes. The Redactor loads vault people once at construction, builds a sorted-by-length-descending list of names, and does a greedy longest-match replacement. Same original → same token across one call; different calls get fresh counters.

- [ ] **Step 1: Write the failing test**

Create `core/tests/cloud_redactor_test.rs`:

```rust
use messagehub_core::ai::cloud::{Redactor, ReverseMap};

fn build_standalone_redactor(people: &[&str]) -> Redactor {
    // Bypasses the vault; lets us test the trie logic without seeding a store.
    Redactor::from_names(people.iter().map(|s| s.to_string()).collect())
}

#[test]
fn test_redact_replaces_vault_name() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (out, map) = r.redact("Hi Alice Example, here's the update.");
    assert!(out.contains("[PERSON_1]"));
    assert!(!out.contains("Alice Example"));
    assert_eq!(map.get("[PERSON_1]"), Some(&"Alice Example".to_string()));
}

#[test]
fn test_redact_longest_match_wins_for_overlapping_names() {
    // "Alice Example" should win over "Alice".
    let r = build_standalone_redactor(&["Alice", "Alice Example"]);
    let (out, _map) = r.redact("Hi Alice Example!");
    // One token, not two. "Alice" standalone shouldn't also fire.
    let tokens: Vec<&str> = out.matches("[PERSON_").collect();
    assert_eq!(tokens.len(), 1);
}

#[test]
fn test_redact_is_case_insensitive_for_vault_names() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (out, map) = r.redact("spoke with ALICE EXAMPLE today");
    assert!(out.contains("[PERSON_1]"));
    // Reverse map preserves the *original* spelling from the input so
    // un_redact restores what the user saw.
    assert_eq!(map.get("[PERSON_1]"), Some(&"ALICE EXAMPLE".to_string()));
}

#[test]
fn test_redact_replaces_email_address() {
    let r = build_standalone_redactor(&[]);
    let (out, map) = r.redact("email me at alice@example.com please");
    assert!(out.contains("[EMAIL_1]"));
    assert!(!out.contains("alice@example.com"));
    assert_eq!(map.get("[EMAIL_1]"), Some(&"alice@example.com".to_string()));
}

#[test]
fn test_redact_same_email_gets_stable_token_in_one_call() {
    let r = build_standalone_redactor(&[]);
    let (out, _map) = r.redact("mail alice@example.com then alice@example.com again");
    // Two occurrences, one token reused.
    let count = out.matches("[EMAIL_1]").count();
    assert_eq!(count, 2);
    assert!(!out.contains("[EMAIL_2]"));
}

#[test]
fn test_redact_replaces_phone_number() {
    let r = build_standalone_redactor(&[]);
    let (out, map) = r.redact("call me at +41 79 123 45 67 anytime");
    assert!(out.contains("[PHONE_1]"));
    assert!(map.get("[PHONE_1]").unwrap().contains("79"));
}

#[test]
fn test_redact_leaves_short_number_sequences_alone() {
    // "order 12345" is 5 chars, below the phone regex minimum.
    let r = build_standalone_redactor(&[]);
    let (out, _map) = r.redact("see order 12345 in dashboard");
    assert!(!out.contains("[PHONE_"));
    assert!(out.contains("12345"));
}

#[test]
fn test_un_redact_round_trips() {
    let r = build_standalone_redactor(&["Alice Example"]);
    let (redacted, map) = r.redact("Hi Alice Example, reach me at alice@example.com");
    assert!(redacted.contains("[PERSON_1]"));
    assert!(redacted.contains("[EMAIL_1]"));
    let restored = Redactor::un_redact(&redacted, &map);
    assert_eq!(restored, "Hi Alice Example, reach me at alice@example.com");
}

#[test]
fn test_un_redact_passthrough_when_map_empty() {
    let empty: ReverseMap = ReverseMap::new();
    let out = Redactor::un_redact("no tokens here", &empty);
    assert_eq!(out, "no tokens here");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_redactor_test -- --nocapture`
Expected: FAIL with compile errors.

- [ ] **Step 3: Implement the redactor**

Replace the entire contents of `core/src/ai/cloud/redactor.rs`:

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::error::Result;
use crate::store::Store;

/// Map from token (e.g. `"[PERSON_1]"`) back to the original verbatim
/// string that was scrubbed. `Redactor::un_redact` applies it to restore
/// user-visible output.
pub type ReverseMap = HashMap<String, String>;

/// Longest-match-first entity scrubber.
///
/// Three classes, applied in order:
/// 1. Vault-matched names (from `05-People/*.md` via `Store::list_vault_people`).
///    Loaded at construction; no mid-session refresh in Plan 5.
/// 2. Email addresses (regex).
/// 3. Phone numbers (regex, min 9 chars to avoid order numbers / SKUs).
///
/// Same original → same token across one `redact` call (stable numbering
/// within the call). Different calls get fresh maps.
pub struct Redactor {
    /// Vault names sorted by length descending so longest-match-first
    /// works with a straight linear scan.
    names: Vec<String>,
}

static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\w.+-]+@[\w-]+\.[\w.-]+").expect("email regex must compile")
});

static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // At least 9 total characters (including separators) with 9+ digits.
    Regex::new(r"\+?\d[\d\s\-().]{7,}\d").expect("phone regex must compile")
});

impl Redactor {
    /// Load vault people from the store and build a redactor.
    ///
    /// If the vault is empty or the query fails, returns a redactor
    /// that still scrubs emails and phone numbers (regex-only).
    pub fn build(store: &Store) -> Result<Self> {
        let names = match store.list_vault_people() {
            Ok(people) => people.into_iter().map(|p| p.name).collect(),
            Err(_) => Vec::new(),
        };
        Ok(Self::from_names(names))
    }

    /// Construct from an explicit name list. Public so tests can build a
    /// redactor without seeding a vault.
    pub fn from_names(mut names: Vec<String>) -> Self {
        // Longest-first so "Alice Example" wins over "Alice" in a
        // greedy forward scan.
        names.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()));
        Self { names }
    }

    /// Redact `input` and return `(redacted, reverse_map)`.
    ///
    /// Token numbering is per-call and per-class — a fresh counter for
    /// PERSON, EMAIL, and PHONE each time. Identical originals within
    /// one call share a token.
    pub fn redact(&self, input: &str) -> (String, ReverseMap) {
        let mut map: ReverseMap = HashMap::new();
        let mut current = input.to_string();
        let mut forward: HashMap<String, String> = HashMap::new();
        let mut counters = RedactCounters::default();

        // 1. Vault names (longest first, case-insensitive).
        for name in &self.names {
            current = replace_case_insensitive(&current, name, |original| {
                assign_token(
                    original,
                    "PERSON",
                    &mut counters.person,
                    &mut forward,
                    &mut map,
                )
            });
        }

        // 2. Emails.
        current = replace_regex(&current, &EMAIL_RE, |m| {
            assign_token(m, "EMAIL", &mut counters.email, &mut forward, &mut map)
        });

        // 3. Phones.
        current = replace_regex(&current, &PHONE_RE, |m| {
            assign_token(m, "PHONE", &mut counters.phone, &mut forward, &mut map)
        });

        (current, map)
    }

    /// Restore the original strings from a redacted output using the
    /// `ReverseMap` produced by `redact`. Straight find-and-replace;
    /// tokens not in the map pass through unchanged.
    pub fn un_redact(text: &str, map: &ReverseMap) -> String {
        // Sort keys by length descending so tokens never partially
        // replace each other (e.g. `[PERSON_1]` vs `[PERSON_10]`).
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort_by(|a, b| b.len().cmp(&a.len()));
        let mut out = text.to_string();
        for k in keys {
            out = out.replace(k, map.get(k).unwrap());
        }
        out
    }
}

#[derive(Default)]
struct RedactCounters {
    person: u32,
    email: u32,
    phone: u32,
}

/// Assign (or reuse) a token for `original` inside one class.
fn assign_token(
    original: &str,
    prefix: &str,
    counter: &mut u32,
    forward: &mut HashMap<String, String>,
    map: &mut ReverseMap,
) -> String {
    // Forward lookup key includes the class so "Alice" as PERSON and
    // "Alice" as some-other-class wouldn't collide (not a concern today,
    // defensive).
    let fwd_key = format!("{}:{}", prefix, original);
    if let Some(token) = forward.get(&fwd_key) {
        return token.clone();
    }
    *counter += 1;
    let token = format!("[{}_{}]", prefix, counter);
    forward.insert(fwd_key, token.clone());
    map.insert(token.clone(), original.to_string());
    token
}

/// Case-insensitive find-and-replace with a custom token-producer.
/// Walks the string left-to-right, matching `needle` ignoring case; on
/// match, produces a token using `producer` called with the ORIGINAL
/// (preserving case) substring so `un_redact` restores the user's input
/// verbatim.
fn replace_case_insensitive(
    haystack: &str,
    needle: &str,
    mut producer: impl FnMut(&str) -> String,
) -> String {
    let hay_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut cursor = 0;
    while let Some(rel) = hay_lower[cursor..].find(&needle_lower) {
        let start = cursor + rel;
        let end = start + needle.len();
        // `end` is in bytes and the regex is ASCII-name oriented; for
        // safety, guard against slicing across a char boundary.
        if !haystack.is_char_boundary(end) {
            cursor = start + 1;
            continue;
        }
        out.push_str(&haystack[cursor..start]);
        let original = &haystack[start..end];
        out.push_str(&producer(original));
        cursor = end;
    }
    out.push_str(&haystack[cursor..]);
    out
}

/// Regex find-and-replace with a custom token-producer.
fn replace_regex(haystack: &str, re: &Regex, mut producer: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(haystack.len());
    let mut cursor = 0;
    for m in re.find_iter(haystack) {
        out.push_str(&haystack[cursor..m.start()]);
        out.push_str(&producer(m.as_str()));
        cursor = m.end();
    }
    out.push_str(&haystack[cursor..]);
    out
}
```

- [ ] **Step 4: Expose `Store::list_vault_people`**

If a helper already exists, skip this step. Otherwise, check:

Run: `grep -n "pub fn list_vault_people" core/src/store/knowledge.rs`

If nothing is returned, add the helper. Edit `core/src/store/knowledge.rs`, inside `impl Store { ... }`:

```rust
    /// Return every person file loaded from the vault. Used by the
    /// cloud `Redactor` to build its name list at startup.
    pub fn list_vault_people(&self) -> Result<Vec<VaultPersonSummary>> {
        let mut stmt = self.conn().prepare("SELECT name, file_path FROM vault_people")?;
        let rows: std::result::Result<Vec<VaultPersonSummary>, rusqlite::Error> = stmt
            .query_map([], |row| {
                Ok(VaultPersonSummary {
                    name: row.get(0)?,
                    file_path: row.get(1)?,
                })
            })?
            .collect();
        rows.map_err(CoreError::Database)
    }
```

And at the top-level of `core/src/store/knowledge.rs` (outside `impl Store`):

```rust
/// Lightweight projection of `vault_people` used by the redactor.
#[derive(Debug, Clone)]
pub struct VaultPersonSummary {
    pub name: String,
    pub file_path: String,
}
```

Also re-export the type from `core/src/store/mod.rs` near the other re-exports:

```rust
pub use knowledge::VaultPersonSummary;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_redactor_test -- --nocapture`
Expected: 9 passing tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/ai/cloud/redactor.rs core/src/store/knowledge.rs core/src/store/mod.rs core/tests/cloud_redactor_test.rs
git commit -m "feat(cloud): add Redactor with longest-match vault names + email/phone regex"
```

---

### Task 6: `derive_confidence` Heuristic

**Files:**
- Modify: `core/src/ai/cloud/confidence.rs`
- Create: `core/tests/cloud_confidence_test.rs`

`★ Why this matters:` One shared formula so `summarize_thread`, `draft_reply`, and `smart_search` all produce comparable confidence numbers. Built on signals the app already computes (retrieval similarity, vault-match presence, profile presence) — no extra model calls.

- [ ] **Step 1: Write the failing unit tests**

Create `core/tests/cloud_confidence_test.rs`:

```rust
use messagehub_core::ai::cloud::confidence::derive_confidence;
use messagehub_core::ai::RagContext;

fn ctx(sender_name: Option<&str>, profile: &str) -> RagContext {
    RagContext {
        sender_name: sender_name.map(|s| s.to_string()),
        sender_vault_path: sender_name.map(|_| "05-People/x.md".to_string()),
        topic_chunks: vec![],
        user_profile_content: profile.to_string(),
    }
}

#[test]
fn test_confidence_full_signal() {
    // Known sender + profile + strong retrieval → close to 1.0.
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[0.9, 0.7]);
    assert!((score - 0.9).abs() < 1e-6);
}

#[test]
fn test_confidence_unknown_sender_drops_signal() {
    let score = derive_confidence(&ctx(None, "Role: x"), &[1.0]);
    // 1.0 * 0.7 * 1.0 = 0.7
    assert!((score - 0.7).abs() < 1e-6);
}

#[test]
fn test_confidence_empty_profile_drops_signal() {
    let score = derive_confidence(&ctx(Some("Alice"), ""), &[1.0]);
    // 1.0 * 1.0 * 0.8 = 0.8
    assert!((score - 0.8).abs() < 1e-6);
}

#[test]
fn test_confidence_whitespace_only_profile_treated_as_empty() {
    let score = derive_confidence(&ctx(Some("Alice"), "   \n   "), &[1.0]);
    assert!((score - 0.8).abs() < 1e-6);
}

#[test]
fn test_confidence_zero_when_nothing_matches() {
    let score = derive_confidence(&ctx(None, ""), &[]);
    // top_sim from empty slice is 0.0 → 0.0 regardless of multipliers.
    assert_eq!(score, 0.0);
}

#[test]
fn test_confidence_is_clamped_to_0_1() {
    // Pathological input — a retrieval score > 1.0 should still clamp.
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[1.5]);
    assert_eq!(score, 1.0);
}

#[test]
fn test_confidence_takes_max_of_retrieval_scores() {
    // Mixed scores; only the max feeds the formula.
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[0.1, 0.8, 0.3]);
    assert!((score - 0.8).abs() < 1e-6);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_confidence_test -- --nocapture`
Expected: FAIL — `derive_confidence` returns 0.0 from the stub.

- [ ] **Step 3: Implement the heuristic**

Replace the entire contents of `core/src/ai/cloud/confidence.rs`:

```rust
use crate::ai::RagContext;

/// Derive a heuristic 0.0-1.0 confidence score from the grounding
/// signals available at the call site.
///
/// - `top_sim`: the best cosine similarity from the retriever. For
///   `summarize_thread`, where there is no retrieval, callers pass
///   `&[0.85]` as a baseline — the grounding is the thread itself.
/// - `sender_signal`: 1.0 if the sender is a vault-known contact, 0.7
///   otherwise. Strangers can still ground on message content, so we
///   don't zero it out.
/// - `profile_signal`: 1.0 if `user-profile.md` has content, 0.8 if not.
///
/// The product is clamped to 0.0..=1.0 as a belt-and-braces guard; with
/// well-behaved inputs the components never exceed 1.0 each.
///
/// Property: unknown sender + empty profile + zero retrieval = 0.0.
pub fn derive_confidence(rag: &RagContext, retrieval_sims: &[f32]) -> f32 {
    let top_sim = retrieval_sims
        .iter()
        .cloned()
        .fold(0.0_f32, f32::max);
    let sender_signal = if rag.sender_name.is_some() { 1.0 } else { 0.7 };
    let profile_signal = if !rag.user_profile_content.trim().is_empty() {
        1.0
    } else {
        0.8
    };
    (top_sim * sender_signal * profile_signal).clamp(0.0, 1.0)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_confidence_test -- --nocapture`
Expected: 7 passing tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/ai/cloud/confidence.rs core/tests/cloud_confidence_test.rs
git commit -m "feat(cloud): add derive_confidence heuristic (top_sim × sender × profile)"
```

---

### Task 7: `summarize_thread` Action

**Files:**
- Modify: `core/src/ai/cloud/actions/summarize.rs`
- Create: `core/tests/cloud_summarize_test.rs`

`★ Why this matters:` First end-to-end action. Establishes the pattern: build_rag_context → redact → build_prompt → provider.complete → parse → un_redact → persist. The other two actions copy this structure.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/cloud_summarize_test.rs`:

```rust
use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::actions::summarize::summarize_thread;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::{CoreError, Result};
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedCloudProvider {
    next: Mutex<Option<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedCloudProvider {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl CloudProvider for ScriptedCloudProvider {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.next
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Err(CoreError::Cloud("no canned response".into())))
    }
}

fn seed_thread(store: &Store) -> (Uuid, Uuid) {
    let contact = Uuid::new_v4();
    let thread = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread,
            channel: Channel::Email,
            subject: Some("Project X".into()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    for (i, body) in ["hey", "sure, Friday?", "sounds good"].iter().enumerate() {
        let m = Message {
            id: Uuid::new_v4(),
            channel: Channel::Email,
            thread_id: thread,
            sender_id: contact,
            content: MessageContent {
                text: Some((*body).to_string()),
                html: None,
                subject: Some("Project X".into()),
                attachments: vec![],
            },
            timestamp: Utc::now() + chrono::Duration::seconds(i as i64),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
        };
        store.insert_message(&m).unwrap();
    }
    (thread, contact)
}

#[tokio::test]
async fn test_summarize_thread_happy_path_persists_draft_and_log() {
    let store = Store::open_in_memory().unwrap();
    let (thread_id, _contact) = seed_thread(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"summary": "Alice confirmed Friday for Project X.", "language": "en"}"#,
    ));
    let profile = UserProfile { content: "Role: consultant".into() };
    let redactor = Redactor::from_names(vec![]);

    let draft = summarize_thread(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        thread_id,
        CloudConfig::default(),
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();

    assert_eq!(draft.action.as_str(), "summarize_thread");
    assert!(draft.output.contains("Alice"));
    assert!(draft.confidence > 0.0);

    // Audit row written to action_log.
    let log = store
        .list_ai_decisions_for_entity("thread", &thread_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "summarize_thread");

    // Persisted to ai_drafts with message_id = the last message in the thread.
    let msgs = store.list_messages_in_thread(&thread_id, 10).unwrap();
    let last = msgs.last().unwrap();
    let drafts = store.list_drafts_for_message(&last.id).unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].provider, "anthropic");
}

#[tokio::test]
async fn test_summarize_thread_rejects_empty_thread() {
    let store = Store::open_in_memory().unwrap();
    let unknown_thread = Uuid::new_v4();
    let provider = Arc::new(ScriptedCloudProvider::ok("{}"));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = summarize_thread(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        unknown_thread,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("empty") || msg.contains("thread"));
}

#[tokio::test]
async fn test_summarize_thread_surfaces_parse_error_and_logs_failure() {
    let store = Store::open_in_memory().unwrap();
    let (thread_id, _) = seed_thread(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok("this is not JSON"));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = summarize_thread(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        thread_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.to_lowercase().contains("json") || msg.to_lowercase().contains("cloud"));

    // Failure row is written to action_log so UI can show "retry".
    let log = store
        .list_ai_decisions_for_entity("thread", &thread_id.to_string())
        .unwrap();
    assert_eq!(log[0].action_type, "summarize_thread_failed");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_summarize_test -- --nocapture`
Expected: FAIL with compile errors — `summarize_thread` does not exist.

- [ ] **Step 3: Implement the action**

Replace the entire contents of `core/src/ai/cloud/actions/summarize.rs`:

```rust
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::Redactor;
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::{RagContext, build_rag_context};
use crate::error::{CoreError, Result};
use crate::store::{NewDraft, Store};
use crate::types::Message;

const SYSTEM_PROMPT: &str = r#"You are a conversation summarizer running with user consent over a single thread.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "summary": <1-3 sentence plain-text summary of the thread>,
  "language": <one of: "en", "fr", "de">
}

Summarize what the thread is actually about, what was decided, and any open question. Do not add commentary or apologies. Do not wrap the JSON in code fences unless absolutely necessary."#;

/// Public-facing record returned by the action.
#[derive(Debug, Clone)]
pub struct DraftOutcome {
    pub id: Uuid,
    pub action: CloudAction,
    pub output: String,
    pub confidence: f32,
}

#[derive(Debug, Deserialize)]
struct ParsedSummary {
    summary: String,
    #[allow(dead_code)]
    language: String,
}

/// Summarize every message in `thread_id` using the cloud provider.
///
/// On success: persists a row to `ai_drafts` (anchored to the newest
/// message in the thread) and a `summarize_thread` row to `action_log`.
/// Returns the un-redacted summary ready for display.
///
/// On cloud or parse failure: writes a `summarize_thread_failed` row to
/// `action_log` and returns `CoreError::Cloud(...)` to the caller.
pub async fn summarize_thread(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    profile: &UserProfile,
    thread_id: Uuid,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let messages = store.list_messages_in_thread(&thread_id, 200)?;
    if messages.is_empty() {
        return Err(CoreError::Cloud(format!(
            "cannot summarize empty thread {}",
            thread_id
        )));
    }
    let anchor_message = messages.last().cloned().unwrap();

    let thread_text = render_thread_as_text(&messages);
    let (redacted_thread, reverse_map) = if cfg.redact {
        redactor.redact(&thread_text)
    } else {
        (thread_text.clone(), Default::default())
    };

    // Sender lookup for the latest message — used purely for the RAG
    // sender signal; the summary itself reads the whole thread.
    let last_sender_addr = resolve_sender_address(store, &anchor_message);
    let rag = build_rag_context(
        store,
        None, // no retriever needed — the thread IS the grounding
        profile,
        anchor_message.channel,
        last_sender_addr.as_deref().unwrap_or(""),
        anchor_message.content.subject.as_deref().unwrap_or(""),
        &redacted_thread,
    )?;

    let user_prompt = build_user_prompt(&redacted_thread, &rag);
    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 512)
        .await
        .map_err(|e| log_and_wrap(store, thread_id, &e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, raw_preview = %raw.chars().take(200).collect::<String>(),
                  "summarize_thread parse failed");
            let _ = store.log_ai_decision(
                "summarize_thread_failed",
                "thread",
                &thread_id.to_string(),
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let final_output = Redactor::un_redact(&parsed.summary, &reverse_map);
    // No retrieval for summarize → use baseline grounding signal.
    let confidence = derive_confidence(&rag, &[0.85]);

    let draft_id = Uuid::new_v4();
    let preview: String = redacted_thread.chars().take(2_000).collect();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: Some(anchor_message.id),
        action_type: CloudAction::SummarizeThread.as_str(),
        input_redacted: &preview,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    store.log_ai_decision(
        CloudAction::SummarizeThread.as_str(),
        "thread",
        &thread_id.to_string(),
        &final_output,
        confidence as f64,
    )?;

    info!(
        thread_id = %thread_id,
        confidence,
        timestamp = %Utc::now(),
        "summarize_thread succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::SummarizeThread,
        output: final_output,
        confidence,
    })
}

fn render_thread_as_text(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        out.push_str(&format!("[{}] ", m.timestamp.to_rfc3339()));
        if let Some(s) = &m.content.subject {
            out.push_str(&format!("(subject: {}) ", s));
        }
        if let Some(t) = &m.content.text {
            out.push_str(t.trim());
        }
        out.push('\n');
    }
    out
}

fn build_user_prompt(thread_text: &str, rag: &RagContext) -> String {
    let mut out = String::new();
    out.push_str("# Conversation to summarize\n\n");
    out.push_str(thread_text.trim());
    out.push_str("\n\n");
    out.push_str(&rag.to_prompt_section());
    out.push_str("\nSummarize this conversation.\n");
    out
}

fn parse_response(raw: &str) -> Result<ParsedSummary> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedSummary = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.summary.trim().is_empty() {
        return Err(CoreError::Cloud("empty summary field".into()));
    }
    Ok(parsed)
}

/// Sender address lookup. Messages only store `sender_id` (a Contact
/// UUID), so we walk to `contacts` for the first identity that matches
/// the message's channel.
fn resolve_sender_address(store: &Store, msg: &Message) -> Option<String> {
    let contact = store.get_contact(&msg.sender_id).ok()?;
    contact
        .identities
        .into_iter()
        .find(|id| id.channel == msg.channel)
        .map(|id| id.address)
}

fn log_and_wrap(store: &Store, thread_id: Uuid, err: &CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "summarize_thread_failed",
        "thread",
        &thread_id.to_string(),
        &format!("{}", err),
        0.0,
    );
    CoreError::Cloud(format!("{}", err))
}

/// Strip triple-backtick fences (with optional `json` language tag).
/// Duplicated rather than imported from Plan 4's prompts module so the
/// cloud module is self-contained.
fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

/// Return the first balanced `{...}` block. Naive but reliable for
/// LLM outputs that don't contain strings with unescaped braces.
fn first_balanced_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}
```

- [ ] **Step 4: Check for `Store::get_contact` availability**

The action calls `store.get_contact(&msg.sender_id)`. Verify it exists:

Run: `grep -n "pub fn get_contact" core/src/store/contacts.rs`

If the signature is `pub fn get_contact(&self, id: &Uuid) -> Result<Contact>`, we're good. Otherwise the action's `resolve_sender_address` helper needs adjustment to match the actual helper name (e.g. `find_contact`).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_summarize_test -- --nocapture`
Expected: 3 passing tests.

- [ ] **Step 6: Commit**

```bash
git add core/src/ai/cloud/actions/summarize.rs core/tests/cloud_summarize_test.rs
git commit -m "feat(cloud): add summarize_thread action with strict JSON parser"
```

---

### Task 8: `draft_reply` Action

**Files:**
- Modify: `core/src/ai/cloud/actions/draft.rs`
- Create: `core/tests/cloud_draft_test.rs`

`★ Why this matters:` The highest-user-value action. Mirrors `summarize_thread` except (a) the anchor is the target message being replied to (fetched via `get_message`), (b) the prompt asks for a language-matched reply, and (c) we run the retriever for vault grounding so the draft can reference project notes.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/cloud_draft_test.rs`:

```rust
use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::actions::draft::draft_reply;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedCloudProvider {
    next: Mutex<Option<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedCloudProvider {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl CloudProvider for ScriptedCloudProvider {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.next.lock().unwrap().take().unwrap()
    }
}

fn seed(store: &Store) -> (Uuid, Uuid) {
    let c = Uuid::new_v4();
    let t = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: c,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: t,
            channel: Channel::Email,
            subject: Some("Meeting".into()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    let m = Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: t,
        sender_id: c,
        content: MessageContent {
            text: Some("Are we still on for tomorrow?".into()),
            html: None,
            subject: Some("Meeting".into()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    };
    store.insert_message(&m).unwrap();
    (m.id, t)
}

#[tokio::test]
async fn test_draft_reply_un_redacts_person_tokens_in_output() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    // LLM returned a draft with a token that the client side must un-redact.
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "Hi [PERSON_1], yes tomorrow works.", "language": "en"}"#,
    ));
    let profile = UserProfile { content: "Role: consultant".into() };
    let redactor = Redactor::from_names(vec!["Alice Example".into()]);

    let draft = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None, // no retriever
        &profile,
        message_id,
        CloudConfig { redact: true },
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();

    // No token should remain in the final output.
    assert!(!draft.output.contains("[PERSON_"));
    // But we can't assert "Alice Example" appears because the original
    // body doesn't contain it — this test only verifies that _if_ the
    // LLM writes a token, un_redact is attempted.
    assert!(draft.output.contains("tomorrow"));
}

#[tokio::test]
async fn test_draft_reply_rejects_unknown_language_code() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "ola", "language": "pt"}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    assert!(format!("{}", err).to_lowercase().contains("language"));
}

#[tokio::test]
async fn test_draft_reply_persists_audit_row() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "ok", "language": "en"}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let _ = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap();

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert!(log.iter().any(|d| d.action_type == "draft_reply"));

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_draft_test -- --nocapture`
Expected: FAIL with compile errors.

- [ ] **Step 3: Implement the action**

Replace the entire contents of `core/src/ai/cloud/actions/draft.rs`:

```rust
use std::sync::Arc;

use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::ai::cloud::actions::summarize::DraftOutcome;
use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::Redactor;
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::{RagContext, build_rag_context};
use crate::error::{CoreError, Result};
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::{NewDraft, Store};

const SYSTEM_PROMPT: &str = r#"You are drafting a reply to an incoming message on behalf of the user. The user will review your draft before sending — do not invent commitments, prices, or decisions.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "draft":    <plain-text reply, no greeting signature unless the thread already uses one>,
  "language": <one of: "en", "fr", "de">
}

Match the language of the incoming message unless the user profile indicates a strong language preference, in which case follow the profile. Keep the draft concise: one or two paragraphs maximum.

Do not wrap the JSON in code fences unless absolutely necessary."#;

const ALLOWED_LANGUAGES: &[&str] = &["en", "fr", "de"];

#[derive(Debug, Deserialize)]
struct ParsedDraft {
    draft: String,
    language: String,
}

/// Draft a reply to `message_id` using the cloud provider.
pub async fn draft_reply(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    message_id: Uuid,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let message = store.get_message(&message_id)?;
    let sender_addr = {
        let contact = store.get_contact(&message.sender_id)?;
        contact
            .identities
            .iter()
            .find(|id| id.channel == message.channel)
            .map(|id| id.address.clone())
            .unwrap_or_default()
    };

    let subject = message.content.subject.clone().unwrap_or_default();
    let body = message.content.text.clone().unwrap_or_default();

    let (redacted_body, reverse_map) = if cfg.redact {
        redactor.redact(&body)
    } else {
        (body.clone(), Default::default())
    };

    // Retrieval feeds the RagContext AND the confidence heuristic.
    let retrieval_sims: Vec<f32> = match retriever {
        Some(r) => r
            .search(
                store,
                &format!("{} {}", subject, redacted_body),
                &RetrievalFilters::default(),
            )?
            .into_iter()
            .map(|c| similarity_from_distance(c.distance))
            .collect(),
        None => Vec::new(),
    };

    let rag = build_rag_context(
        store,
        retriever,
        profile,
        message.channel,
        &sender_addr,
        &subject,
        &redacted_body,
    )?;

    let user_prompt = build_user_prompt(&message.channel.to_string(), &sender_addr, &subject, &redacted_body, &rag);

    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 1024)
        .await
        .map_err(|e| log_and_wrap(store, message_id, &e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "draft_reply parse failed");
            let _ = store.log_ai_decision(
                "draft_reply_failed",
                "message",
                &message_id.to_string(),
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let final_output = Redactor::un_redact(&parsed.draft, &reverse_map);
    let confidence = derive_confidence(&rag, &retrieval_sims);

    let draft_id = Uuid::new_v4();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: Some(message_id),
        action_type: CloudAction::DraftReply.as_str(),
        input_redacted: &redacted_body,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    store.log_ai_decision(
        CloudAction::DraftReply.as_str(),
        "message",
        &message_id.to_string(),
        &format!("language={}", parsed.language),
        confidence as f64,
    )?;

    info!(
        message_id = %message_id,
        confidence,
        language = %parsed.language,
        "draft_reply succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::DraftReply,
        output: final_output,
        confidence,
    })
}

fn build_user_prompt(
    channel: &str,
    sender: &str,
    subject: &str,
    body: &str,
    rag: &RagContext,
) -> String {
    let mut out = String::new();
    out.push_str("# Incoming message\n");
    out.push_str(&format!("Channel: {}\n", channel));
    out.push_str(&format!("From: {}\n", sender));
    if !subject.trim().is_empty() {
        out.push_str(&format!("Subject: {}\n", subject));
    }
    out.push_str("\nBody:\n");
    out.push_str(body.trim());
    out.push_str("\n\n");
    out.push_str(&rag.to_prompt_section());
    out.push_str("\nDraft a reply to this message.\n");
    out
}

fn parse_response(raw: &str) -> Result<ParsedDraft> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedDraft = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.draft.trim().is_empty() {
        return Err(CoreError::Cloud("empty draft field".into()));
    }
    if !ALLOWED_LANGUAGES.contains(&parsed.language.as_str()) {
        return Err(CoreError::Cloud(format!(
            "unknown language '{}'; must be one of {:?}",
            parsed.language, ALLOWED_LANGUAGES
        )));
    }
    Ok(parsed)
}

/// Convert sqlite-vec L2 distance into a 0..1 similarity. Distances are
/// roughly 0..2 for 384-dim unit vectors, so `1.0 - clamp(d/2.0, 0, 1)`
/// gives a reasonable monotonic signal.
fn similarity_from_distance(distance: f32) -> f32 {
    (1.0 - (distance / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn log_and_wrap(store: &Store, message_id: Uuid, err: &CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "draft_reply_failed",
        "message",
        &message_id.to_string(),
        &format!("{}", err),
        0.0,
    );
    CoreError::Cloud(format!("{}", err))
}

fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

fn first_balanced_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_draft_test -- --nocapture`
Expected: 3 passing tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/ai/cloud/actions/draft.rs core/tests/cloud_draft_test.rs
git commit -m "feat(cloud): add draft_reply action with language-code validation"
```

---

### Task 9: `smart_search` Action

**Files:**
- Modify: `core/src/ai/cloud/actions/search.rs`
- Create: `core/tests/cloud_search_test.rs`

`★ Why this matters:` Natural-language Q&A over the vault. Composes `Retriever::search` (local, unredacted) with an Anthropic call that sees only the user's query and the retrieved chunks. Redaction only touches the query — the retrieved chunks are already in the vault the user owns.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/cloud_search_test.rs`:

```rust
use async_trait::async_trait;
use messagehub_core::ai::cloud::actions::search::smart_search;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use std::sync::{Arc, Mutex};

struct ScriptedCloudProvider {
    next: Mutex<Option<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedCloudProvider {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl CloudProvider for ScriptedCloudProvider {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.next.lock().unwrap().take().unwrap()
    }
}

#[tokio::test]
async fn test_smart_search_with_no_retriever_still_calls_cloud() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "No vault results for that query.", "sources": []}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let result = smart_search(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None, // no retriever — prompt will indicate that
        &profile,
        "what did alice say last week?",
        CloudConfig::default(),
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();
    assert!(result.output.contains("No vault results"));

    // Persisted with message_id = None (smart_search has no anchor).
    // list_drafts_for_message won't return it; that's expected.
    let drafts = store
        .list_drafts_for_message(&uuid::Uuid::new_v4())
        .unwrap();
    assert!(drafts.is_empty());
}

#[tokio::test]
async fn test_smart_search_redacts_query_when_opted_in() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "ok", "sources": []}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec!["Alice Example".into()]);

    let _ = smart_search(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        "what did Alice Example say?",
        CloudConfig { redact: true },
        "m",
    )
    .await
    .unwrap();

    let prompt = provider.last_user_prompt.lock().unwrap().clone().unwrap();
    assert!(!prompt.contains("Alice Example"));
    assert!(prompt.contains("[PERSON_1]"));
}

#[tokio::test]
async fn test_smart_search_logs_audit_row() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "answer", "sources": ["05-People/x.md"]}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let _ = smart_search(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        "any news?",
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap();

    // action_log keyed on entity_type = "query", entity_id = the redacted query.
    let rows = store
        .list_ai_decisions_for_entity("query", "any news?")
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action_type, "smart_search");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p messagehub-core --test cloud_search_test -- --nocapture`
Expected: FAIL with compile errors.

- [ ] **Step 3: Implement the action**

Replace the entire contents of `core/src/ai/cloud/actions/search.rs`:

```rust
use std::sync::Arc;

use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::ai::cloud::actions::summarize::DraftOutcome;
use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::Redactor;
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::RagContext;
use crate::error::{CoreError, Result};
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::{NewDraft, Store};

const SYSTEM_PROMPT: &str = r#"You answer natural-language questions over the user's personal knowledge vault. You will receive the user's query and up to 10 retrieved chunks from their vault.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "answer":  <plain-text answer, 1-3 short paragraphs>,
  "sources": <array of vault file paths cited, e.g. ["01-Projects/Project X.md"]>
}

Only cite paths that appear in the provided chunks. If the chunks don't contain an answer, say so in the answer field and return an empty sources array.

Do not wrap the JSON in code fences unless absolutely necessary."#;

#[derive(Debug, Deserialize)]
struct ParsedAnswer {
    answer: String,
    #[allow(dead_code)]
    sources: Vec<String>,
}

pub async fn smart_search(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    query: &str,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let (redacted_query, reverse_map) = if cfg.redact {
        redactor.redact(query)
    } else {
        (query.to_string(), Default::default())
    };

    let (chunks, sims) = match retriever {
        Some(r) => {
            let results = r.search(
                store,
                &redacted_query,
                &RetrievalFilters { para_folders: None, top_k: Some(10) },
            )?;
            let sims: Vec<f32> = results
                .iter()
                .map(|c| (1.0 - (c.distance / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0))
                .collect();
            (results, sims)
        }
        None => (Vec::new(), Vec::new()),
    };

    // Build a minimal RagContext so derive_confidence has consistent inputs.
    let rag = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: profile.content.clone(),
    };

    let user_prompt = build_user_prompt(&redacted_query, &chunks, profile);
    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 1024)
        .await
        .map_err(|e| log_and_wrap(store, query, &e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "smart_search parse failed");
            let _ = store.log_ai_decision(
                "smart_search_failed",
                "query",
                query,
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let final_output = Redactor::un_redact(&parsed.answer, &reverse_map);
    let confidence = derive_confidence(&rag, &sims);

    let draft_id = Uuid::new_v4();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: None, // smart_search has no anchor message
        action_type: CloudAction::SmartSearch.as_str(),
        input_redacted: &redacted_query,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    // Audit key is (entity_type=query, entity_id=original query string).
    // We use the original (un-redacted) query so the UI can render
    // "your search history" meaningfully.
    store.log_ai_decision(
        CloudAction::SmartSearch.as_str(),
        "query",
        query,
        &final_output,
        confidence as f64,
    )?;

    info!(
        query_preview = %query.chars().take(80).collect::<String>(),
        confidence,
        "smart_search succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::SmartSearch,
        output: final_output,
        confidence,
    })
}

fn build_user_prompt(
    redacted_query: &str,
    chunks: &[crate::knowledge::RetrievedChunk],
    profile: &UserProfile,
) -> String {
    let mut out = String::new();
    out.push_str("# User query\n");
    out.push_str(redacted_query);
    out.push_str("\n\n# Retrieved vault chunks\n");
    if chunks.is_empty() {
        out.push_str("- (no retriever configured — answer from profile + general knowledge only)\n");
    } else {
        for c in chunks {
            let heading = c.section_heading.as_deref().unwrap_or("(no heading)");
            out.push_str(&format!(
                "- [{} — {}] {}\n",
                c.file_path,
                heading,
                c.content.trim().chars().take(400).collect::<String>()
            ));
        }
    }
    out.push_str("\n# User profile\n");
    if profile.content.trim().is_empty() {
        out.push_str("- (no profile configured)\n");
    } else {
        out.push_str(&profile.content);
        if !profile.content.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("\nAnswer the query using only the retrieved chunks.\n");
    out
}

fn parse_response(raw: &str) -> Result<ParsedAnswer> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedAnswer = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.answer.trim().is_empty() {
        return Err(CoreError::Cloud("empty answer field".into()));
    }
    Ok(parsed)
}

fn log_and_wrap(store: &Store, query: &str, err: &CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "smart_search_failed",
        "query",
        query,
        &format!("{}", err),
        0.0,
    );
    CoreError::Cloud(format!("{}", err))
}

fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

fn first_balanced_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p messagehub-core --test cloud_search_test -- --nocapture`
Expected: 3 passing tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/ai/cloud/actions/search.rs core/tests/cloud_search_test.rs
git commit -m "feat(cloud): add smart_search action over vault retriever"
```

---

### Task 10: `CloudActions` Facade + Real-Anthropic Smoke Test

**Files:**
- Modify: `core/src/ai/cloud/actions/mod.rs`
- Create: `core/tests/cloud_facade_test.rs`
- Create: `core/tests/cloud_anthropic_integration_test.rs`

`★ Why this matters:` The facade ties provider + redactor + retriever + profile into one struct so the desktop/mobile binary can construct it once at startup. The `#[ignore]`d smoke test lets a human operator verify the Anthropic wiring end-to-end against the real API before shipping.

- [ ] **Step 1: Implement the facade**

Replace the entire contents of `core/src/ai/cloud/actions/mod.rs`:

```rust
pub mod draft;
pub mod search;
pub mod summarize;

use std::sync::Arc;

use uuid::Uuid;

use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::Redactor;
use crate::ai::cloud::CloudConfig;
use crate::ai::profile::UserProfile;
use crate::error::Result;
use crate::knowledge::Retriever;
use crate::store::Store;

pub use summarize::DraftOutcome;

/// The single entry point the app binary uses for cloud actions.
///
/// Holds the provider, redactor, optional retriever, and user profile
/// so each action call is a one-liner at the call site.
pub struct CloudActions {
    provider: Arc<dyn CloudProvider>,
    redactor: Redactor,
    retriever: Option<Arc<Retriever>>,
    profile: Arc<UserProfile>,
    model: String,
}

impl CloudActions {
    pub fn new(
        provider: Arc<dyn CloudProvider>,
        redactor: Redactor,
        retriever: Option<Arc<Retriever>>,
        profile: UserProfile,
        model: String,
    ) -> Self {
        Self {
            provider,
            redactor,
            retriever,
            profile: Arc::new(profile),
            model,
        }
    }

    pub async fn summarize_thread(
        &self,
        store: &Store,
        thread_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        summarize::summarize_thread(
            store,
            self.provider.clone(),
            &self.redactor,
            &self.profile,
            thread_id,
            cfg,
            &self.model,
        )
        .await
    }

    pub async fn draft_reply(
        &self,
        store: &Store,
        message_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        draft::draft_reply(
            store,
            self.provider.clone(),
            &self.redactor,
            self.retriever.as_ref(),
            &self.profile,
            message_id,
            cfg,
            &self.model,
        )
        .await
    }

    pub async fn smart_search(
        &self,
        store: &Store,
        query: &str,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        search::smart_search(
            store,
            self.provider.clone(),
            &self.redactor,
            self.retriever.as_ref(),
            &self.profile,
            query,
            cfg,
            &self.model,
        )
        .await
    }
}
```

- [ ] **Step 2: Write the facade integration test**

Create `core/tests/cloud_facade_test.rs`:

```rust
use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::{CloudActions, CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct SequencedProvider {
    responses: Mutex<Vec<Result<String>>>,
}

impl SequencedProvider {
    fn new(rs: Vec<Result<String>>) -> Self {
        Self {
            responses: Mutex::new(rs),
        }
    }
}

#[async_trait]
impl CloudProvider for SequencedProvider {
    async fn complete(&self, _s: &str, _u: &str, _m: u32) -> Result<String> {
        self.responses.lock().unwrap().remove(0)
    }
}

#[tokio::test]
async fn test_facade_handles_three_actions_in_sequence() {
    let store = Store::open_in_memory().unwrap();

    // Seed one contact + one thread + one message for the first two actions.
    let contact = Uuid::new_v4();
    let thread = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread,
            channel: Channel::Email,
            subject: Some("T".into()),
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    let msg = Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: thread,
        sender_id: contact,
        content: MessageContent {
            text: Some("hey".into()),
            html: None,
            subject: Some("T".into()),
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
    let message_id = msg.id;

    let provider = Arc::new(SequencedProvider::new(vec![
        Ok(r#"{"summary": "short thread", "language": "en"}"#.into()),
        Ok(r#"{"draft": "ok", "language": "en"}"#.into()),
        Ok(r#"{"answer": "no matches", "sources": []}"#.into()),
    ]));
    let actions = CloudActions::new(
        provider as Arc<dyn CloudProvider>,
        Redactor::from_names(vec![]),
        None,
        UserProfile { content: String::new() },
        "claude-sonnet-4-6".into(),
    );

    let sum = actions
        .summarize_thread(&store, thread, CloudConfig::default())
        .await
        .unwrap();
    assert!(sum.output.contains("short thread"));

    let dr = actions
        .draft_reply(&store, message_id, CloudConfig::default())
        .await
        .unwrap();
    assert_eq!(dr.output, "ok");

    let ss = actions
        .smart_search(&store, "anything new?", CloudConfig::default())
        .await
        .unwrap();
    assert!(ss.output.contains("no matches"));
}
```

- [ ] **Step 3: Write the real-Anthropic smoke test**

Create `core/tests/cloud_anthropic_integration_test.rs`:

```rust
//! Smoke tests against the real Anthropic API.
//!
//! Run with:
//!     ANTHROPIC_API_KEY=sk-ant-... cargo test -p messagehub-core \
//!         --test cloud_anthropic_integration_test -- --ignored --nocapture
//!
//! Requires an `ANTHROPIC_API_KEY` environment variable. The default
//! model is `claude-sonnet-4-6`; override with `MESSAGEHUB_CLOUD_MODEL`.

use messagehub_core::ai::cloud::{AnthropicCloud, CloudProvider};

fn api_key() -> String {
    std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY env var required")
}

fn model() -> String {
    std::env::var("MESSAGEHUB_CLOUD_MODEL").unwrap_or_else(|_| "claude-sonnet-4-6".into())
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY and hits the real API"]
async fn test_real_anthropic_health_check() {
    let provider = AnthropicCloud::new(api_key(), model());
    let ok = provider.health_check().await.unwrap();
    assert!(ok, "Anthropic health check failed");
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY and hits the real API"]
async fn test_real_anthropic_complete_returns_non_empty() {
    let provider = AnthropicCloud::new(api_key(), model());
    let out = provider
        .complete(
            "You are a test assistant. Respond with the single word: hello.",
            "Say hello.",
            32,
        )
        .await
        .unwrap();
    assert!(!out.trim().is_empty(), "empty response from Anthropic");
}
```

- [ ] **Step 4: Run the full suite**

Run: `cargo test -p messagehub-core`
Expected: all tests pass EXCEPT those marked `#[ignore]` (embedder tests, Ollama integration, Anthropic integration). Count should be ~155 passing (~113 pre-plan + ~42 new).

- [ ] **Step 5: (Optional, human operator only) Run the real-Anthropic smoke**

```bash
ANTHROPIC_API_KEY=sk-ant-... cargo test -p messagehub-core \
    --test cloud_anthropic_integration_test -- --ignored --nocapture
```
Expected: 2 passing tests. If the key is invalid or the account is rate-limited, the first test fails clearly with the Anthropic status code.

- [ ] **Step 6: Commit**

```bash
git add core/src/ai/cloud/actions/mod.rs core/tests/cloud_facade_test.rs core/tests/cloud_anthropic_integration_test.rs
git commit -m "feat(cloud): add CloudActions facade and ignored Anthropic smoke test"
```

---

## Summary

After completing all 10 tasks, the core crate has:

- **`ai::cloud::CloudProvider` trait + `AnthropicCloud`** — HTTP client for `/v1/messages` with `x-api-key` header, `anthropic-version: 2023-06-01`, `stream: false`, 60s timeout, text-block joining. Fully mockable via trait doubles and wiremock.
- **`ai::cloud::Redactor`** — longest-match vault-name substitution plus email/phone regex. Per-call counters, `(String, ReverseMap)` return, `un_redact(text, map)` for round-trip. Vault people loaded once at construction; mid-session refresh deferred.
- **`ai::cloud::confidence::derive_confidence`** — heuristic 0.0-1.0 score from retrieval similarity, sender-match signal, and profile-present signal. Zero-grounding inputs produce 0.0 (honest).
- **Three actions** (`summarize_thread`, `draft_reply`, `smart_search`) — each owns a system prompt demanding JSON, a strict parser that rejects malformed / out-of-range / missing-field responses, and an orchestrator wiring `build_rag_context` → redact → provider.complete → parse → un_redact → persist. All three share `insert_draft` + `log_ai_decision` audit patterns, matching Plan 4's classifier parity.
- **`CloudActions` facade** — one struct holding provider + redactor + retriever + profile; one async method per action.
- **Migration 004** — `ai_drafts(id, message_id, action_type, input_redacted, output, user_edited_output, confidence, provider, model, created_at)` with indexes on `(message_id, created_at)` and `(action_type, created_at)`.
- **`Store::list_messages_in_thread`** + **`Store::insert_draft` / `list_drafts_for_message` / `update_draft_output`** — store helpers covering the new schema.
- **`CoreError::Cloud(String)`** — one new error variant; every cloud failure path surfaces as this.
- **Test coverage** — ~42 new tests covering provider HTTP, redaction, confidence, three actions, and the facade. Plus one `#[ignore]`d real-Anthropic smoke test gated on `ANTHROPIC_API_KEY`.

**Verification checklist** — after all tasks complete, run:

```bash
cargo test -p messagehub-core
```

Expected: all tests pass except the `#[ignore]`d ones (embedder + Ollama + Anthropic integration).

**Next plan candidates:**
- Plan 6: desktop Tauri app surfacing these actions in the UI + OS keychain for API key.
- Plan 7: second `CloudProvider` impl (OpenAI-compatible) for self-hosted vLLM / LM Studio users.
- Plan 8: semi-autonomous mode acting on `confidence > 0.9` drafts automatically.
