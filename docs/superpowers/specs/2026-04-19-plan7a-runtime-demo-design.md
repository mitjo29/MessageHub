# Plan 7a — `runtime-demo` Binary — Design Specification

**Date:** 2026-04-19
**Status:** Approved
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plan 6 (channel runtime) merged on master.

## Overview

Plan 7a adds a single smoke-test binary, `runtime-demo`, that runs the Plan 6 `Runtime` against real email and Telegram accounts. It exists to dogfood the pipeline end-to-end — watch real messages arrive, get classified, land in the store — before committing to a UI direction (Tauri desktop vs. UniFFI + mobile).

**What it is:** a 200-LOC binary target inside the `core` crate. Reads a TOML config, opens a SQLCipher database, registers Email and Telegram factories, starts the Runtime, prints every `RuntimeEvent` to stdout, exits cleanly on Ctrl-C.

**What it is not:** a product. No UI, no OAuth, no keychain, no daemon, no Windows support, no vault integration.

## Goals

1. `cargo run --bin runtime-demo` against a real IMAP account shows messages flowing through the full pipeline in real time.
2. Zero new runtime dependencies in the `core` library beyond `toml` (binary-only deps can be heavier).
3. Re-runs preserve DB state (`last_sync_at`, accumulated messages) so the binary behaves like a long-running process that's been restarted.
4. Credentials live in a gitignored `messagehub.toml` file; the committed `messagehub.toml.example` documents the schema.
5. Exercises the graceful-degradation path (`[ai].enabled = false`) as the default so first run works without Ollama installed.

## Non-Goals

- No new channel adapters. SMS/WhatsApp/Teams remain Plan 7d.
- No keychain integration. Credentials in plain text in a gitignored file — acceptable for local dogfooding, documented as such.
- No OAuth flows. Gmail users use app-specific passwords.
- No vault/RAG wiring. `retriever: None` on `AiPipeline::new`.
- No CLI subcommands. The binary has one job; path to config file via `--config <path>` or default `./messagehub.toml`.
- No Windows path or keychain handling. Linux/macOS only for 7a.
- No daemonization / systemd / launchd integration. Run it in a terminal, Ctrl-C to stop.

## File Layout

```
core/
├── Cargo.toml                # MODIFY: add [[bin]] target + toml dep + signal feature
├── messagehub.toml.example   # CREATE: committed schema template
└── src/
    └── bin/
        └── runtime-demo.rs   # CREATE: binary entry point, ~200 LOC

.gitignore                    # MODIFY: add messagehub.toml, *.db, *.db-journal, *.db-wal
```

## Dependencies

- **Added to `core/Cargo.toml`:**
  - `toml = "0.8"` — runtime dep. `core` library already uses `serde`.
  - `tokio` features — ensure `signal` is enabled (for `tokio::signal::ctrl_c()`).
- **Already present (library deps that the binary reuses):** `tokio`, `serde`, `tracing`, `tracing-subscriber`, `uuid`, `anyhow`/`thiserror`.

## TOML Schema

```toml
# SQLCipher database path and master key.
database = "./messagehub.db"
password = "change-me"

# Optional AI tier. Omit the whole [ai] block to run ingest-only (Plan 6
# graceful degradation).
[ai]
enabled = false
# ollama_url = "http://localhost:11434"
# model = "llama3.2"

# One [[channels]] block per connected account.
[[channels]]
kind = "email"                  # "email" or "telegram"
label = "Personal Gmail"
poll_interval_secs = 30
enabled = true
[channels.credentials]
imap_host = "imap.gmail.com"
imap_port = 993
smtp_host = "smtp.gmail.com"
smtp_port = 587
username = "me@example.com"
password = "app-specific-password"
mailbox = "INBOX"

[[channels]]
kind = "telegram"
label = "My Bot"
poll_interval_secs = 5
enabled = true
[channels.credentials]
bot_token = "123:ABC..."
```

Schema mirrored by serde structs in `runtime-demo.rs`:
- `Config { database, password, ai: Option<AiConfig>, channels: Vec<ChannelEntry> }`.
- `AiConfig { enabled: bool, ollama_url: Option<String>, model: Option<String> }`.
- `ChannelEntry { kind: String, label, poll_interval_secs, enabled, credentials: toml::Value }` — `credentials` is deserialized into a per-kind struct at factory-construction time.

## Runtime Flow

1. **Tracing.** `tracing_subscriber::fmt()` with `EnvFilter::from_default_env()` fallback to `info`. Users can crank verbosity with `RUST_LOG=messagehub_core=debug`.

