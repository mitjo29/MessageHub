# Plan 7b.3: AI-Assisted Email Reply — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve the read-only three-pane inbox from 7b.2 into a read-and-reply inbox for Email. Reply button in `MessageDetail` opens a modal composer with autosave-backed drafts (new `reply_drafts` table), proper RFC 5322 threading on send (`In-Reply-To`, `References`, fresh `Message-ID`), and a rich AI assist panel that wraps the existing `ai/cloud/draft_reply` action (generate / regenerate / confidence / redaction toggle / prior-drafts dropdown with Restore).

**Architecture:** Core adds one migration (`007_reply_drafts.sql`), one new store module (`store/reply_drafts.rs`), a `ReplyHeaders` option on `MessageContent`, and threading-aware branches in `EmailAdapter::send_reply`. Tauri gains 7 commands plus new fields on `AppState` (a `CloudActions` handle + an `email_connections` map sourced from `messagehub.toml`). React grows a `ReplyModal` component (+ `AiAssistPanel` + `PriorDraftsDropdown`), a `useAutosave` hook, and one reducer action (`OPEN_REPLY`). Send is synchronous — failures surface as an inline banner; autosave is React-driven with a 5 s idle debounce.

**Tech Stack:** Rust 1.75+, rusqlite / SQLCipher, lettre (SMTP), Tauri 2.x, React 18, TypeScript 5. No new npm or cargo deps.

**Prerequisites:**
- Master at `7a52b4a` or later (7b.3 spec committed on top of 7b.2.1 merge).
- `cargo build --workspace` clean on a fresh checkout.
- Existing manual UAT of 7b.2 passes — the three-pane layout renders, `mark_read` works, polling advances without duplicates.
- A reachable SMTP endpoint for end-to-end UAT: either a real email account (app-password in `[[channels]]`) or a local sink like `maildev` / `Papercut`.
- For AI UAT: an Anthropic API key. Without one, all AI-related UAT steps are skipped and the panel renders disabled.

**Spec:** `docs/superpowers/specs/2026-04-21-plan7b3-ai-assisted-email-reply-design.md`.

---

## File Structure

```
MessageHub/
├── core/
│   ├── migrations/
│   │   └── 007_reply_drafts.sql                 CREATE
│   ├── src/
│   │   ├── store/
│   │   │   ├── reply_drafts.rs                  CREATE
│   │   │   ├── mod.rs                           MODIFY: re-export + register module
│   │   │   └── migrations.rs                    MODIFY: register 007
│   │   ├── types/
│   │   │   └── message.rs                       MODIFY: ReplyHeaders + reply_headers field
│   │   └── adapters/
│   │       └── email.rs                         MODIFY: send_reply honors reply_headers
│   └── tests/
│       ├── store_reply_drafts_test.rs           CREATE
│       └── adapter_email_reply_test.rs          CREATE
└── desktop/
    ├── src-tauri/
    │   └── src/
    │       ├── commands.rs                      MODIFY: 7 new commands + DTOs
    │       ├── config.rs                        MODIFY: parse [cloud] + [[channels]]
    │       ├── state.rs                         MODIFY: cloud + email_connections
    │       └── main.rs                          MODIFY: wire new commands + fields
    └── src/
        ├── api.ts                               MODIFY: 7 new wrappers
        ├── types.ts                             MODIFY: new DTO types
        ├── App.tsx                              MODIFY: render ReplyModal when open
        ├── App.css                              MODIFY: reply-modal styles
        ├── components/
        │   ├── MessageDetail.tsx                MODIFY: Reply button
        │   ├── ReplyModal.tsx                   CREATE
        │   ├── AiAssistPanel.tsx                CREATE
        │   └── PriorDraftsDropdown.tsx          CREATE
        ├── hooks/
        │   └── useAutosave.ts                   CREATE
        └── state/
            └── InboxContext.tsx                 MODIFY: replyFor + OPEN_REPLY/CLOSE_REPLY
```

---

### Task 1: Preflight + feature branch

**Files:** (none — setup only)

`★ Why this matters:` Start from a known-good master. Confirm the 7b.3 spec is committed and the working tree is clean before touching anything.

- [ ] **Step 1: Confirm repo state**

```bash
git status
git log --oneline | head -3
git branch --show-current
```

Expected: top commit is `8c677fa` or later (`docs(spec): 7b.3 …`). Current branch is `master`. Working tree clean except for ignored/untracked noise (`.remember/`, `README.md`, `*.db-shm`).

- [ ] **Step 2: Create the feature branch**

```bash
git checkout -b feat/reply-composer
```

- [ ] **Step 3: Baseline build + tests**

```bash
cargo build --workspace
cargo test -p messagehub-core --lib
```

Expected: clean build; all core unit tests pass. If not, stop and fix master first.

---

### Task 2: Migration 007 — `reply_drafts` table

**Files:**
- Create: `core/migrations/007_reply_drafts.sql`
- Modify: `core/src/store/migrations.rs`

`★ Why this matters:` The schema is the anchor for autosave. One row per thread, UPSERT on save, DELETE on send.

- [ ] **Step 1: Write the migration SQL**

Create `core/migrations/007_reply_drafts.sql`:

```sql
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
```

- [ ] **Step 2: Register the migration**

Open `core/src/store/migrations.rs` and find the migrations array. Append:

```rust
("007_reply_drafts", include_str!("../../migrations/007_reply_drafts.sql")),
```

- [ ] **Step 3: Build sanity**

```bash
cargo build -p messagehub-core
```

Expected: clean build. `include_str!` fails at compile time if the path is wrong.

- [ ] **Step 4: Commit**

```bash
git add core/migrations/007_reply_drafts.sql core/src/store/migrations.rs
git commit -m "feat(core): migration 007 — reply_drafts table"
```

---

### Task 3: `ReplyDraft` type + `store/reply_drafts.rs` — failing tests first

**Files:**
- Create: `core/tests/store_reply_drafts_test.rs`
- (Not yet: `core/src/store/reply_drafts.rs` — Step 4 creates it.)

`★ Why this matters:` TDD. Lock the semantics (upsert overwrites, delete is idempotent, get returns None for unknown thread) as tests before writing the impl.

- [ ] **Step 1: Write the failing integration test**

Create `core/tests/store_reply_drafts_test.rs`:

```rust
use chrono::Utc;
use messagehub_core::store::{NewReplyDraft, ReplyDraft, Store};
use uuid::Uuid;

fn fresh_store() -> Store {
    Store::open_in_memory().expect("open_in_memory")
}

#[test]
fn upsert_then_get_roundtrip() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    let msg = Uuid::new_v4();

    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg,
            body: "hello world",
            subject: Some("Re: ping"),
        })
        .expect("upsert ok");

    let got: ReplyDraft = store
        .get_reply_draft(&thread)
        .expect("get ok")
        .expect("row exists");
    assert_eq!(got.thread_id, thread);
    assert_eq!(got.in_reply_to_message_id, msg);
    assert_eq!(got.body, "hello world");
    assert_eq!(got.subject.as_deref(), Some("Re: ping"));
    // updated_at is set by the DB default.
    assert!(got.updated_at <= Utc::now());
}

#[test]
fn second_upsert_overwrites_body_and_reply_target() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    let msg1 = Uuid::new_v4();
    let msg2 = Uuid::new_v4();

    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg1,
            body: "v1",
            subject: None,
        })
        .unwrap();
    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg2,
            body: "v2",
            subject: Some("Re: foo"),
        })
        .unwrap();

    let got = store.get_reply_draft(&thread).unwrap().unwrap();
    assert_eq!(got.in_reply_to_message_id, msg2);
    assert_eq!(got.body, "v2");
    assert_eq!(got.subject.as_deref(), Some("Re: foo"));
}

#[test]
fn get_unknown_thread_returns_none() {
    let store = fresh_store();
    assert!(store.get_reply_draft(&Uuid::new_v4()).unwrap().is_none());
}

#[test]
fn delete_is_idempotent() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    // Delete when absent.
    store.delete_reply_draft(&thread).unwrap();
    // Insert then delete.
    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: Uuid::new_v4(),
            body: "hi",
            subject: None,
        })
        .unwrap();
    store.delete_reply_draft(&thread).unwrap();
    assert!(store.get_reply_draft(&thread).unwrap().is_none());
    // Second delete — still Ok.
    store.delete_reply_draft(&thread).unwrap();
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p messagehub-core --test store_reply_drafts_test
```

Expected: compile error — `NewReplyDraft`, `ReplyDraft`, `upsert_reply_draft`, `get_reply_draft`, `delete_reply_draft` do not exist.

---

### Task 4: `store/reply_drafts.rs` — minimal implementation

**Files:**
- Create: `core/src/store/reply_drafts.rs`
- Modify: `core/src/store/mod.rs`

- [ ] **Step 1: Write the module**

Create `core/src/store/reply_drafts.rs`:

