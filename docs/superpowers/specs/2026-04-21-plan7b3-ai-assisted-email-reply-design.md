# Plan 7b.3 — AI-Assisted Email Reply — Design Specification

**Date:** 2026-04-21
**Status:** Approved
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plan 7b.2 merged on master (commit `6fad9b5` or later).

## Overview

Plan 7b.3 turns the read-only three-pane inbox from 7b.2 into a read-and-reply
inbox, but only for Email. The user clicks **Reply** on an email, a modal
composer opens, and they either type a reply by hand or click **Generate
draft** in the side panel to have Anthropic produce one via the existing
`ai/cloud/draft_reply` action. The reply sends through the existing
`EmailAdapter::send_reply`, but with proper RFC 5322 threading headers
(`In-Reply-To`, `References`, a fresh `Message-ID`) so it shows up threaded in
the recipient's client. Drafts autosave to a new `reply_drafts` table so work
survives window close and crashes.

**What it is:** the first outbound slice of the desktop app, and the first UI
surface on top of the cloud-action module that has been sitting unused since
Plan 5. ~800–1000 LOC of real code plus one SQL migration.

**What it is not:** a finished reply experience. No Telegram reply, no
Reply-All, no Forward, no attachments on send, no HTML body, no retry/outbox
for failed sends, no in-app settings screen for the API key, no cancellation
of in-flight AI requests.

## Goals

1. A **Reply** button in `MessageDetail` opens a modal composer for email
   messages. The button is hidden (or disabled) for non-email channels.
2. The modal pre-fills the body with a quoted-original block
   (`> On <date>, <sender> wrote:` + indented original body) below the cursor.
   The user types above it.
3. The body autosaves every 5 s of idle into a new `reply_drafts` table, one
   row per thread. Closing and reopening the modal restores the in-progress
   text.
4. When `[cloud]` is configured in `messagehub.toml`, the modal's side panel
   renders an enabled **AI assist** surface with: **Generate draft**,
   **Regenerate**, a confidence chip, a **Redact PII** toggle, and an
   expandable **Prior drafts** dropdown. Generate calls the existing
   `CloudActions::draft_reply`; the returned text replaces the textarea body.
   Regenerate is the same action with a fresh call.
5. When `[cloud]` is missing or `enabled = false`, the AI panel renders
   disabled with a one-line hint pointing the user at `messagehub.toml`.
6. The **Prior drafts** dropdown lists past AI generations for the original
   message (from `ai_drafts`, newest first), each with timestamp, confidence
   and an 80-char preview. A **Restore** button per row overwrites the
   textarea body with that draft's stored output.
7. The **Send** button sends the reply through a fresh `EmailAdapter`
   instance, with the recipient resolved from the original sender's contact
   identity and `In-Reply-To` / `References` / a fresh `Message-ID` set from
   the original's stored metadata. Subject is locked to `Re: <original>`
   (de-duplicated).
8. On SMTP failure, the modal stays open, an inline red banner shows the
   error, and the draft row stays in place for retry.
9. On SMTP success, the modal closes and the `reply_drafts` row is deleted.

## Non-Goals

- **Telegram reply.** 7b.3 covers Email only. Telegram `send_reply` already
  works at the adapter level; UI is deferred.
- **Reply-All / Forward.** 7b.3 replies only to the original sender.
- **Attachments on send.** Inbound attachments render today in 7b.2; outbound
  attachments (from disk) are deferred.
- **HTML body.** Plain text only. The email is sent with a text body; no
  `multipart/alternative`.
- **Retry / outbox for failed sends.** Send is synchronous. Failures surface
  inline; any durable retry story belongs in a later plan that owns the
  runtime's outbound queuing in general.
- **Cancellation of in-flight AI requests.** If the user closes the modal
  mid-generation, the backend call completes and its result is discarded.
  Anthropic is charged. Explicit YAGNI — threads a `CancellationToken` through
  `CloudActions::draft_reply`; revisit if it bites.
- **In-app settings screen for the Anthropic API key.** Key lives in
  `messagehub.toml` under `[cloud]`, same shape as the existing `[ai]`
  section. A keychain-backed settings UI is a later plan (7b.5 territory).
- **Multi-account Email disambiguation on Reply.** If two Email channels are
  configured, `send_email_reply` picks the first `ChannelConfig` whose
  variant matches the original's channel. Same limitation as 7b.2's sidebar.

## Architecture

Layers touched, newest at the top:

