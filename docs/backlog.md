# Backlog

Issues and improvements queued for future plans. Each entry has a severity,
discovered-during context, and a proposed fix. When work begins, promote the
item to a proper plan under `docs/superpowers/plans/`.

*(Empty — no open items at the moment.)*

---

## Resolved

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