```rust
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct ReplyDraft {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: String,
    pub subject: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Borrowed payload for `upsert_reply_draft` — avoids string allocations at
/// the call site on every autosave tick.
#[derive(Debug, Clone)]
pub struct NewReplyDraft<'a> {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: &'a str,
    pub subject: Option<&'a str>,
}

impl Store {
    pub fn upsert_reply_draft(&self, draft: &NewReplyDraft<'_>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO reply_drafts
                (thread_id, in_reply_to_message_id, body, subject, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT(thread_id) DO UPDATE SET
                 in_reply_to_message_id = excluded.in_reply_to_message_id,
                 body = excluded.body,
                 subject = excluded.subject,
                 updated_at = excluded.updated_at",
            params![
                draft.thread_id.to_string(),
                draft.in_reply_to_message_id.to_string(),
                draft.body,
                draft.subject,
            ],
        )?;
        Ok(())
    }

    pub fn get_reply_draft(&self, thread_id: &Uuid) -> Result<Option<ReplyDraft>> {
        let row = self
            .conn()
            .query_row(
                "SELECT thread_id, in_reply_to_message_id, body, subject, updated_at
                 FROM reply_drafts
                 WHERE thread_id = ?1",
                params![thread_id.to_string()],
                |row| {
                    let thread: String = row.get(0)?;
                    let irt: String = row.get(1)?;
                    let body: String = row.get(2)?;
                    let subject: Option<String> = row.get(3)?;
                    let updated_at: String = row.get(4)?;
                    Ok((thread, irt, body, subject, updated_at))
                },
            )
            .optional()?;

        match row {
            None => Ok(None),
            Some((thread, irt, body, subject, updated_at)) => Ok(Some(ReplyDraft {
                thread_id: Uuid::parse_str(&thread)
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                in_reply_to_message_id: Uuid::parse_str(&irt)
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                body,
                subject,
                updated_at: parse_sqlite_ts(&updated_at)?,
            })),
        }
    }

    /// Idempotent — deleting a missing row is Ok.
    pub fn delete_reply_draft(&self, thread_id: &Uuid) -> Result<()> {
        self.conn().execute(
            "DELETE FROM reply_drafts WHERE thread_id = ?1",
            params![thread_id.to_string()],
        )?;
        Ok(())
    }
}

/// Parse the `%Y-%m-%dT%H:%M:%SZ` format emitted by the SQLite `strftime`
/// default. Returns `CoreError::InvalidInput` on malformed values (should
/// never happen — writes go through the same format).
fn parse_sqlite_ts(s: &str) -> Result<DateTime<Utc>> {
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
        .map_err(|e| CoreError::InvalidInput(format!("bad updated_at '{}': {}", s, e)))?;
    Ok(Utc.from_utc_datetime(&naive))
}
```

- [ ] **Step 2: Register the module + re-exports**

Edit `core/src/store/mod.rs`. In the module declarations block, add:

```rust
pub mod reply_drafts;
```

In the `pub use` block, add:

```rust
pub use reply_drafts::{NewReplyDraft, ReplyDraft};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p messagehub-core --test store_reply_drafts_test
```

Expected: all four tests pass.

- [ ] **Step 4: Commit**

```bash
git add core/migrations/007_reply_drafts.sql \
        core/src/store/reply_drafts.rs \
        core/src/store/mod.rs \
        core/tests/store_reply_drafts_test.rs
git commit -m "feat(core): reply_drafts upsert/get/delete with tests"
```