```
┌──────────────────────────────────────────────────────────────┐
│ React (desktop/src/)                                          │
│   ReplyModal.tsx                NEW                           │
│   AiAssistPanel.tsx             NEW  (rendered inside modal)  │
│   PriorDraftsDropdown.tsx       NEW  (inside AiAssistPanel)   │
│   hooks/useAutosave.ts          NEW  (debounced write helper) │
│   state/InboxContext.tsx        MODIFY: replyFor: Uuid | null │
│   components/MessageDetail.tsx  MODIFY: Reply button          │
│   api.ts / types.ts             MODIFY: 7 new wrappers + DTOs │
├──────────────────────────────────────────────────────────────┤
│ Tauri (desktop/src-tauri/src/)                                │
│   commands.rs                   MODIFY: 7 new commands        │
│   state.rs                      MODIFY: CloudActions handle   │
│   config.rs                     MODIFY: [cloud] section       │
│   main.rs                       MODIFY: register + wire cloud │
├──────────────────────────────────────────────────────────────┤
│ Core library                                                  │
│   migrations/007_reply_drafts.sql   NEW  reply_drafts table   │
│   store/reply_drafts.rs             NEW  upsert/get/delete    │
│   store/mod.rs                      MODIFY: re-export         │
│   store/drafts.rs                   MODIFY: list_for_message  │
│   types/message.rs                  MODIFY: ReplyHeaders type │
│   adapters/email.rs                 MODIFY: consume headers   │
│   ai/cloud/actions/draft.rs         (unchanged — reused)      │
└──────────────────────────────────────────────────────────────┘
```

## Data Model

### New migration — `007_reply_drafts.sql`

```sql
CREATE TABLE IF NOT EXISTS reply_drafts (
    thread_id                TEXT PRIMARY KEY,
    in_reply_to_message_id   TEXT NOT NULL,
    body                     TEXT NOT NULL DEFAULT '',
    subject                  TEXT,
    updated_at               TEXT NOT NULL
                             DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

One row per thread. UPSERT on autosave, DELETE on successful send or explicit
discard. `in_reply_to_message_id` pins the specific message being replied to,
so that on send the threading headers can be rebuilt without ambiguity. If
the user clicks Reply on a newer message in the same thread while a draft
exists, the UPSERT silently updates `in_reply_to_message_id` — matches how
most mail clients treat "one pending reply per conversation."

### Type change — `types/message.rs`

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplyHeaders {
    /// Recipient (RFC 5322 address). Extracted from the original's sender
    /// contact identity for the relevant channel.
    pub to: String,
    /// Value for the outgoing In-Reply-To header (the original's Message-ID).
    pub in_reply_to: String,
    /// References chain: the original's existing References header (if any),
    /// then the original's Message-ID appended.
    pub references: Vec<String>,
}

pub struct MessageContent {
    // existing fields...
    pub reply_headers: Option<ReplyHeaders>,   // NEW — additive
}
```

Inbound messages have `reply_headers = None`. The Tauri command that sends a
reply builds a `MessageContent` with `reply_headers = Some(...)` and passes
it to `EmailAdapter::send_reply`. Telegram ignores the field.

### Config — `messagehub.toml`

A new section, modeled on the existing `[ai]` section:

```toml
[cloud]
enabled = true
api_key = "sk-ant-..."
model   = "claude-sonnet-4-6"
```

Plain text in a gitignored TOML file, consistent with the rest of the app's
credentials today (IMAP passwords, Telegram bot tokens). Keychain migration
is a later plan.

## Core Library Changes

### `store/reply_drafts.rs` (new)

```rust
pub struct ReplyDraft {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: String,
    pub subject: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewReplyDraft<'a> {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: &'a str,
    pub subject: Option<&'a str>,
}

impl Store {
    pub fn upsert_reply_draft(&self, draft: &NewReplyDraft) -> Result<()>;
    pub fn get_reply_draft(&self, thread_id: &Uuid) -> Result<Option<ReplyDraft>>;
    pub fn delete_reply_draft(&self, thread_id: &Uuid) -> Result<()>;
}
```

`upsert_reply_draft` uses `INSERT ... ON CONFLICT(thread_id) DO UPDATE SET
...`. `delete_reply_draft` is idempotent. Re-exported through `store/mod.rs`
as `ReplyDraft` and `NewReplyDraft`.

### `store/drafts.rs` (existing, extended)

One new helper for the prior-drafts dropdown:

