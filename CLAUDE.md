# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Workspace layout

Cargo workspace (edition 2021) with two members plus a React/Vite frontend:

- `core/` — `messagehub-core` library + `runtime-demo` binary
- `desktop/src-tauri/` — `messagehub-desktop` Tauri 2 shell that depends on `messagehub-core`
- `desktop/` — React 18 + TypeScript + Vite frontend (served by Tauri via `beforeDevCommand`)
- `docs/superpowers/specs/` and `docs/superpowers/plans/` — the authoritative design + plan history. **Read the relevant spec/plan before changing a subsystem**; inline doc comments frequently reference them (e.g. `see docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`).
- `docs/backlog.md` — tracked issues (`B-###`) and resolutions with commit refs.
- `core/migrations/` — numbered SQL migrations applied in order by `store::migrations::run_migrations`.

## Common commands

```bash
# Full workspace test suite
cargo test --workspace

# Single test
cargo test test_classify_happy_path --workspace

# Tests that require external resources (Ollama running, ~120MB FastEmbed model download)
cargo test --workspace -- --ignored

# Run the smoke-test binary against real accounts (needs core/messagehub.toml)
cargo run --bin runtime-demo               # config from ./messagehub.toml
cargo run --bin runtime-demo -- --config core/messagehub.toml

# Desktop (from desktop/)
npm run tauri dev     # starts Vite + Tauri shell
npm run build         # type-check (tsc --noEmit) + vite build
```

First-time config: `cp core/messagehub.toml.example core/messagehub.toml` and fill in IMAP/Telegram credentials. The file is gitignored. Both `runtime-demo` and the Tauri host resolve relative `database` paths against the TOML's parent dir (not CWD) so `./messagehub.db` in `core/messagehub.toml` always means `core/messagehub.db`.

## High-level architecture

### Core library modules (`core/src/`)

- `types/` — domain types (`Message`, `Channel`, `ChannelConfig`, `Contact`, `Thread`).
- `store/` — SQLCipher-encrypted SQLite, one `rusqlite::Connection` wrapped by `Store`. The `sqlite-vec` extension is registered via `sqlite3_auto_extension` in `ensure_sqlite_vec_loaded` so every new connection auto-loads it — do not call `load_extension`.
- `adapters/` — `ChannelAdapter` trait (`connect`/`fetch_messages`/`send_reply`/`disconnect` + optional `cursor_state`/`set_cursor_state`). Email (IMAP+SMTP), Telegram (bot polling), and a `mock` adapter used by tests.
- `ai/` — **Tier 1 local classification**. `LlmBackend` (trait) / `OllamaLlm` (HTTP client), `Classifier`, `AiPipeline`, `UserProfile` loader, `RagContext` builder, prompts + JSON parser.
- `ai/cloud/` — **Tier 2 opt-in cloud actions** against Anthropic: `summarize_thread`, `draft_reply`, `smart_search`. Includes `Redactor` (entity scrubbing with reverse-map un-redaction) and a heuristic `confidence` score. `CloudAction::as_str` values are persisted in `ai_drafts.action_type` / `action_log.action_type` — keep them stable.
- `knowledge/` — markdown-vault indexer, frontmatter/section parser, FastEmbed (384-dim) embedder, sqlite-vec retrieval, file watcher, people extraction.
- `runtime/` — orchestration layer (see below).

### Runtime orchestration (`core/src/runtime/`)

`RuntimeBuilder` → `Runtime::start()` spawns:

1. An **ingestor** task (bounded mpsc, persists messages and forwards to the classifier).
2. An optional **classifier worker** (spawned only if `with_ai_pipeline` was called).
3. One **channel task** per enabled row in the `channels` table, built via a registered `AdapterFactory`.

Coordination primitives: a `CancellationToken` tree rooted at `Runtime`, a `broadcast::Sender<RuntimeEvent>` event bus for UI/observability subscribers, and bounded `mpsc` channels for backpressure between stages. `reload_channels()` reconciles the DB with running tasks (stop removed/disabled, start new enabled). `shutdown()` cancels the root token, joins channel tasks, drops the ingest sender, then joins ingestor + classifier, each bounded by `shutdown_timeout` (default 30s).

**Lock discipline (critical):** the `Arc<Mutex<Store>>` is held only across brief synchronous rusqlite calls. Never hold the guard across `.await`. See `Runtime::reload_channels` for the pattern (collect rows under lock, drop guard, then do async work).

### Ingest idempotency (Plan 7b.2.1)

Schema-level: `messages.external_id` with a unique partial index on `(channel_type, external_id) WHERE external_id IS NOT NULL`. `insert_message` uses `INSERT ... ON CONFLICT DO NOTHING` (first-write-wins). Adapters that need a polling cursor implement `cursor_state` / `set_cursor_state` (currently Telegram's `last_update_id`); the channel task hydrates from and writes back to `channels.last_sync_cursor`. When adding a new adapter, decide whether dedup via `external_id` alone is sufficient or whether a cursor is also needed.

### Desktop shell (`desktop/src-tauri/src/`)

`main.rs` loads `messagehub.toml`, opens the encrypted store, constructs `AppState`, and registers Tauri commands (`list_messages`, `get_message`, `list_channels`, `get_config`, `mark_read`, `sidebar_counts`). If init fails, a window still opens but commands return "state not managed" so the frontend's error banner surfaces the cause. Config lookup order: `./`, `../core/`, `../../core/`, `core/`.

## Conventions worth preserving

- Message IDs in `runtime-demo` are stable UUIDv5 hashes of `(kind, label)`. Re-running against the same TOML reuses the same `channels` row.
- `Category` enum values are the only legal classifier outputs; `parse_classification_response` rejects anything else.
- Ignored tests fall into two buckets: "needs Ollama running" and "downloads a ~120MB embedding model." Gate new tests in the same way rather than making them unconditional.
- When adding to the design, write a spec in `docs/superpowers/specs/` and a plan in `docs/superpowers/plans/` following the dated `NNNN-NN-NN-planX-<name>.md` naming. Link the spec from the module-level doc comment of whatever you build.