(Note: the migration was staged in Task 2 but not yet committed if you're reordering — adjust accordingly. The above assumes Task 2 already committed the migration; if your local state differs, stage only the new files here.)

---

### Task 5: `ReplyHeaders` type on `MessageContent`

**Files:**
- Modify: `core/src/types/message.rs`

`★ Why this matters:` The only way to get threading info from the Tauri send command down to the SMTP builder without adding a new trait method. Additive only — everything currently constructing `MessageContent` keeps working because the field defaults to `None`.

- [ ] **Step 1: Find the current `MessageContent` definition**

Open `core/src/types/message.rs`. Locate the `MessageContent` struct and confirm its current shape (for accurate merging). Check the `impl Default` block if one exists.

- [ ] **Step 2: Add `ReplyHeaders`**

At the bottom of the file's type declarations (before any `impl` blocks or tests), add:

```rust
/// Threading metadata for an outbound reply. When present, the email adapter
/// renders `In-Reply-To` / `References` / a fresh `Message-ID` so the reply
/// threads in the recipient's client. Inbound messages always have this as
/// `None`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ReplyHeaders {
    /// RFC 5322 destination address, typically the original sender.
    pub to: String,
    /// The original message's Message-ID (bare — no `<...>` wrapping needed).
    pub in_reply_to: String,
    /// Existing References chain + the original's Message-ID appended. Bare
    /// ids, one per Vec entry; the adapter wraps each in `<...>`.
    pub references: Vec<String>,
}
```

- [ ] **Step 3: Add the field to `MessageContent`**

Find `pub struct MessageContent` and add one field at the end, before the closing brace:

```rust
    /// Set on outbound messages the Tauri layer produces for replies;
    /// always `None` for messages ingested from adapters.
    #[serde(default)]
    pub reply_headers: Option<ReplyHeaders>,
```

The `#[serde(default)]` lets old serialized messages without this field still deserialize cleanly.

- [ ] **Step 4: Update `Default` if one is derived manually**

If `MessageContent` uses `#[derive(Default)]`, no change needed. If it has `impl Default for MessageContent`, add `reply_headers: None` inside that block.

- [ ] **Step 5: Build sanity**

```bash
cargo build -p messagehub-core
```

Expected: clean build. Any downstream code that constructs `MessageContent` with explicit fields (not `..Default::default()`) will fail compilation here — fix those sites by appending `reply_headers: None`.

- [ ] **Step 6: Run the full test suite to catch downstream fallout**

```bash
cargo test -p messagehub-core
```

Expected: all tests pass. If any adapter tests fail because they construct `MessageContent` positionally, add `reply_headers: None` to those literals.

- [ ] **Step 7: Commit**

```bash
git add core/src/types/message.rs
# plus any downstream test files you touched
git commit -m "feat(core): ReplyHeaders + MessageContent.reply_headers"
```

---

### Task 6: Email adapter — honor `reply_headers` — failing test

**Files:**
- Create: `core/tests/adapter_email_reply_test.rs`

`★ Why this matters:` Proves the rendered bytes carry correct threading headers without needing a live SMTP server. Uses lettre's message builder in the same way the adapter does.

- [ ] **Step 1: Write the failing test**

Create `core/tests/adapter_email_reply_test.rs`:

```rust
//! Verify EmailAdapter::send_reply renders a threaded RFC 5322 message when
//! MessageContent.reply_headers is Some. We drive the adapter all the way to
//! building a lettre::Message and inspect its serialized bytes — no SMTP
//! transport is opened.
//!
//! The test relies on a #[cfg(test)] helper `build_reply_message` we add to
//! email.rs in the next task. Until that helper exists this file fails to
//! compile.

use messagehub_core::adapters::email::build_reply_message;
use messagehub_core::types::{MessageContent, ReplyHeaders};

fn sample_content(subject: &str, body: &str, headers: ReplyHeaders) -> MessageContent {
    MessageContent {
        text: Some(body.to_string()),
        html: None,
        subject: Some(subject.to_string()),
        attachments: Vec::new(),
        reply_headers: Some(headers),
    }
}

#[test]
fn renders_in_reply_to_and_references() {
    let content = sample_content(
        "Re: quote",
        "Thanks, sending the updated quote.\n",
        ReplyHeaders {
            to: "bob@example.com".into(),
            in_reply_to: "abc@orig".into(),
            references: vec!["root@orig".into(), "abc@orig".into()],
        },
    );

    let msg = build_reply_message(
        "alice@example.com",
        &content,
        "smtp.example.com",
    )
    .expect("build ok");

    let raw = std::str::from_utf8(&msg.formatted()).expect("utf-8");

    assert!(raw.contains("From: alice@example.com"));
    assert!(raw.contains("To: bob@example.com"));
    assert!(raw.contains("Subject: Re: quote"));
    assert!(raw.contains("In-Reply-To: <abc@orig>"));
    assert!(raw.contains("References: <root@orig> <abc@orig>"));
    // Lettre auto-generates a Message-ID; we only care it's present and
    // scoped to the SMTP host.
    assert!(raw.contains("Message-ID: <"));
    assert!(raw.contains("@smtp.example.com>"));
}

#[test]
fn dedupes_re_prefix() {
    let content = sample_content(
        "Re: already tagged",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "m@x".into(),
            references: vec!["m@x".into()],
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let raw = std::str::from_utf8(&msg.formatted()).expect("utf-8");
    // Only one "Re: ".
    let re_count = raw.matches("Subject: Re: ").count();
    assert_eq!(re_count, 1, "Subject should have exactly one Re: prefix");
    assert!(!raw.contains("Subject: Re: Re: already tagged"));
}

#[test]
fn prepends_re_when_missing() {
    let content = sample_content(
        "plain subject",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "m@x".into(),
            references: vec!["m@x".into()],
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let raw = std::str::from_utf8(&msg.formatted()).expect("utf-8");
    assert!(raw.contains("Subject: Re: plain subject"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p messagehub-core --test adapter_email_reply_test
```

Expected: compile error — `build_reply_message` not found, field `reply_headers` on `MessageContent` ok (Task 5 added it).

---

### Task 7: Email adapter — `build_reply_message` helper + threaded `send_reply`

**Files:**
- Modify: `core/src/adapters/email.rs`

`★ Why this matters:` Pulling the message-building logic into a pure function lets the test drive it without SMTP. The existing `send_reply` becomes a thin shell that builds + sends.

- [ ] **Step 1: Add the `build_reply_message` helper**

Open `core/src/adapters/email.rs`. Add these imports near the top (merge with existing `use lettre::…` line if one exists):

```rust
use lettre::message::{header, MessageBuilder};
```

At module scope (public, near the bottom of the file but before `#[cfg(test)] mod tests`), add:

```rust
/// Build a lettre::Message for a threaded reply. Extracted for testability —
/// the production path (`send_reply`) calls this then hands the result to
/// an SMTP transport. The test suite calls it and inspects `.formatted()`
/// without opening a socket.
///
/// Preconditions:
/// - `content.reply_headers` is `Some(...)`.
/// - `content.text` is `Some(...)`.
///
/// Returns `CoreError::InvalidInput` for any address / header parse failure.
pub fn build_reply_message(
    from_username: &str,
    content: &crate::types::MessageContent,
    smtp_host: &str,
) -> crate::error::Result<lettre::Message> {
    use crate::error::CoreError;

    let headers = content.reply_headers.as_ref().ok_or_else(|| {
        CoreError::InvalidInput("build_reply_message called without reply_headers".into())
    })?;
    let text = content.text.as_deref().ok_or_else(|| {
        CoreError::InvalidInput("reply body text is required".into())
    })?;

    let subject_raw = content.subject.as_deref().unwrap_or("");
    let subject = if subject_raw.trim_start().to_ascii_lowercase().starts_with("re:") {
        subject_raw.to_string()
    } else if subject_raw.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {}", subject_raw)
    };

    let from_addr: lettre::message::Mailbox = from_username
        .parse()
        .map_err(|e: lettre::address::AddressError| {
            CoreError::InvalidInput(format!("invalid from address: {}", e))
        })?;
    let to_addr: lettre::message::Mailbox =
        headers.to.parse().map_err(|e: lettre::address::AddressError| {
            CoreError::InvalidInput(format!("invalid to address: {}", e))
        })?;

    // Build In-Reply-To and References with proper <...> wrapping. Lettre's
    // typed headers don't cover these two, so we attach them as raw header
    // lines.
    let in_reply_to = wrap_angle(&headers.in_reply_to);
    let references = headers
        .references
        .iter()
        .map(|r| wrap_angle(r))
        .collect::<Vec<_>>()
        .join(" ");

    // Lettre generates a unique Message-ID using the provided hostname.
    let mut builder = MessageBuilder::new()
        .from(from_addr)
        .to(to_addr)
        .subject(subject)
        .message_id(Some(format!("{}@{}", uuid::Uuid::new_v4(), smtp_host)));

    if !in_reply_to.is_empty() {
        builder = builder.header(
            header::HeaderName::new_from_ascii_str("In-Reply-To"),
        );
        // HeaderName-only API doesn't exist on older lettre versions; fall
        // back to the raw-header variant if the above doesn't compile.
        // (See Step 2 note.)
    }

    builder
        .body(text.to_string())
        .map_err(|e| CoreError::Channel(format!("failed to build email: {}", e)))
}

fn wrap_angle(s: &str) -> String {
    let trimmed = s.trim().trim_start_matches('<').trim_end_matches('>');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("<{}>", trimmed)
    }
}
```

- [ ] **Step 2: Resolve the lettre header API**

Lettre 0.11 exposes raw header attachment through `MessageBuilder::header` taking a typed header, not a name-only call. Replace the In-Reply-To / References block above with the following, which works across 0.11.x patch versions:

```rust
use lettre::message::header::{HeaderName, HeaderValue};

let mut builder = MessageBuilder::new()
    .from(from_addr)
    .to(to_addr)
    .subject(subject)
    .message_id(Some(format!("{}@{}", uuid::Uuid::new_v4(), smtp_host)));

let irt_header_name = HeaderName::new_from_ascii("In-Reply-To".to_string())
    .map_err(|e| CoreError::Channel(format!("bad header name: {}", e)))?;
let irt_header_value = HeaderValue::new(irt_header_name.clone(), in_reply_to.clone());
builder = builder.header(irt_header_value);

if !references.is_empty() {
    let refs_name = HeaderName::new_from_ascii("References".to_string())
        .map_err(|e| CoreError::Channel(format!("bad header name: {}", e)))?;
    let refs_value = HeaderValue::new(refs_name, references.clone());
    builder = builder.header(refs_value);
}

builder
    .body(text.to_string())
    .map_err(|e| CoreError::Channel(format!("failed to build email: {}", e)))
```

If a lettre API mismatch forces a different shape, consult `cargo doc --package lettre --open` → `lettre::message::header` for the exact types available in your pinned version. The test asserts the *serialized bytes* contain the header lines, so any API that produces those bytes is acceptable.

- [ ] **Step 3: Update `send_reply` to branch on `reply_headers`**

Find the existing `async fn send_reply` in `impl ChannelAdapter for EmailAdapter`. Replace its body so it dispatches to `build_reply_message` when headers are present:

```rust
async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
    if !self.connected {
        return Err(CoreError::Connection("not connected".to_string()));
    }
    let smtp_host = self.smtp_host.as_ref().ok_or_else(|| {
        CoreError::Connection("SMTP host not set".to_string())
    })?;
    let username = self.username.as_ref().ok_or_else(|| {
        CoreError::Connection("username not set".to_string())
    })?;
    let password = self.password.as_ref().ok_or_else(|| {
        CoreError::Connection("password not set".to_string())
    })?;

    let email = if content.reply_headers.is_some() {
        build_reply_message(username, content, smtp_host)?
    } else {
        // Legacy path — thread_id used as the destination address. No
        // callers today; kept for forward compat.
        let text = content.text.as_deref().ok_or_else(|| {
            CoreError::InvalidInput("email body text is required".to_string())
        })?;
        let subject = content.subject.as_deref().unwrap_or("Re:");
        lettre::Message::builder()
            .from(username.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid from address: {}", e))
            })?)
            .to(thread_id.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid to address: {}", e))
            })?)
            .subject(subject)
            .body(text.to_string())
            .map_err(|e| CoreError::Channel(format!("failed to build email: {}", e)))?
    };

    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

    let creds = Credentials::new(username.clone(), password.clone());
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
        .map_err(|e| CoreError::Channel(format!("smtp transport: {}", e)))?
        .port(self.smtp_port)
        .credentials(creds)
        .build();

    mailer
        .send(email)
        .await
        .map_err(|e| CoreError::Channel(format!("smtp send: {}", e)))?;
    info!("sent email reply via smtp");
    Ok(())
}
```

- [ ] **Step 4: Run the adapter reply tests**

```bash
cargo test -p messagehub-core --test adapter_email_reply_test
```

Expected: all three tests pass. If the lettre header API doesn't match Step 2, adjust and retry.

- [ ] **Step 5: Run the full suite to confirm nothing else regressed**

```bash
cargo test -p messagehub-core
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add core/src/adapters/email.rs core/tests/adapter_email_reply_test.rs
git commit -m "feat(core): email send_reply honors ReplyHeaders for RFC 5322 threading"
```

---

### Task 8: Tauri `config.rs` — parse `[cloud]` and `[[channels]]`

**Files:**
- Modify: `desktop/src-tauri/src/config.rs`

`★ Why this matters:` The Tauri host today only reads `database` + `password`. To send email and run cloud actions, it needs the same channel credentials and cloud config that `runtime-demo` reads.

- [ ] **Step 1: Read current config.rs**

Open `desktop/src-tauri/src/config.rs`. Confirm the existing `Config` struct and `load_config` function. The edits below assume a simple `Config { database, password }` layout — merge as appropriate.

- [ ] **Step 2: Extend the Config types**

Replace the existing `Config` struct (and add the new support types) with:

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub database: String,
    pub password: String,
    #[serde(default)]
    pub cloud: Option<TauriCloudConfig>,
    #[serde(default)]
    pub channels: Vec<ChannelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TauriCloudConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelEntry {
    pub kind: String,
    pub label: String,
    pub enabled: bool,
    pub credentials: toml::Value,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EmailCredentials {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
}
```

Keep `load_config` and `resolve_config_path` as they are — `serde(default)` makes the new fields optional so legacy `messagehub.toml` files still load.

- [ ] **Step 3: Build sanity**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build. If there are unused-imports warnings for the new types, ignore — they'll be consumed in Task 9.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/config.rs
git commit -m "feat(desktop): parse [cloud] + [[channels]] from messagehub.toml"
```

---

### Task 9: `AppState` grows `cloud` + `email_connections`

**Files:**
- Modify: `desktop/src-tauri/src/state.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` Two pieces of init state needed by the new commands: an `Option<Arc<CloudActions>>` for the AI path, and a `HashMap<Uuid, EmailConnection>` for the send path. Both are populated once at startup from `messagehub.toml`.

- [ ] **Step 1: Read state.rs**

Confirm the existing `AppState` struct layout so you know what to preserve.

- [ ] **Step 2: Add the new fields + a constructor update**

Edit `desktop/src-tauri/src/state.rs`. Add near the top (or merge with existing imports):

```rust
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use messagehub_core::ai::cloud::{AnthropicCloud, CloudActions, Redactor};
use messagehub_core::ai::profile::UserProfile;
```

Add the struct:

```rust
#[derive(Debug, Clone)]
pub struct EmailConnection {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
}
```

Add the new fields to `AppState`. Wherever `AppState` is currently declared, add:

```rust
    pub cloud: Option<Arc<CloudActions>>,
    pub email_connections: HashMap<Uuid, EmailConnection>,
```

- [ ] **Step 3: Helper to build stable channel ids**

Add inside `state.rs` (private, used by init):

```rust
pub fn stable_channel_id(kind: &str, label: &str) -> Uuid {
    // Matches runtime-demo's UUIDv5(OID, "{kind}:{label}") mapping so Reply
    // lines up with the runtime's channels rows.
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:{}", kind, label).as_bytes(),
    )
}
```

- [ ] **Step 4: Wire init in main.rs**

Open `desktop/src-tauri/src/main.rs`. In `try_init`, after the existing code that loads the config and opens the store, before constructing `AppState`, add:

```rust
// Build email connections map from [[channels]]. Telegram entries are
// parsed elsewhere (no send path for them in 7b.3).
let mut email_connections = std::collections::HashMap::new();
for entry in &cfg.channels {
    if entry.kind == "email" {
        let creds: crate::config::EmailCredentials = entry
            .credentials
            .clone()
            .try_into()
            .map_err(|e| format!(
                "channel '{}': {}", entry.label, e,
            ))?;
        let id = crate::state::stable_channel_id(&entry.kind, &entry.label);
        email_connections.insert(id, crate::state::EmailConnection {
            imap_host: creds.imap_host,
            imap_port: creds.imap_port,
            smtp_host: creds.smtp_host,
            smtp_port: creds.smtp_port,
            username: creds.username,
            password: creds.password,
        });
    }
}

// Optional cloud actions handle + the model string we stash for
// cloud_config_status.
//
// `Redactor::build(&store)` requires a live store, so we construct the
// cloud handle *inside* AppState::init after the store is open. See
// Step 5.
```

Update the `AppState::init` signature + body to accept the parsed `Config`
(or `cfg.cloud` + `cfg.channels`) and build the cloud handle inside. The
snippet above belongs in `main.rs` only for the `email_connections` map;
move the cloud construction into `state.rs` per Step 5.

- [ ] **Step 5: Fix `AppState::init` signature and build the cloud handle**

Locate `AppState::init` in `state.rs`. Extend the parameters so the caller
can pass the two new bits: the `email_connections` map (already built in
`main.rs` Step 4) and the full `Config` (so the cloud handle can be
constructed with a live store). Also add a `cloud_model: Option<String>`
field alongside `cloud` — `cloud_config_status` needs to read it.

Shape:

```rust
use std::sync::Arc;
use messagehub_core::ai::cloud::{AnthropicCloud, CloudActions, Redactor};
use messagehub_core::ai::profile::UserProfile;

pub struct AppState {
    pub store: std::sync::Mutex<messagehub_core::store::Store>,
    pub db_path: String,
    pub channel_labels_by_variant: /* existing type */,
    pub email_connections: HashMap<Uuid, EmailConnection>,
    pub cloud: Option<Arc<CloudActions>>,
    pub cloud_model: Option<String>,
}

impl AppState {
    pub fn init(
        db_path: &str,
        password: &str,
        email_connections: HashMap<Uuid, EmailConnection>,
        cloud_cfg: Option<&crate::config::TauriCloudConfig>,
    ) -> Result<Self, String> {
        let store = messagehub_core::store::Store::open(
            std::path::Path::new(db_path), password,
        ).map_err(|e| format!("open store: {}", e))?;

        // Build cloud while the store is in scope so Redactor::build has
        // access to the vault (if any).
        let (cloud, cloud_model) = match cloud_cfg.filter(|c| c.enabled) {
            Some(c) => match (c.api_key.as_ref(), c.model.as_ref()) {
                (Some(k), Some(m)) => {
                    let redactor = Redactor::build(&store)
                        .map_err(|e| format!("Redactor::build: {}", e))?;
                    let provider: Arc<dyn messagehub_core::ai::cloud::CloudProvider> =
                        Arc::new(AnthropicCloud::new(k.clone(), m.clone()));
                    let actions = CloudActions::new(
                        provider,
                        redactor,
                        None, // no vault retriever in the desktop shell
                        UserProfile { content: String::new() },
                        m.clone(),
                    );
                    (Some(Arc::new(actions)), Some(m.clone()))
                }
                _ => {
                    eprintln!(
                        "messagehub-desktop: [cloud] enabled but api_key / model missing — running without cloud",
                    );
                    (None, None)
                }
            },
            None => (None, None),
        };

        // Build channel_labels_by_variant the same way init does today.
        let channel_labels_by_variant = /* preserve existing logic */;

        Ok(Self {
            store: std::sync::Mutex::new(store),
            db_path: db_path.to_string(),
            channel_labels_by_variant,
            email_connections,
            cloud,
            cloud_model,
        })
    }
}
```

Update the `AppState::init(...)` call in `main.rs` accordingly — pass
`email_connections` and `cfg.cloud.as_ref()`.

Delete the now-unused `cloud` construction block left in `main.rs` from
Step 4's previous version; that logic lives only inside `init` now.

- [ ] **Step 6: Build**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build. If `CloudActions::new`'s arity differs from what's above, adjust — check with:

```bash
grep -n "pub fn new" core/src/ai/cloud/actions/mod.rs
```

and pass exactly the fields it expects in the version on master.

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/state.rs desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): AppState gains cloud + email_connections from config"
```

---

### Task 10: Reply-draft Tauri commands (save / get / delete)

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` Three parallel thin wrappers around `Store::*_reply_draft`. Autosave fires `save_reply_draft` every 5 s; modal mount fires `get_reply_draft`; Discard fires `delete_reply_draft`.

- [ ] **Step 1: Add the DTO + three commands**

Open `desktop/src-tauri/src/commands.rs`. Add near the DTOs:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyDraftDto {
    pub thread_id: String,
    pub in_reply_to_message_id: String,
    pub body: String,
    pub subject: Option<String>,
    pub updated_at: String,
}

impl From<&messagehub_core::store::ReplyDraft> for ReplyDraftDto {
    fn from(d: &messagehub_core::store::ReplyDraft) -> Self {
        Self {
            thread_id: d.thread_id.to_string(),
            in_reply_to_message_id: d.in_reply_to_message_id.to_string(),
            body: d.body.clone(),
            subject: d.subject.clone(),
            updated_at: d.updated_at.to_rfc3339(),
        }
    }
}
```

At the bottom of `commands.rs` (or adjacent to the existing `#[tauri::command]` functions), add:

```rust
#[tauri::command]
pub fn save_reply_draft(
    thread_id: String,
    in_reply_to_message_id: String,
    body: String,
    subject: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let msg = Uuid::parse_str(&in_reply_to_message_id)
        .map_err(|e| format!("bad in_reply_to_message_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    store
        .upsert_reply_draft(&messagehub_core::store::NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg,
            body: &body,
            subject: subject.as_deref(),
        })
        .map_err(|e| format!("upsert_reply_draft: {}", e))
}

#[tauri::command]
pub fn get_reply_draft(
    thread_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ReplyDraftDto>, String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    let draft = store
        .get_reply_draft(&thread)
        .map_err(|e| format!("get_reply_draft: {}", e))?;
    Ok(draft.as_ref().map(ReplyDraftDto::from))
}

#[tauri::command]
pub fn delete_reply_draft(
    thread_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    store
        .delete_reply_draft(&thread)
        .map_err(|e| format!("delete_reply_draft: {}", e))
}
```

- [ ] **Step 2: Register the commands**

Open `desktop/src-tauri/src/main.rs`. In both `tauri::generate_handler![...]` blocks (there are two — the happy path and the fallback), append:

```rust
commands::save_reply_draft,
commands::get_reply_draft,
commands::delete_reply_draft,
```

- [ ] **Step 3: Add a DTO unit test**

In the `#[cfg(test)]` module at the bottom of `commands.rs`, add:

```rust
#[test]
fn reply_draft_dto_round_trips() {
    use messagehub_core::store::ReplyDraft;
    use chrono::TimeZone;

    let d = ReplyDraft {
        thread_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        in_reply_to_message_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        body: "hi".to_string(),
        subject: Some("Re: ping".to_string()),
        updated_at: chrono::Utc.with_ymd_and_hms(2026, 4, 21, 10, 0, 0).unwrap(),
    };
    let dto = ReplyDraftDto::from(&d);
    assert_eq!(dto.thread_id, "00000000-0000-0000-0000-000000000001");
    assert_eq!(dto.body, "hi");
    assert_eq!(dto.subject.as_deref(), Some("Re: ping"));
    assert!(dto.updated_at.starts_with("2026-04-21T10:00:00"));
}
```

- [ ] **Step 4: Build + test**

```bash
cargo build -p messagehub-desktop
cargo test -p messagehub-desktop
```

Expected: clean build, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): save_reply_draft / get_reply_draft / delete_reply_draft"
```

---

### Task 11: `send_email_reply` Tauri command

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` The big one. Builds `ReplyHeaders` from the original's stored metadata, spins up a one-shot `EmailAdapter`, sends, deletes the draft row on success.