```rust
pub fn list_drafts_for_message(
    &self,
    message_id: &Uuid,
    action_type: &str,   // "draft_reply"
    limit: u32,
) -> Result<Vec<AiDecision>>;
```

Backed by the existing `idx_ai_drafts_message` index. No schema changes.

### `adapters/email.rs` — `send_reply` update

Today's `send_reply(thread_id, content)` uses `thread_id` as the destination
address and produces a non-threaded message. The new contract:

- If `content.reply_headers.is_some()`:
  - `To:` = `reply_headers.to`
  - `Subject:` = `content.subject.as_deref().unwrap_or("Re:")`
  - `In-Reply-To:` = `reply_headers.in_reply_to` (wrap in `<...>` if bare)
  - `References:` = space-separated list of `reply_headers.references`
    (each wrapped in `<...>` if bare)
  - `Message-ID:` = freshly generated `<uuid@<smtp_host>>`
- If `content.reply_headers.is_none()`: fall through to today's behavior
  unchanged (the runtime never calls `send_reply` today, but leaving the
  fallthrough keeps any future callers working and keeps the diff small).

All lettre builder errors are mapped to `CoreError::InvalidInput` or
`CoreError::Channel` as today.

## Tauri Layer

### `state.rs` — `AppState` extensions

Two additions, both necessary because the Tauri host currently only reads
`database` + `password` from `messagehub.toml`, not the channel credentials
or cloud config that the send path and AI path require:

1. `cloud: Option<Arc<CloudActions>>` — populated on init if `[cloud]` is
   present and `enabled = true`. Host constructs `AnthropicCloud::new(api_key,
   model)`, a `Redactor`, a `UserProfile`, and wraps them in
   `CloudActions::new(...)`. `None` otherwise.

2. `email_connections: HashMap<Uuid, EmailConnection>` where `EmailConnection
   = { imap_host, imap_port, smtp_host, smtp_port, username, password }`.
   Populated on init by reading the `[[channels]]` entries from
   `messagehub.toml` and mapping each to the UUIDv5 id used by runtime-demo
   (`stable_channel_id(kind, label)`). The `send_email_reply` command looks
   up the entry for the original's `ChannelConfig.id` to build the adapter.

   This intentionally duplicates runtime-demo's credential parsing. Unifying
   the two (e.g. a shared `config::parse_channels` helper exported from
   `messagehub-core` or a crate-local util) is a code-organization improvement
   that belongs in a follow-up — 7b.3 adds a second consumer, which is the
   right moment to notice the duplication but the wrong moment to refactor
   it.

### `config.rs` — `[cloud]` + channel credentials

The Tauri-side struct is named `TauriCloudConfig` to avoid colliding with
`messagehub_core::ai::cloud::CloudConfig` (the per-call redaction flag).

```rust
#[derive(Debug, Deserialize)]
pub struct TauriCloudConfig {
    pub enabled: bool,
    pub api_key: Option<String>,  // required if enabled
    pub model: Option<String>,    // required if enabled; no default
}

#[derive(Debug, Deserialize)]
pub struct ChannelEntry {
    pub kind: String,            // "email" | "telegram"
    pub label: String,
    pub enabled: bool,
    pub credentials: toml::Value,
}

#[derive(Debug, Deserialize)]
pub struct EmailCredentials {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
}
```

`load_config` parses both new sections. `try_init` builds the
`email_connections` map and the optional `CloudActions` handle before
constructing `AppState`. If `enabled = true` but `api_key` or `model` is
missing, init logs a warning and behaves as if cloud is disabled (same
degraded state as no section).

### `commands.rs` — new commands

| Command | Signature | Behavior |
|---|---|---|
| `save_reply_draft` | `(thread_id, in_reply_to_message_id, body, subject) -> Result<(), String>` | UPSERT a row via `Store::upsert_reply_draft`. Autosaves fire this every 5 s idle. |
| `get_reply_draft` | `(thread_id) -> Result<Option<ReplyDraftDto>, String>` | Called on modal mount to restore in-progress text. |
| `delete_reply_draft` | `(thread_id) -> Result<(), String>` | Called on explicit Discard. Internally also called after a successful send. |
| `send_email_reply` | `(thread_id, in_reply_to_message_id, body, subject) -> Result<(), String>` | Build `ReplyHeaders`, spin up a throwaway `EmailAdapter`, send, delete the draft row. See **Send Path** below. |
| `ai_draft_reply` | `(message_id, redact) -> Result<AiDraftDto, String>` where `AiDraftDto = { body, confidence, draft_id }` | Thin wrapper: resolves `CloudActions` from `AppState` (errors if `None`), calls `draft_reply(... CloudConfig { redact })`, returns the outcome. |
| `list_ai_drafts` | `(message_id) -> Result<Vec<AiDraftSummaryDto>, String>` | Calls `Store::list_drafts_for_message(message_id, "draft_reply", 10)`, maps each `AiDecision` to `{ id, created_at, confidence, preview, has_user_edit }`. |
| `cloud_config_status` | `() -> CloudStatusDto = { configured, model }` | Reads `AppState.cloud` to tell the UI whether the panel should render enabled. |