2. **Load config.** Read the file at the path given by `--config <path>` or default `./messagehub.toml`. Parse via `toml::from_str`. Fail fast with a clear message if missing or malformed.

3. **Open store.** `Store::open(&config.database, &config.password)` → wrap in `Arc<std::sync::Mutex<Store>>`.

4. **Channel reconciliation.** For each TOML channel entry:
   - Derive a stable `Uuid` via `Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:{}", kind, label).as_bytes())` so the same TOML row always maps to the same DB row across restarts.
   - Lock the store briefly: if `list_channel_configs` doesn't already contain that id, call `insert_channel_config` with a fresh `ChannelConfig` (enabled + poll interval from TOML; `keychain_ref` set to `"toml:{label}"` as a placeholder; `status` + `last_error` + `consecutive_failures` default-initialized).
   - Drop the lock.

5. **Build factories.** Two concrete types in the binary file:
   - `EmailFactoryImpl { creds: HashMap<Uuid, EmailCredentials> }` — built by iterating the TOML channels, keying by each channel's derived Uuid.
   - `TelegramFactoryImpl { creds: HashMap<Uuid, TelegramCredentials> }` — same pattern.
   Each implements `AdapterFactory::build(&self, row: &ChannelConfig)` by looking up `row.id` and constructing the appropriate adapter via its existing constructor (from Plan 2).

6. **Build Runtime.**
   ```rust
   let mut builder = Runtime::builder(Arc::clone(&store))
       .with_factory("Email", Arc::new(email_factory))
       .with_factory("Telegram", Arc::new(telegram_factory));
   if let Some(ai) = &config.ai {
       if ai.enabled {
           let llm = LlmBackend::ollama(&ai.ollama_url.clone().unwrap_or_default(),
                                        &ai.model.clone().unwrap_or_default());
           let pipeline = AiPipeline::new(Arc::new(llm), None, UserProfile { content: String::new() });
           builder = builder.with_ai_pipeline(Arc::new(pipeline));
       }
   }
   let mut rt = builder.build();
   ```

7. **Subscribe + start.** `let mut events = rt.subscribe(); rt.start().await?;`

8. **Main loop.** `tokio::select!` between `tokio::signal::ctrl_c()` and a loop that awaits events and prints them. Ctrl-C breaks out.

9. **Shutdown.** `rt.shutdown().await` — consumes `rt`; bounded by the built-in shutdown timeout (30s).

## Event Formatting

One line per event, with a wall-clock timestamp and channel labels resolved via a `HashMap<Uuid, String>` built at startup:

```
[12:34:56.789 ingested]  msg=a1b2c3d4 channel="Personal Gmail"
[12:34:57.012 classified] msg=a1b2c3d4 category=Work priority=4
[12:34:58.341 sync ok]   channel="Personal Gmail" count=3
[12:35:02.891 sync fail] channel="Personal Gmail" attempt=1 error="connection refused"
[12:35:03.000 status]    channel="Personal Gmail" Healthy → Degraded{1}
```

Printed via `println!` (not `tracing::info!`) so it lands on stdout regardless of `RUST_LOG`.

## Error Handling

| Situation | Behavior |
|---|---|
| `messagehub.toml` not found | Print clear error + path to the example file, exit 1. |
| Malformed TOML | Print the `toml` parse error with line/column, exit 1. |
| Bad DB password / file corruption | `Store::open` returns Err, print, exit 1. |
| Bad credentials | `adapter.connect()` fails during `Runtime::reload_channels`; `rt.start()` returns Err, print, exit 1. |
| Network failure during operation | Surfaces as `SyncFailed` + `ChannelStatusChanged` events; binary keeps running, user sees them on stdout. |
| Missing factory for a channel kind | Runtime logs a warning and skips that row; binary keeps running for the channels it *can* serve. |
| Ctrl-C | `tokio::select!` arm fires → `rt.shutdown().await` runs → process exits 0. |

## Testing

No new automated tests. Manual verification:

1. **Smoke test with a mock TOML** (just Telegram, bogus token → expect `SyncFailed` events appearing on stdout).
2. **With a real IMAP account** (Gmail app password) → expect `MessageIngested` events for the latest messages within one poll interval.
3. **Restart** → no duplicate ingestion (cursor persisted via `last_sync_at`).
4. **Ctrl-C** → graceful shutdown, no panic, no orphaned tasks.

## Out of Scope (Future Plans)

- **7b (Tauri desktop shell)** — real UI on top of the Runtime.
- **7c (UniFFI bridge)** — foreign-language exposure.
- **7d (Additional adapters)** — SMS/WhatsApp/Teams.
- **7e (Keychain integration)** — move credentials off-disk.

---

*Spec approved 2026-04-19.*