- [ ] **Step 1: Add the command**

In `commands.rs`, add:

```rust
#[tauri::command]
pub async fn send_email_reply(
    thread_id: String,
    in_reply_to_message_id: String,
    body: String,
    subject: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use messagehub_core::adapters::email::{EmailAdapter, ImapSettings};
    use messagehub_core::adapters::ChannelAdapter;
    use messagehub_core::types::{Channel, MessageContent, ReplyHeaders};

    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let irt = Uuid::parse_str(&in_reply_to_message_id)
        .map_err(|e| format!("bad in_reply_to_message_id: {}", e))?;

    // Gather everything we need under the store lock, then drop it before
    // any await — same discipline as the runtime.
    let (channel_config, to_addr, in_reply_to_hdr, references_hdr) = {
        let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
        let message = store
            .get_message(&irt)
            .map_err(|e| format!("get_message: {}", e))?;

        if message.channel != Channel::Email {
            return Err("send_email_reply only supports Email channels".into());
        }

        let channel_cfg = store
            .list_channel_configs()
            .map_err(|e| format!("list_channel_configs: {}", e))?
            .into_iter()
            .find(|c| c.channel == message.channel)
            .ok_or_else(|| {
                "No Email channel configured for this message".to_string()
            })?;

        let contact = store
            .get_contact(&message.sender_id)
            .map_err(|e| format!("get_contact: {}", e))?;
        let to = contact
            .identities
            .iter()
            .find(|id| id.channel == Channel::Email)
            .map(|id| id.address.clone())
            .ok_or_else(|| {
                "No recipient address known for this contact on Email".to_string()
            })?;

        let original_msg_id = message
            .metadata
            .get("message_id")
            .cloned()
            .ok_or_else(|| {
                "Cannot reply: original message has no Message-ID header".to_string()
            })?;

        let references: Vec<String> = message
            .metadata
            .get("references")
            .map(|s| {
                s.split_whitespace()
                    .map(|r| r.trim_matches(|c| c == '<' || c == '>').to_string())
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut references = references;
        references.push(original_msg_id.clone());

        (channel_cfg, to, original_msg_id, references)
    };

    let conn = state
        .email_connections
        .get(&channel_config.id)
        .cloned()
        .ok_or_else(|| {
            "No credentials configured for this channel in messagehub.toml".to_string()
        })?;

    let subject_final = if subject.trim_start().to_ascii_lowercase().starts_with("re:") {
        subject
    } else if subject.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {}", subject)
    };

    let content = MessageContent {
        text: Some(body),
        html: None,
        subject: Some(subject_final),
        attachments: Vec::new(),
        reply_headers: Some(ReplyHeaders {
            to: to_addr,
            in_reply_to: in_reply_to_hdr,
            references: references_hdr,
        }),
    };

    // Build a config with the plain user:password keychain_ref shape the
    // adapter's connect() expects.
    let mut config_for_connect = channel_config.clone();
    config_for_connect.keychain_ref = format!("{}:{}", conn.username, conn.password);

    let mut adapter = EmailAdapter::with_settings(ImapSettings {
        host: conn.imap_host.clone(),
        port: conn.imap_port,
        smtp_host: conn.smtp_host.clone(),
        smtp_port: conn.smtp_port,
    });
    adapter
        .connect(&config_for_connect)
        .await
        .map_err(|e| format!("connect: {}", e))?;
    let send_result = adapter.send_reply("", &content).await;
    let _ = adapter.disconnect().await; // best-effort

    send_result.map_err(|e| format!("smtp send: {}", e))?;

    // Best-effort draft cleanup. Logged on failure but not surfaced — the
    // email already left.
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    if let Err(e) = store.delete_reply_draft(&thread) {
        eprintln!("send_email_reply: delete_reply_draft failed: {}", e);
    }
    Ok(())
}
```