### Send Path — `send_email_reply` step by step

1. Parse UUIDs. Load the original message via `Store::get_message(in_reply_to_message_id)`.
2. Find the originating `ChannelConfig`: iterate `list_channel_configs()`,
   take the first whose `.channel == message.channel` (documented multi-
   account limitation).
3. Resolve recipient: load the original sender's contact via
   `Store::get_contact(message.sender_id)`; find the `ContactIdentity` whose
   `channel == Channel::Email`; use its `address`. If none, return
   `Err("No recipient address known for this contact on Email")`.
4. Build `ReplyHeaders`:
   - `to` = the address from step 3
   - `in_reply_to` = `message.metadata["message_id"]` (required; error if absent)
   - `references` = split `message.metadata["references"]` by whitespace,
     trim `<...>`, then append `message.metadata["message_id"]`
5. Build `MessageContent { subject: Some(subject), text: Some(body),
   reply_headers: Some(headers), ..Default::default() }`. Subject was built
   client-side as `"Re: <stripped>"` but we re-apply the `Re:` guard
   server-side for safety.
6. Look up `AppState.email_connections[channel_config.id]` to get the
   `{ imap_host, imap_port, smtp_host, smtp_port, username, password }`.
   If absent (e.g. channel row exists in DB but was removed from
   `messagehub.toml`), return `Err("No credentials configured for this
   channel in messagehub.toml")`. Instantiate `EmailAdapter::with_settings
   (ImapSettings { imap_host, imap_port, smtp_host, smtp_port })`. Call
   `adapter.connect(&channel_config)` — the adapter reads
   `channel_config.keychain_ref` as `user:password`, which was populated at
   init from the same TOML entry.
7. Call `adapter.send_reply("", &content).await`. (The legacy `thread_id`
   string arg is ignored when `reply_headers.is_some()`.)
8. `adapter.disconnect().await` — best-effort; logged if it errors.
9. On send success: `Store::delete_reply_draft(thread_id)`. Errors here are
   logged but swallowed — the email already left.
10. Return `Ok(())`. On send failure, return `Err(format!(...))`; the
    `reply_drafts` row stays put.

