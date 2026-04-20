# Backlog

Issues and improvements queued for future plans. Each entry has a severity,
discovered-during context, and a proposed fix. When work begins, promote the
item to a proper plan under `docs/superpowers/plans/`.

### B-003 — Ingest has no idempotency; every poll re-inserts all messages

Discovered during 7b.2 manual verification on 2026-04-20. Two cascading
bugs, both inherited from Plan 6:

1. **Telegram adapter never advances its cursor.** `last_update_id` is
   declared on the adapter instance but never written back from the
   channel task. `adapters/telegram.rs:147-148` literally says "should be
   updated by the caller (Runtime channel task)" — and it isn't. Every
   poll returns every message the bot has ever received.
2. **Ingestor doesn't dedup.** `runtime/ingestor.rs::ingest_one` calls
   `insert_message` with a freshly generated UUID every time. The
   `messages` table (migrations/001_initial.sql) has no `external_id`
   column, so there's nothing to dedup against.

**Symptom:** After ~30 minutes of `runtime-demo` running, the DB had
~thousands of duplicate rows of the same ~5 Telegram/email messages.

**Proposed fix (own phase, ~1-2h):**
- New migration adding `messages.external_id TEXT NULL` + unique index
  on `(channel_type, external_id) WHERE external_id IS NOT NULL`.
- `insert_message` uses `INSERT ... ON CONFLICT DO NOTHING`, or the
  ingestor looks up first and skips on hit.
- Fix telegram cursor: persist `last_update_id` via `update_sync_state`
  or a new per-adapter state column.
- Confirm email's `since` watermark round-trip is tight enough not to
  re-fetch on restart (may be fine; dedup covers ties regardless).

Blocks 7b.2 manual checks #11 (15s poll) and #12 (focus refresh) — both
would show spurious unread-count climbs even without new messages.

---

## Resolved

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