- [ ] **Step 2: Register the command**

In both `tauri::generate_handler![...]` blocks in `main.rs`, append `commands::send_email_reply,`.

- [ ] **Step 3: Build**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build. If `Channel::Email`, `ChannelConfig.keychain_ref`, or any referenced field name drifts, grep for the canonical spelling and update.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): send_email_reply with threaded RFC 5322 headers"
```

---

### Task 12: AI commands — `ai_draft_reply`, `list_ai_drafts`, `cloud_config_status`

**Files:**
- Modify: `core/src/ai/cloud/actions/mod.rs`
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` `CloudActions::draft_reply(&self, store: &Store, ...)`
currently requires the caller to hold a `std::sync::MutexGuard<Store>` for
its entire duration — including the Anthropic HTTP call. That's `!Send`,
which means an async Tauri command holding such a guard across `.await` does
not compile. Step 1 adds a `draft_reply_via(Arc<Mutex<Store>>, ...)` wrapper
on `CloudActions` that mirrors `AiPipeline::classify_stored`'s discipline:
read under brief lock, release, HTTP, re-lock to write. Steps 2+ use it.

- [ ] **Step 1: Add `CloudActions::draft_reply_via` in core**

Open `core/src/ai/cloud/actions/mod.rs`. Add a new method on `impl CloudActions`:

```rust
pub async fn draft_reply_via(
    &self,
    store: std::sync::Arc<std::sync::Mutex<Store>>,
    message_id: Uuid,
    cfg: CloudConfig,
) -> Result<DraftOutcome> {
    // draft::draft_reply takes &Store; we can't hold a MutexGuard across
    // the HTTP .await (MutexGuard is !Send). Work around by running the
    // whole operation on a dedicated blocking thread that uses tokio's
    // current-thread runtime to poll draft_reply. Tauri gives us
    // spawn_blocking via tokio.
    let provider = self.provider.clone();
    let redactor = self.redactor.clone();
    let retriever = self.retriever.clone();
    let profile = self.profile.clone();
    let model = self.model.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| crate::error::CoreError::Other(format!("runtime: {}", e)))?;
        let guard = store.lock().map_err(|e| {
            crate::error::CoreError::Other(format!("store mutex poisoned: {}", e))
        })?;
        rt.block_on(super::actions::draft::draft_reply(
            &*guard,
            provider,
            &redactor,
            retriever.as_ref(),
            &profile,
            message_id,
            cfg,
            &model,
        ))
    })
    .await
    .map_err(|e| crate::error::CoreError::Other(format!("spawn_blocking: {}", e)))??;

    Ok(outcome)
}
```

Note: `Redactor` must be `Clone` for this to compile. If it isn't, either
derive `Clone` on `Redactor` in `core/src/ai/cloud/redactor.rs` (the inner
regex cache is Clone — `Vec<(String, Regex)>` works because `Regex: Clone`)
or wrap it in `Arc<Redactor>` inside `CloudActions`. Smallest diff: derive
`Clone`.

If `CoreError` doesn't have an `Other(String)` variant on master, use
whichever variant the rest of the codebase uses for "unexpected internal
error" — `CoreError::Channel(...)` and `CoreError::InvalidInput(...)` both
exist and are acceptable stand-ins.

- [ ] **Step 2: Add DTOs**