### DTO shapes

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyDraftDto {
    thread_id: String,
    in_reply_to_message_id: String,
    body: String,
    subject: Option<String>,
    updated_at: String,  // RFC 3339
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiDraftDto {
    draft_id: String,
    body: String,
    confidence: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiDraftSummaryDto {
    id: String,
    created_at: String,
    confidence: f32,
    preview: String,         // first 80 chars of output (or user_edited_output)
    has_user_edit: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudStatusDto {
    configured: bool,
    model: Option<String>,
}
```

## Frontend

### State shape

`InboxContext` reducer gains two actions: `OPEN_REPLY({ messageId, threadId })`
and `CLOSE_REPLY`. `replyFor: { messageId: Uuid, threadId: Uuid } | null` on
state. The caller (`MessageDetail`) already has both ids in hand when the
user clicks Reply, so carrying both in the action avoids a redundant
`get_message` roundtrip inside the modal. When non-null, `App.tsx` renders
`<ReplyModal messageId={...} threadId={...} />` as an overlay sibling of the
three-pane grid. Everything else about the modal (body text, AI loading
flag, redaction toggle, prior-drafts expanded) is local `useState` inside
`ReplyModal` — at most one modal is open at a time, so lifting buys nothing.

### `ReplyModal.tsx`

Renders a modal backdrop + centered dialog (CSS from scratch, no dep).
Layout: left side is the compose area (subject row, textarea, bottom action
row); right side is the AI assist panel (240px fixed width).

**Mount sequence (effect, on first render):**
1. `Promise.all([getReplyDraft(threadId), cloudConfigStatus(), listAiDrafts(messageId)])`
2. If draft exists: hydrate `body` from `draft.body`. Else: seed `body` with
   the quoted-original block, with cursor positioned at `body[0]` on mount.
3. Set `cloudConfig` and `priorDrafts` state.

**Subject row.** Read-only input showing `Re: <stripped>`, computed once from
the original message's `subject`. Locked — user cannot edit.

**Textarea.** Controlled. Monospace is fine; full-width; min-height 240 px.
`useAutosave(body, 5000, b => saveReplyDraft(...))`.

**Action row.** `Send` (primary, disabled when `body.trim() === ""` or
`sending === true`), `Discard` (deletes draft row + closes), `Cancel` (just
closes — keeps the row). `Cmd/Ctrl+Enter` triggers Send. `Esc` triggers
Cancel.

**Failure banner.** A red bar above the textarea when `sendError !== null`.
Contains the error text and an `✕` button to dismiss (does not retry; user
clicks Send again).

### `AiAssistPanel.tsx`

Props: `{ messageId: Uuid, cloudConfigured: boolean, onDraftReady: (body: string, confidence: number) => void }`.

State:
- `loading: boolean`
- `confidence: number | null`
- `redact: boolean` (default `true`)
- `priorOpen: boolean`
- `priorDrafts: AiDraftSummaryDto[]`

Render:
- Header: **AI assist** label + confidence chip (if `confidence !== null`).
- If `!cloudConfigured`: disabled state, one line: "Not configured — add `[cloud]` to messagehub.toml". No buttons.
- If `cloudConfigured`:
  - **Generate draft** button (label flips to **Regenerate** once at least one draft exists). Disabled while `loading`.
  - **Redact PII** toggle (checkbox styled as a switch). Default on.
  - **Prior drafts ▾** expandable. Shows `priorDrafts.length`. Expanding renders `<PriorDraftsDropdown />`.

On click Generate / Regenerate:
1. Set `loading = true`.
2. `const out = await aiDraftReply(messageId, redact)`
3. `onDraftReady(out.body, out.confidence)` — parent overwrites textarea.
4. `setConfidence(out.confidence); refresh prior drafts; setLoading(false)`.
5. Any thrown error → inline toast inside the panel; panel stays otherwise
   functional.

### `PriorDraftsDropdown.tsx`

Renders a flat list of rows: `{timestamp} · conf {confidence.toFixed(2)} · {preview}` with a **Restore** button per row. Restore calls `onRestore(body)` which the parent passes through to `ReplyModal`, overwriting the textarea and forcing an immediate autosave flush. `has_user_edit` rows show a subtle "(edited)" badge.

### `useAutosave.ts`

```ts
export function useAutosave<T>(
  value: T,
  delayMs: number,
  onSave: (value: T) => Promise<void>
): void
```

Debounced. Internally tracks a timeout ref and a last-saved-value ref. On
unmount, if `value !== lastSaved`, fire one final flush (fire-and-forget).
Swallows `onSave` rejections (logs to console); next keystroke retries.

## Error Handling

| # | Condition | Surface | Draft preserved? |
|---|---|---|---|
| 1 | `[cloud]` missing / `enabled = false` / required field missing | AI panel disabled with config hint | n/a |
| 2 | `CloudActions::draft_reply` errors (network / 4xx / 5xx / parse) | Inline toast in AI panel. Manual path unaffected. | Yes |
| 3 | User clicks Generate while one is in flight | Guarded by `loading` flag; second click is a no-op | Yes |
| 4 | User closes modal while AI request is in flight | Backend call completes; result discarded. Explicit YAGNI on cancellation. | n/a |
| 5 | SMTP send fails (auth, connection, transient) | Inline red banner, modal stays open | **Yes** |
| 6 | Recipient resolution fails (contact gone / no Email identity) | Banner: "No recipient address known for this contact on Email" | Yes |
| 7 | Original message absent at send time | Banner: "Original message not found" | Yes |
| 8 | Multi-account Email: two configs for same variant | First matching config wins. Documented limitation. | n/a |
| 9 | Autosave command fails (DB locked, mutex poisoned) | Log + swallow. Next keystroke retries. No UI indicator. | Kept in React state until next flush |
| 10 | User hits Send with empty body | Send button disabled while `body.trim() === ""` | n/a |
| 11 | Send succeeds, `delete_reply_draft` fails | Log, return `Ok(())`. Stale row surfaces next time the modal opens for that thread. Minor orphan, not worth surfacing. | Stale row left behind |
| 12 | `AppState.email_connections` has no entry for the channel (row in DB, no matching `[[channels]]` entry in `messagehub.toml`) | Banner: "No credentials configured for this channel in messagehub.toml" | Yes |
| 13 | Original message has no `message_id` in metadata (rare; inbound emails almost always set one, but not required by RFC) | Banner: "Cannot reply: original message has no Message-ID header" | Yes |

## Testing Strategy

No frontend test harness (consistent with 7b.1 / 7b.2). Rust tests carry the
correctness load; frontend is manual UAT.

### New test files — `core/tests/`

| File | Proves |
|---|---|
| `store_reply_drafts_test.rs` | `upsert_reply_draft` → `get_reply_draft` round-trip; second UPSERT overwrites both `body` and `in_reply_to_message_id`; `delete_reply_draft` is idempotent; `get` on unknown thread returns `None`. |
| `adapter_email_reply_test.rs` | Build `MessageContent` with `ReplyHeaders` → render via `lettre::Message::builder()` without an SMTP transport → assert the rendered bytes contain the expected `In-Reply-To: <...>`, `References: <a> <b>`, `Subject: Re: ...`, `To:`, and a generated `Message-ID:`. |

### Extensions to existing files

| File | Added case |
|---|---|
| `store_drafts_test.rs` | `list_drafts_for_message`: insert 3 `ai_drafts` rows with distinct `created_at`s, verify `DESC` order + `limit` respected. |
| `cloud_draft_test.rs` | Regenerate: calling `draft_reply` twice against the same `message_id` inserts two `ai_drafts` rows. (Extends the existing wiremock-backed test.) |
| `desktop/src-tauri/src/commands.rs` (`#[cfg(test)]` module) | DTO unit tests: `AiDraftSummaryDto::from(&AiDecision)` and `ReplyDraftDto::from(&ReplyDraft)` are pure mappers and get direct tests. Full Tauri-handler tests stay out — they need a real `State<AppState>` and are covered by manual UAT. |

No new `dev-dependencies`. `wiremock` already covers the Anthropic mock path.

### Manual UAT checklist (goes in PLAN.md, not here)

1. Open Reply on an email → modal mounts with quoted-original block below
   cursor; subject is locked to `Re: <original>`.
2. Type a few lines, wait 6 s, close modal, reopen → body is restored.
3. With `[cloud]` absent, AI panel renders disabled with the config hint.
4. With `[cloud]` present: click **Generate draft** → textarea fills,
   confidence chip renders, prior-drafts count 0 → 1.
5. Click **Regenerate** → textarea overwrites; count 1 → 2.
6. Expand **Prior drafts** → click **Restore** on the first row → textarea
   shows that body.
7. Toggle **Redact PII** off → regenerate → verify the stored
   `ai_drafts.input_redacted` row matches the un-redacted body
   (or inspect via a log grep).
8. Disconnect the network → click **Send** → red banner renders; modal
   stays; `reply_drafts` row still present.
9. Reconnect → click **Send** → email leaves via SMTP (verify in a maildev
   sink or the recipient's inbox); modal closes; `reply_drafts` row gone.
10. Open the sent reply in a separate mail client → confirm it shows as a
    reply under the original conversation, not as a new thread.

## Open Questions

None at spec time. All scope questions resolved during the brainstorm — see
the Scope Recap below.

## Scope Recap

Locked in during the 2026-04-21 brainstorm:

- **Channels:** Email only.
- **Composer UX:** Modal overlay.
- **AI interaction:** Rich side panel (generate + regenerate + confidence
  chip + redaction toggle + prior-drafts dropdown).
- **Draft lifecycle:** Autosave, 5 s idle debounce, new `reply_drafts` table.
- **Prior drafts:** Expandable dropdown with per-row Restore.
- **Email threading:** Minimal (recipient + `In-Reply-To` + `References` +
  `Re:` prefix) plus a quoted-original block. No Reply-All.
- **Missing cloud config:** panel disabled with hint.
- **SMTP failure:** inline banner, modal stays, draft preserved.

## Approach Chosen

Approach 1: synchronous send + per-thread mutable draft row (`reply_drafts`
UPSERT / DELETE) + direct `CloudActions::draft_reply` call from the Tauri
layer. Rejected alternatives: an outbox-backed async send with background
worker (doubles the runtime's work, out of scope), and an append-only draft
log with history scrubber (over-engineered for autosave UX).
