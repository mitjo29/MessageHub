# Backlog

Issues and improvements queued for future plans. Each entry has a severity,
discovered-during context, and a proposed fix. When work begins, promote the
item to a proper plan under `docs/superpowers/plans/`.

---

## Resolved

### B-006 — DTO camelCase sweep — **Fixed (2026-04-24)**

Added `#[serde(rename_all = "camelCase")]` to the five 7b.2 DTOs that
lacked it: `MessageRow`, `AttachmentInfo`, `MessageDetail`,
`ChannelInfo`, `UiConfig`. Rewrote `desktop/src/types.ts` field names to
match (`channel_label` → `channelLabel`, `sender_name` → `senderName`,
`is_read` → `isRead`, `size_bytes` → `sizeBytes`, `thread_id` →
`threadId`, `channel_type` → `channelType`, `last_sync_at` →
`lastSyncAt`, `db_path` → `dbPath`, `channel_count` → `channelCount`).

Swept five consuming components — `MessageList.tsx`, `MessageDetail.tsx`,
`ReplyModal.tsx`, `Sidebar.tsx`, `InboxContext.tsx`. Tauri *command*
names (`mark_read`, `sidebar_counts`) stay snake_case — that's the IPC
convention, not the DTO one. Convention documented in `CLAUDE.md`.

Verified: `npm run build` (tsc + vite) passes, `cargo test --workspace`
passes.

### B-009 — `send_email_reply` post-send lock poisoning — **Fixed (2026-04-24)**

`send_email_reply` used `?` on the post-send `store.lock()`, so a
poisoned mutex turned a successful send into a caller-visible error —
exactly the opposite of the spec's "log but return Ok, the email
already left" contract. Replaced with a `match` that log-and-swallows
on both poisoning and `delete_reply_draft` failures. Pure code-path
fix; no test because the poisoning path requires panicking in a
lock-holding thread, and the only interesting observable is "no
error returned from send" which is already covered by the existing
happy-path tests.

### B-007 — `stable_channel_id` duplicated — **Fixed (2026-04-24)**

Extracted the helper to new module `messagehub_core::channel_id`.
`runtime-demo.rs` imports it directly; `desktop/src-tauri/src/state.rs`
re-exports through a one-line wrapper to avoid churning every call
site inside the desktop crate. Four unit tests in the new module,
including a `format_pinned` test that locks in the UUIDv5 output
string — any future byte-level drift in the seed format will fail
this test loudly instead of silently orphaning persisted channel
rows.

### B-005 — Tauri config resolver missing `../messagehub.toml` — **Fixed (2026-04-24)**

Added `../messagehub.toml` as the second candidate in
`resolve_config_path`. From Tauri dev CWD (`desktop/src-tauri/`) it now
finds a TOML placed next to `desktop/`, which is the "obvious"
location users try first.

### B-008 — `UserProfile` always empty — **Fixed (2026-04-24)**

Both the desktop host and `runtime-demo` were constructing their AI
integration points (`CloudActions` / `AiPipeline`) with
`UserProfile { content: String::new() }`. The backlog entry's premise
that "runtime-demo already loads the profile" was wrong — the bug was
symmetric across both binaries.

Fix: new optional `profile_path` key at the root of `messagehub.toml`
(next to `database`). Resolved the same way as `database` (relative →
anchored at TOML parent; absolute → passthrough), then fed through
`UserProfile::load`, which already degrades gracefully on missing file.
Path is threaded into `AppState::init` (desktop) and the `AiPipeline`
builder (runtime-demo). TODO comment removed from `state.rs`.

Deliberately chose a scalar key over a `[knowledge]` block — defers the
vault-layout convention question until the indexer actually gets wired
through config (no plan scheduled for that yet).

### B-004 — `send_email_reply` first-match channel routing — **Fixed in branch 1 (2026-04-24)**

Schema-based fix: migration 008 adds `messages.received_on_channel_id`
(FK → `channels.id`); the ingestor sets it from `IngestJob.channel_id`
at write time; `send_email_reply` now resolves through a new
`resolve_reply_channel` helper that prefers the recorded receiving
channel and falls back to single-variant match for legacy NULL rows
(ambiguous legacy errors rather than guessing). Five unit tests in
`commands::tests::resolver_*` pin the precedence rules. Pre-existing
TODO at `commands.rs:148` for label display is now unblocked but not
yet executed — separate follow-up.

### B-003 — Ingest has no idempotency — **Fixed in Plan 7b.2.1 (2026-04-21)**

Channel runtime re-inserted every message on every poll. Two cascading
bugs from Plan 6: telegram `last_update_id` was never persisted
across polls (`adapters/telegram.rs` said "caller should update this"
and no caller did), and `messages` had no `external_id` column so the
ingestor could not dedup.

Fix landed via Plan 7b.2.1:
- Migration 006 added `messages.external_id` + a unique partial index
  on `(channel_type, external_id) WHERE external_id IS NOT NULL`.
- `insert_message` uses `INSERT ... ON CONFLICT ... WHERE external_id
  IS NOT NULL DO NOTHING` — schema-level enforcement, first-write-wins.
- New `ChannelAdapter::cursor_state` / `set_cursor_state` trait methods
  (default None / Ok(())). Telegram impl serializes `last_update_id`
  as a string, `fetch_messages` advances it on each batch, and the
  channel task hydrates from / writes to `channels.last_sync_cursor`.

7b.2 manual checks #11 (15s poll) and #12 (focus refresh) now pass —
sidebar counts stay stable when runtime-demo re-delivers the same
messages.

### B-001 — LLM timeout path is silent — **Fixed in `2bbc75e` (2026-04-20)**

Surfaced as empty `raw_preview=` in classifier logs when the model was too
slow. `OllamaLlm::complete` now checks `reqwest::Error::is_timeout()` and
returns `CoreError::AiTimeout { timeout_secs }`. `new_with_timeout()` lets
tests exercise short timeouts without the production 60s wait.

### B-002 — Classifier didn't strip markdown code fences — **Already handled; tests added in `597a738` (2026-04-20)**

Investigation during burn-down found `strip_fences` already existed in
`core/src/ai/prompts.rs` and was wired into the classifier parse path — the
reported issue was unreproducible. Added unit tests (plain fence, `json`
fence, passthrough, trailing whitespace) plus an end-to-end classifier test
to lock in the behavior and catch future regressions. The user's original
empty-response symptom was purely B-001 (timeout), not fence-parsing.