In `desktop/src-tauri/src/commands.rs`:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftDto {
    pub draft_id: String,
    pub body: String,
    pub confidence: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftSummaryDto {
    pub id: String,
    pub created_at: String,
    pub confidence: f32,
    pub preview: String,
    pub has_user_edit: bool,
}

impl From<&messagehub_core::store::DraftRecord> for AiDraftSummaryDto {
    fn from(d: &messagehub_core::store::DraftRecord) -> Self {
        let body = d.user_edited_output.as_deref().unwrap_or(&d.output);
        let preview: String = body.chars().take(80).collect();
        Self {
            id: d.id.to_string(),
            created_at: d.created_at.clone(),
            confidence: d.confidence,
            preview,
            has_user_edit: d.user_edited_output.is_some(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatusDto {
    pub configured: bool,
    pub model: Option<String>,
}
```

- [ ] **Step 3: Add the commands**

```rust
#[tauri::command]
pub async fn ai_draft_reply(
    message_id: String,
    redact: bool,
    state: State<'_, AppState>,
) -> Result<AiDraftDto, String> {
    use messagehub_core::ai::cloud::CloudConfig;

    let msg = Uuid::parse_str(&message_id).map_err(|e| format!("bad message_id: {}", e))?;
    let cloud = state
        .cloud
        .clone()
        .ok_or_else(|| "Cloud not configured — add [cloud] to messagehub.toml".to_string())?;

    // Use the Mutex-aware variant from Step 1: it handles the lock discipline
    // internally (spawn_blocking + scoped MutexGuard). Cloning the Arc is
    // cheap; the blocking task takes ownership for the duration of the call.
    let store = state.store.clone();
    let outcome = cloud
        .draft_reply_via(store, msg, CloudConfig { redact })
        .await
        .map_err(|e| format!("draft_reply: {}", e))?;

    Ok(AiDraftDto {
        draft_id: outcome.id.to_string(),
        body: outcome.output,
        confidence: outcome.confidence,
    })
}

#[tauri::command]
pub fn list_ai_drafts(
    message_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AiDraftSummaryDto>, String> {
    let msg = Uuid::parse_str(&message_id).map_err(|e| format!("bad message_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    let rows = store
        .list_drafts_for_message(&msg)
        .map_err(|e| format!("list_drafts_for_message: {}", e))?;
    Ok(rows
        .iter()
        .filter(|r| r.action_type == "draft_reply")
        .map(AiDraftSummaryDto::from)
        .collect())
}

#[tauri::command]
pub fn cloud_config_status(state: State<'_, AppState>) -> Result<CloudStatusDto, String> {
    // CloudActions doesn't expose a getter for model on master; we stored
    // it on AppState alongside the handle. If that field isn't present,
    // add it in Task 9's AppState and propagate here.
    Ok(CloudStatusDto {
        configured: state.cloud.is_some(),
        model: state.cloud_model.clone(),
    })
}
```

If your Task 9 didn't add a `cloud_model: Option<String>` field on `AppState`, add it now — store the model string alongside the `CloudActions` handle during init.

- [ ] **Step 4: Register commands**

In both `tauri::generate_handler![...]` blocks in `main.rs`, append:

```rust
commands::ai_draft_reply,
commands::list_ai_drafts,
commands::cloud_config_status,
```

- [ ] **Step 5: Add a DTO test**

In the `#[cfg(test)]` block of `commands.rs`:

```rust
#[test]
fn ai_draft_summary_dto_from_draft_record() {
    use messagehub_core::store::DraftRecord;
    let d = DraftRecord {
        id: Uuid::new_v4(),
        message_id: Some(Uuid::new_v4()),
        action_type: "draft_reply".into(),
        input_redacted: "body".into(),
        output: "Thanks!".into(),
        user_edited_output: None,
        confidence: 0.73,
        provider: "anthropic".into(),
        model: "claude-sonnet-4-6".into(),
        created_at: "2026-04-21T10:00:00Z".into(),
    };
    let dto = AiDraftSummaryDto::from(&d);
    assert_eq!(dto.preview, "Thanks!");
    assert!(!dto.has_user_edit);
    assert!((dto.confidence - 0.73).abs() < 1e-6);
}

#[test]
fn ai_draft_summary_uses_user_edit_when_present() {
    use messagehub_core::store::DraftRecord;
    let d = DraftRecord {
        id: Uuid::new_v4(),
        message_id: Some(Uuid::new_v4()),
        action_type: "draft_reply".into(),
        input_redacted: "body".into(),
        output: "original".into(),
        user_edited_output: Some("edited".into()),
        confidence: 0.5,
        provider: "anthropic".into(),
        model: "claude-sonnet-4-6".into(),
        created_at: "2026-04-21T10:00:00Z".into(),
    };
    let dto = AiDraftSummaryDto::from(&d);
    assert_eq!(dto.preview, "edited");
    assert!(dto.has_user_edit);
}
```

- [ ] **Step 6: Build + test the full workspace**

```bash
cargo build --workspace
cargo test -p messagehub-core --lib
cargo test -p messagehub-desktop
```

Expected: clean build across both crates; the new `draft_reply_via`
method doesn't break any existing cloud tests (they all call the
underlying `draft_reply` directly).

- [ ] **Step 7: Commit**

```bash
git add core/src/ai/cloud/actions/mod.rs \
        core/src/ai/cloud/redactor.rs \
        desktop/src-tauri/src/commands.rs \
        desktop/src-tauri/src/main.rs \
        desktop/src-tauri/src/state.rs
git commit -m "feat: CloudActions::draft_reply_via + AI Tauri commands"
```

---

### Task 13: Frontend types + `api.ts` wrappers

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/api.ts`

- [ ] **Step 1: Add DTO types**

Open `desktop/src/types.ts` and append:

```ts
export interface ReplyDraft {
  threadId: string;
  inReplyToMessageId: string;
  body: string;
  subject: string | null;
  updatedAt: string;
}

export interface AiDraft {
  draftId: string;
  body: string;
  confidence: number;
}

export interface AiDraftSummary {
  id: string;
  createdAt: string;
  confidence: number;
  preview: string;
  hasUserEdit: boolean;
}

export interface CloudStatus {
  configured: boolean;
  model: string | null;
}
```

- [ ] **Step 2: Add the wrappers**

Open `desktop/src/api.ts` and append:

```ts
import { invoke } from "@tauri-apps/api/core";
import type {
  ReplyDraft,
  AiDraft,
  AiDraftSummary,
  CloudStatus,
} from "./types";

export function saveReplyDraft(
  threadId: string,
  inReplyToMessageId: string,
  body: string,
  subject: string | null,
): Promise<void> {
  return invoke("save_reply_draft", {
    threadId,
    inReplyToMessageId,
    body,
    subject,
  });
}

export function getReplyDraft(threadId: string): Promise<ReplyDraft | null> {
  return invoke("get_reply_draft", { threadId });
}

export function deleteReplyDraft(threadId: string): Promise<void> {
  return invoke("delete_reply_draft", { threadId });
}

export function sendEmailReply(
  threadId: string,
  inReplyToMessageId: string,
  body: string,
  subject: string,
): Promise<void> {
  return invoke("send_email_reply", {
    threadId,
    inReplyToMessageId,
    body,
    subject,
  });
}

export function aiDraftReply(
  messageId: string,
  redact: boolean,
): Promise<AiDraft> {
  return invoke("ai_draft_reply", { messageId, redact });
}

export function listAiDrafts(messageId: string): Promise<AiDraftSummary[]> {
  return invoke("list_ai_drafts", { messageId });
}

export function cloudConfigStatus(): Promise<CloudStatus> {
  return invoke("cloud_config_status");
}
```

(If `api.ts` already has an `invoke` import, reuse it — don't create a second import.)

- [ ] **Step 3: Type-check**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/types.ts desktop/src/api.ts
git commit -m "feat(desktop): frontend DTOs + api.ts wrappers for reply flow"
```

---

### Task 14: `useAutosave` hook

**Files:**
- Create: `desktop/src/hooks/useAutosave.ts`

- [ ] **Step 1: Create the hook**

Create `desktop/src/hooks/useAutosave.ts`:

```ts
import { useEffect, useRef } from "react";

/**
 * Debounced autosave. Calls `onSave(value)` once the value has been stable
 * for `delayMs`. Also flushes a final save on unmount if the value has
 * changed since the last successful save. Save errors are swallowed — the
 * next change will trigger another attempt.
 */
export function useAutosave<T>(
  value: T,
  delayMs: number,
  onSave: (value: T) => Promise<void>,
): void {
  const lastSavedRef = useRef<T>(value);
  const timerRef = useRef<number | null>(null);
  const onSaveRef = useRef(onSave);

  // Keep onSave fresh without re-arming the timer on every re-render.
  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  useEffect(() => {
    if (Object.is(value, lastSavedRef.current)) {
      return;
    }
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
    }
    const handle = window.setTimeout(() => {
      const snapshot = value;
      onSaveRef
        .current(snapshot)
        .then(() => {
          lastSavedRef.current = snapshot;
        })
        .catch((err) => {
          console.error("useAutosave: save failed", err);
        });
    }, delayMs);
    timerRef.current = handle;
    return () => {
      window.clearTimeout(handle);
    };
  }, [value, delayMs]);

  // Final flush on unmount if dirty.
  useEffect(() => {
    return () => {
      if (!Object.is(value, lastSavedRef.current)) {
        onSaveRef.current(value).catch((err) => {
          console.error("useAutosave: final flush failed", err);
        });
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
```

- [ ] **Step 2: Type-check**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

- [ ] **Step 3: Commit**

```bash
git add desktop/src/hooks/useAutosave.ts
git commit -m "feat(desktop): useAutosave hook"
```

---

### Task 15: `InboxContext` — `replyFor` state

**Files:**
- Modify: `desktop/src/state/InboxContext.tsx`

- [ ] **Step 1: Read the current reducer**

Open `desktop/src/state/InboxContext.tsx`. Identify the `State`, `Action`, and reducer `switch`.

- [ ] **Step 2: Extend state**

Add to the state interface:

```ts
  replyFor: { messageId: string; threadId: string } | null;
```

In the initial state object, add:

```ts
  replyFor: null,
```

- [ ] **Step 3: Extend actions**

Add to the action union:

```ts
  | { type: "OPEN_REPLY"; messageId: string; threadId: string }
  | { type: "CLOSE_REPLY" };
```

- [ ] **Step 4: Handle in the reducer**

Add two cases to the reducer `switch`:

```ts
    case "OPEN_REPLY":
      return { ...state, replyFor: { messageId: action.messageId, threadId: action.threadId } };
    case "CLOSE_REPLY":
      return { ...state, replyFor: null };
```

- [ ] **Step 5: Export helper action creators (optional but tidy)**

If the file already exports `dispatchOpenMessage` / similar helpers, add:

```ts
export function openReply(dispatch: Dispatch<Action>, messageId: string, threadId: string) {
  dispatch({ type: "OPEN_REPLY", messageId, threadId });
}

export function closeReply(dispatch: Dispatch<Action>) {
  dispatch({ type: "CLOSE_REPLY" });
}
```

- [ ] **Step 6: Type-check**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src/state/InboxContext.tsx
git commit -m "feat(desktop): InboxContext adds replyFor + OPEN_REPLY/CLOSE_REPLY"
```

---

### Task 16: `ReplyModal` component — skeleton + autosave + send

**Files:**
- Create: `desktop/src/components/ReplyModal.tsx`

`★ Why this matters:` The heart of the feature. Mounts, hydrates from `get_reply_draft`, autosaves, sends, renders the failure banner. AI panel wiring is Task 18; this task uses a placeholder right-side div so the modal renders end-to-end first.

- [ ] **Step 1: Create the component**

Create `desktop/src/components/ReplyModal.tsx`:

```tsx
import { useEffect, useState } from "react";
import {
  deleteReplyDraft,
  getMessage,
  getReplyDraft,
  saveReplyDraft,
  sendEmailReply,
} from "../api";
import { useAutosave } from "../hooks/useAutosave";

interface Props {
  messageId: string;
  threadId: string;
  onClose: () => void;
}

/**
 * Build the quoted-original block inserted below the cursor on a blank
 * compose. Two leading newlines separate the user's reply from the quote.
 */
function quotedOriginal(
  senderName: string,
  timestamp: string,
  body: string,
): string {
  const when = new Date(timestamp).toLocaleString();
  const quoted = body
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  return `\n\n> On ${when}, ${senderName} wrote:\n${quoted}\n`;
}

function ensureRePrefix(subject: string | null | undefined): string {
  const s = (subject ?? "").trim();
  if (!s) return "Re:";
  return /^re:/i.test(s) ? s : `Re: ${s}`;
}

export function ReplyModal({ messageId, threadId, onClose }: Props) {
  const [body, setBody] = useState<string>("");
  const [subject, setSubject] = useState<string>("Re:");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  // Mount: hydrate from existing draft if any, else seed with quoted original.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [draft, msg] = await Promise.all([
          getReplyDraft(threadId),
          getMessage(messageId),
        ]);
        if (cancelled) return;
        setSubject(ensureRePrefix(msg.subject));
        if (draft && draft.body.length > 0) {
          setBody(draft.body);
        } else {
          setBody(quotedOriginal(msg.senderName, msg.timestamp, msg.body));
        }
        setLoaded(true);
      } catch (err) {
        console.error("ReplyModal mount failed:", err);
        if (!cancelled) setSendError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [messageId, threadId]);

  useAutosave(body, 5000, async (value) => {
    if (!loaded) return;
    await saveReplyDraft(threadId, messageId, value, subject);
  });

  async function handleSend() {
    setSending(true);
    setSendError(null);
    try {
      await sendEmailReply(threadId, messageId, body, subject);
      onClose();
    } catch (err) {
      setSendError(typeof err === "string" ? err : String(err));
    } finally {
      setSending(false);
    }
  }

  async function handleDiscard() {
    try {
      await deleteReplyDraft(threadId);
    } catch (err) {
      console.error("deleteReplyDraft failed:", err);
    }
    onClose();
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void handleSend();
    }
  }

  // Esc to close (Cancel — keeps the draft row).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canSend = body.trim().length > 0 && !sending && loaded;

  return (
    <div className="reply-modal-backdrop" onClick={onClose}>
      <div
        className="reply-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Reply"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="reply-modal-compose">
          <div className="reply-modal-header">
            <input
              className="reply-modal-subject"
              type="text"
              value={subject}
              readOnly
              aria-label="Subject"
            />
          </div>
          {sendError != null && (
            <div className="reply-modal-error" role="alert">
              <span>{sendError}</span>
              <button
                className="reply-modal-error-dismiss"
                onClick={() => setSendError(null)}
                aria-label="Dismiss error"
              >
                ✕
              </button>
            </div>
          )}
          <textarea
            className="reply-modal-body"
            value={body}
            onChange={(e) => setBody(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={loaded ? "Write your reply..." : "Loading..."}
            autoFocus
            disabled={!loaded}
          />
          <div className="reply-modal-actions">
            <button
              className="reply-modal-send"
              onClick={handleSend}
              disabled={!canSend}
            >
              {sending ? "Sending..." : "Send"}
            </button>
            <button className="reply-modal-discard" onClick={handleDiscard}>
              Discard
            </button>
            <button className="reply-modal-cancel" onClick={onClose}>
              Cancel
            </button>
          </div>
        </div>
        <div className="reply-modal-aside">
          {/* Task 18 replaces this with AiAssistPanel. */}
          <div className="reply-modal-ai-stub">AI assist (Task 18)</div>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Type-check**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Expected: no errors. If `getMessage` returns a DTO with different field names than `senderName` / `body` / `subject` / `timestamp`, grep `desktop/src/types.ts` for the actual shape and update `quotedOriginal` accordingly.

- [ ] **Step 3: Commit**

```bash
git add desktop/src/components/ReplyModal.tsx
git commit -m "feat(desktop): ReplyModal skeleton with autosave + send"
```

---

### Task 17: `MessageDetail` — Reply button + mount `ReplyModal`

**Files:**
- Modify: `desktop/src/components/MessageDetail.tsx`
- Modify: `desktop/src/App.tsx`

- [ ] **Step 1: Add the Reply button to `MessageDetail`**

Open `desktop/src/components/MessageDetail.tsx`. Find the detail header / toolbar area. Add a Reply button that is only rendered for Email messages:

```tsx
{message.channel === "Email" && (
  <button
    className="message-detail-reply"
    onClick={() =>
      dispatch({
        type: "OPEN_REPLY",
        messageId: message.id,
        threadId: message.threadId,
      })
    }
  >
    Reply
  </button>
)}
```

(If `MessageDetail` doesn't currently pull `dispatch` from `InboxContext`, import `useInbox` or equivalent and destructure it.)

- [ ] **Step 2: Render the modal from `App`**

Open `desktop/src/App.tsx`. Import `ReplyModal` and `useInbox` state. After the three-pane grid, render:

```tsx
{state.replyFor != null && (
  <ReplyModal
    messageId={state.replyFor.messageId}
    threadId={state.replyFor.threadId}
    onClose={() => dispatch({ type: "CLOSE_REPLY" })}
  />
)}
```

- [ ] **Step 3: Add minimal modal CSS**

Open `desktop/src/App.css` and append:

```css
.reply-modal-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.55);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}

.reply-modal {
  background: var(--surface, #1e1e1e);
  color: var(--text, #ddd);
  border: 1px solid var(--border, #333);
  border-radius: 8px;
  width: min(820px, 90vw);
  height: min(560px, 85vh);
  display: grid;
  grid-template-columns: 1fr 240px;
  gap: 0;
  overflow: hidden;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
}

.reply-modal-compose {
  display: flex;
  flex-direction: column;
  padding: 12px;
  min-width: 0;
}

.reply-modal-subject {
  width: 100%;
  background: transparent;
  border: 0;
  border-bottom: 1px solid var(--border, #333);
  padding: 6px 4px;
  font-size: 14px;
  color: var(--text, #ddd);
}

.reply-modal-error {
  background: #3a1e1e;
  border: 1px solid #6b3434;
  color: #f0b0b0;
  padding: 8px 10px;
  margin: 8px 0;
  border-radius: 4px;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.reply-modal-error-dismiss {
  background: transparent;
  border: 0;
  color: #f0b0b0;
  cursor: pointer;
  font-size: 14px;
}

.reply-modal-body {
  flex: 1;
  background: var(--bg, #111);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 10px;
  color: var(--text, #ddd);
  font-family: inherit;
  font-size: 13px;
  resize: none;
  margin-top: 8px;
}

.reply-modal-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.reply-modal-send {
  background: #4a6bcc;
  color: white;
  border: 0;
  padding: 6px 14px;
  border-radius: 4px;
  cursor: pointer;
}

.reply-modal-send:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.reply-modal-discard,
.reply-modal-cancel {
  background: transparent;
  color: var(--text-muted, #bbb);
  border: 1px solid var(--border, #333);
  padding: 6px 12px;
  border-radius: 4px;
  cursor: pointer;
}

.reply-modal-aside {
  background: var(--surface-alt, #161616);
  border-left: 1px solid var(--border, #333);
  padding: 12px;
  overflow: auto;
}

.reply-modal-ai-stub {
  color: var(--text-muted, #888);
  font-size: 12px;
}

.message-detail-reply {
  background: #4a6bcc;
  color: white;
  border: 0;
  padding: 4px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}
```

- [ ] **Step 4: Type-check + run the dev app briefly**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Manual: start the dev server, open an email, click Reply, verify the modal appears with quoted-original prefilled and Esc closes it. Do not send yet.

```bash
cd desktop && npm run tauri dev
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src/components/MessageDetail.tsx desktop/src/App.tsx desktop/src/App.css
git commit -m "feat(desktop): Reply button + modal wiring"
```

---

### Task 18: `AiAssistPanel` + `PriorDraftsDropdown`

**Files:**
- Create: `desktop/src/components/AiAssistPanel.tsx`
- Create: `desktop/src/components/PriorDraftsDropdown.tsx`
- Modify: `desktop/src/components/ReplyModal.tsx`

- [ ] **Step 1: Create `AiAssistPanel`**

Create `desktop/src/components/AiAssistPanel.tsx`:

```tsx
import { useEffect, useState } from "react";
import {
  aiDraftReply,
  cloudConfigStatus,
  listAiDrafts,
} from "../api";
import type { AiDraftSummary } from "../types";
import { PriorDraftsDropdown } from "./PriorDraftsDropdown";

interface Props {
  messageId: string;
  onDraftReady: (body: string, confidence: number) => void;
}

export function AiAssistPanel({ messageId, onDraftReady }: Props) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(false);
  const [confidence, setConfidence] = useState<number | null>(null);
  const [redact, setRedact] = useState(true);
  const [priorDrafts, setPriorDrafts] = useState<AiDraftSummary[]>([]);
  const [priorOpen, setPriorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cfg, drafts] = await Promise.all([
          cloudConfigStatus(),
          listAiDrafts(messageId),
        ]);
        if (cancelled) return;
        setConfigured(cfg.configured);
        setPriorDrafts(drafts);
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [messageId]);

  async function handleGenerate() {
    if (loading) return;
    setLoading(true);
    setError(null);
    try {
      const out = await aiDraftReply(messageId, redact);
      onDraftReady(out.body, out.confidence);
      setConfidence(out.confidence);
      const drafts = await listAiDrafts(messageId);
      setPriorDrafts(drafts);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setLoading(false);
    }
  }

  if (configured === null) {
    return <div className="ai-panel">Loading...</div>;
  }
  if (!configured) {
    return (
      <div className="ai-panel ai-panel-disabled">
        <div className="ai-panel-title">AI assist</div>
        <p className="ai-panel-hint">
          Not configured — add <code>[cloud]</code> to{" "}
          <code>messagehub.toml</code>
        </p>
      </div>
    );
  }

  const btnLabel = priorDrafts.length === 0 ? "Generate draft" : "Regenerate";

  return (
    <div className="ai-panel">
      <div className="ai-panel-title">
        AI assist
        {confidence != null && (
          <span className="ai-panel-chip">
            {confidence.toFixed(2)}
          </span>
        )}
      </div>
      <button
        className="ai-panel-generate"
        onClick={handleGenerate}
        disabled={loading}
      >
        {loading ? "Generating..." : btnLabel}
      </button>
      <label className="ai-panel-redact">
        <input
          type="checkbox"
          checked={redact}
          onChange={(e) => setRedact(e.target.checked)}
        />{" "}
        Redact PII
      </label>
      <button
        className="ai-panel-prior-toggle"
        onClick={() => setPriorOpen((x) => !x)}
        disabled={priorDrafts.length === 0}
      >
        Prior drafts ({priorDrafts.length}) {priorOpen ? "▴" : "▾"}
      </button>
      {priorOpen && (
        <PriorDraftsDropdown
          drafts={priorDrafts}
          onRestore={(body) => onDraftReady(body, 0)}
        />
      )}
      {error && <div className="ai-panel-error">{error}</div>}
    </div>
  );
}
```

- [ ] **Step 2: Create `PriorDraftsDropdown`**

Create `desktop/src/components/PriorDraftsDropdown.tsx`:

```tsx
import type { AiDraftSummary } from "../types";

interface Props {
  drafts: AiDraftSummary[];
  onRestore: (body: string) => void;
}

export function PriorDraftsDropdown({ drafts, onRestore }: Props) {
  return (
    <ul className="prior-drafts">
      {drafts.map((d) => (
        <li key={d.id} className="prior-draft-row">
          <div className="prior-draft-meta">
            <span>{new Date(d.createdAt).toLocaleString()}</span>
            <span>·</span>
            <span>conf {d.confidence.toFixed(2)}</span>
            {d.hasUserEdit && <span className="prior-draft-edited">(edited)</span>}
          </div>
          <div className="prior-draft-preview">{d.preview}</div>
          <button
            className="prior-draft-restore"
            onClick={() => onRestore(d.preview)}
          >
            Restore
          </button>
        </li>
      ))}
    </ul>
  );
}
```

Note: `prior-draft-restore` passes `d.preview` (80 chars max) to `onRestore` for the first iteration. If UAT confirms the full body is needed, add a `fullBody` field to `AiDraftSummaryDto` in a follow-up — the preview is all the UI gets today.

Actually — Restore overwriting with 80 chars is user-hostile. Fix it now: extend `AiDraftSummaryDto` to include the full stored output.

- [ ] **Step 3: Extend `AiDraftSummaryDto` with full body**

In `desktop/src-tauri/src/commands.rs`, edit `AiDraftSummaryDto` and the `From` impl:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftSummaryDto {
    pub id: String,
    pub created_at: String,
    pub confidence: f32,
    pub preview: String,
    pub body: String,           // NEW — full stored output for Restore.
    pub has_user_edit: bool,
}

impl From<&messagehub_core::store::DraftRecord> for AiDraftSummaryDto {
    fn from(d: &messagehub_core::store::DraftRecord) -> Self {
        let body = d.user_edited_output.as_deref().unwrap_or(&d.output).to_string();
        let preview: String = body.chars().take(80).collect();
        Self {
            id: d.id.to_string(),
            created_at: d.created_at.clone(),
            confidence: d.confidence,
            preview,
            body,
            has_user_edit: d.user_edited_output.is_some(),
        }
    }
}
```

Update `desktop/src/types.ts` — add `body: string;` to `AiDraftSummary`.

Change the restore call in `PriorDraftsDropdown`:

```tsx
onClick={() => onRestore(d.body)}
```

- [ ] **Step 4: Wire `AiAssistPanel` into `ReplyModal`**

In `ReplyModal.tsx`, replace the `reply-modal-ai-stub` placeholder:

```tsx
<AiAssistPanel
  messageId={messageId}
  onDraftReady={(text, _conf) => {
    setBody(text);
    void saveReplyDraft(threadId, messageId, text, subject);
  }}
/>
```

Add the import at the top: `import { AiAssistPanel } from "./AiAssistPanel";`

- [ ] **Step 5: Add CSS for the AI panel**

Append to `desktop/src/App.css`:

```css
.ai-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  font-size: 12px;
}
.ai-panel-title {
  font-weight: 600;
  color: var(--text, #ddd);
  display: flex;
  align-items: center;
  gap: 8px;
}
.ai-panel-chip {
  background: #2a3a2a;
  color: #7bc47b;
  padding: 1px 8px;
  border-radius: 10px;
  font-size: 10px;
}
.ai-panel-generate {
  background: #4a6bcc;
  color: white;
  border: 0;
  padding: 6px 10px;
  border-radius: 4px;
  cursor: pointer;
}
.ai-panel-generate:disabled {
  opacity: 0.5;
}
.ai-panel-redact {
  display: flex;
  gap: 6px;
  align-items: center;
  color: var(--text-muted, #bbb);
}
.ai-panel-prior-toggle {
  background: transparent;
  border: 1px solid var(--border, #333);
  color: var(--text, #ddd);
  padding: 4px 8px;
  border-radius: 4px;
  cursor: pointer;
  text-align: left;
}
.ai-panel-prior-toggle:disabled {
  opacity: 0.5;
  cursor: default;
}
.ai-panel-error {
  background: #3a1e1e;
  color: #f0b0b0;
  padding: 6px 8px;
  border-radius: 4px;
}
.ai-panel-hint code {
  background: #222;
  padding: 1px 4px;
  border-radius: 3px;
}
.prior-drafts {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.prior-draft-row {
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 6px;
  font-size: 11px;
}
.prior-draft-meta {
  display: flex;
  gap: 4px;
  color: var(--text-muted, #888);
}
.prior-draft-edited {
  color: #bbb;
}
.prior-draft-preview {
  margin: 4px 0;
  color: var(--text, #ddd);
}
.prior-draft-restore {
  background: transparent;
  border: 1px solid var(--border, #333);
  color: var(--text, #ddd);
  padding: 2px 8px;
  border-radius: 3px;
  cursor: pointer;
}
```

- [ ] **Step 6: Type-check + build**

```bash
cd desktop
npx tsc --noEmit
cd ..
cargo build -p messagehub-desktop
```

Expected: both clean.

- [ ] **Step 7: Commit**

```bash
git add desktop/src/components/AiAssistPanel.tsx \
        desktop/src/components/PriorDraftsDropdown.tsx \
        desktop/src/components/ReplyModal.tsx \
        desktop/src/types.ts \
        desktop/src/App.css \
        desktop/src-tauri/src/commands.rs
git commit -m "feat(desktop): AiAssistPanel + PriorDraftsDropdown wired to ReplyModal"
```

---

### Task 19: Regenerate test — `cloud_draft_test.rs` extension

**Files:**
- Modify: `core/tests/cloud_draft_test.rs`

`★ Why this matters:` The spec calls out regenerate as a first-class flow. Locking in "two `draft_reply` calls = two `ai_drafts` rows" prevents a regression where a future refactor de-duplicates by message_id and silently breaks history.

- [ ] **Step 1: Read the existing test to find the wiremock setup**

```bash
grep -n "fn \|wiremock\|MockServer" core/tests/cloud_draft_test.rs
```

Identify a happy-path test and the mock builder helper.

- [ ] **Step 2: Append the regenerate case**

Add a new `#[tokio::test]` (or `#[test]` matching the file's style) that:
1. Sets up the same wiremock server and CloudActions as the happy-path test.
2. Calls `draft_reply` twice with the same `message_id`.
3. Asserts `store.list_drafts_for_message(&message_id).unwrap().len() == 2`.
4. Asserts both rows have `action_type == "draft_reply"`.

The exact constructor calls must match whatever the existing tests use; copy the setup block.

- [ ] **Step 3: Run**

```bash
cargo test -p messagehub-core --test cloud_draft_test
```

- [ ] **Step 4: Commit**

```bash
git add core/tests/cloud_draft_test.rs
git commit -m "test(core): regenerate inserts a second ai_drafts row"
```

---

### Task 20: Manual UAT

**Files:** (none — manual verification)

`★ Why this matters:` The frontend has no automated harness. These ten checks are the acceptance criteria before merging.

Prerequisites:
- `messagehub.toml` in `core/` has at least one `[[channels]]` block with `kind = "email"` pointing at a reachable SMTP server (maildev, a real Gmail app-password account, etc.).
- Run `cargo run --bin runtime-demo -- --config core/messagehub.toml` in one terminal to ingest at least one email for UAT.
- For AI steps (5–7, 11), add `[cloud]` with a valid Anthropic API key and model.

Run the app:

```bash
cd desktop && npm run tauri dev
```

- [ ] **Check 1: Modal opens with quoted original**
Click an email, click **Reply**. Modal mounts. Subject is `Re: <original>` and read-only. Textarea has two blank lines at top followed by `> On <date>, <sender> wrote:` and the original body prefixed with `> `. Cursor is at position 0.

- [ ] **Check 2: Autosave + restore**
Type a few lines at the top. Wait 6 s. Close the modal (click backdrop or press Esc). Reopen Reply on the same email. Body is restored as you left it.

- [ ] **Check 3: AI panel disabled without config**
Set `[cloud].enabled = false` (or remove the section), restart the app. Open a reply modal. Right-side panel renders the hint "Not configured — add [cloud] to messagehub.toml". No buttons are clickable.

- [ ] **Check 4: Generate draft**
Re-enable `[cloud]`, restart. Open a reply modal. Click **Generate draft**. Within ~5 s, textarea body is overwritten with the AI output. Confidence chip appears in the header with a value like `0.70`. Prior-drafts count goes from `(0)` to `(1)`.

- [ ] **Check 5: Regenerate**
Click **Regenerate**. Textarea overwrites with a new draft. Confidence updates. Prior-drafts count: `(2)`.

- [ ] **Check 6: Restore prior**
Click the **Prior drafts (2) ▾** button. Two rows appear with timestamp + confidence + preview + Restore. Click Restore on the first row. Textarea shows that row's full body.

- [ ] **Check 7: Redact toggle**
Uncheck **Redact PII**. Click Regenerate. Inspect `ai_drafts.input_redacted` for the newest row:

```bash
sqlite3 core/messagehub.db \
  "SELECT input_redacted FROM ai_drafts ORDER BY created_at DESC LIMIT 1;"
```

(Adjust DB path if different. You may need to close the app to release the exclusive SQLCipher lock, or use the SQLCipher CLI with the password.)

Verify the stored `input_redacted` matches the original un-redacted body (no `[PERSON_1]` / `[EMAIL_1]` placeholders).

- [ ] **Check 8: SMTP failure path**
Disconnect the network (or point `smtp_host` to an unreachable host and restart). Open reply, write a body, click **Send**. Within ~30 s an inline red banner renders with the SMTP error text. Modal stays open. Verify the `reply_drafts` row still exists:

```bash
sqlite3 core/messagehub.db \
  "SELECT thread_id, length(body) FROM reply_drafts;"
```

- [ ] **Check 9: Successful send**
Reconnect / fix the config. Click **Send**. Modal closes. Verify the row is gone:

```bash
sqlite3 core/messagehub.db "SELECT count(*) FROM reply_drafts;"   # 0
```

- [ ] **Check 10: Threading renders correctly in a different client**
Open the received email in Gmail / Outlook / `mutt` — whatever your SMTP sink routes to. Confirm the reply threads under the original (same conversation, not a new one).

If any check fails, capture the failure and either fix before PR or log a follow-up task in `docs/backlog.md`.

---

### Task 21: PR prep — squash review, docs touch-ups, open PR

**Files:** (no code)

- [ ] **Step 1: Review the commit graph**

```bash
git log --oneline master..HEAD
```

Expected: ~15–20 commits, one per task. No "WIP" / "fix typo" commits — if there are, squash with `git rebase -i master`.

- [ ] **Step 2: Run the full test suite one more time**

```bash
cargo test --workspace
cd desktop && npx tsc --noEmit && cd ..
```

Expected: all green.

- [ ] **Step 3: Update the backlog**

Open `docs/backlog.md`. Add a short resolved entry referencing the PR (or leave for the merge commit):

```markdown
### 7b.3 shipped — first outbound slice (AI-assisted email reply)

Modal composer, autosaved reply drafts, RFC 5322 threading on send,
Anthropic draft_reply wired through the UI with regenerate + prior-drafts
Restore. Email-only; Telegram reply deferred.
```

(Only add this after the PR merges; don't pre-resolve.)

- [ ] **Step 4: Push + open PR**

```bash
git push -u origin feat/reply-composer
```

Use `gh pr create` with a body that references the spec and lists the UAT checks performed.

- [ ] **Step 5: Merge**

After review + all UAT checks pass, merge via the usual `--no-ff` merge commit so the branch boundary is preserved in the history.

---

## Invariants (what this plan must not break)

- **Ingest dedup from 7b.2.1 still holds.** Nothing in this plan touches `messages.external_id` or the unique partial index.
- **Mark-read from 7b.2 still fires on detail open.** `MessageDetail`'s existing effect remains; adding the Reply button is additive.
- **Store mutex lock discipline.** `send_email_reply` grabs the lock, builds a local snapshot, drops it before the SMTP call's `.await`. Same pattern as runtime's reload_channels.

## Out of Scope (for clarity, not to be confused with "skip")

Everything in the spec's Non-Goals list — Telegram reply, Reply-All, Forward, attachments, HTML, outbox retry, in-app settings UI, AI cancellation, multi-account Email disambiguation. If any of these crop up during implementation, write them as follow-up entries in `docs/backlog.md`, don't expand 7b.3.
