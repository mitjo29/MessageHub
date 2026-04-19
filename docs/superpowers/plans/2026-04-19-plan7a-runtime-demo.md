# Plan 7a: `runtime-demo` Binary — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a single `runtime-demo` binary that runs the Plan 6 `Runtime` against real email + Telegram accounts. TOML-configured, SQLCipher-persisted, stdout-event-printing, Ctrl-C-shutdown. No UI, no keychain, no OAuth — strictly a dogfooding tool.

**Architecture:** One file at `core/src/bin/runtime-demo.rs` (~250 LOC). Serde structs mirror the TOML schema. Two concrete `AdapterFactory` implementations (`EmailFactoryImpl`, `TelegramFactoryImpl`) live in the binary and hold per-channel credentials. Existing `EmailAdapter::with_settings` + `TelegramAdapter::new` constructors are used as-is. The `ChannelConfig.keychain_ref` field is populated with either `"user:password"` (email) or the raw bot token (telegram) — this matches what the existing `connect()` implementations already read.

**Tech Stack:** `toml = "0.8"` (NEW dep), `tokio` with `signal` feature (check if already enabled; enable if not), `tracing-subscriber` (already present), `serde` (already present). No new library dependencies.

**Prerequisites:** Plan 6 merged to master (commit `0c55b89`). Tests green.

**Spec:** `docs/superpowers/specs/2026-04-19-plan7a-runtime-demo-design.md`.

---

## File Structure

```
core/
├── Cargo.toml                          # MODIFY — add [[bin]], `toml = "0.8"`, tokio `signal` feature
├── messagehub.toml.example             # CREATE — committed schema template
└── src/
    └── bin/
        └── runtime-demo.rs             # CREATE — binary entry point

.gitignore                              # MODIFY — add messagehub.toml, *.db, *.db-journal, *.db-wal
```

---

### Task 1: Cargo manifest + `.gitignore` + committed TOML example

**Files:**
- Modify: `core/Cargo.toml`
- Create: `core/messagehub.toml.example`
- Modify: `.gitignore`

`★ Why this matters:` Before any Rust code, set up the workspace so the binary target compiles from an empty stub and so `messagehub.toml` with secrets can be created locally without being staged accidentally.

- [ ] **Step 1: Check current `tokio` features**

Run: `grep -A2 '^tokio' core/Cargo.toml`

Inspect which features are enabled. We need `signal`. If absent, add it in Step 2. If present, leave alone.

- [ ] **Step 2: Modify `core/Cargo.toml`**

Add `toml = "0.8"` under `[dependencies]` (alphabetical order). Ensure `tokio` includes the `signal` feature (commonly by extending `features = [...]`). Then append the binary target at the end of the file:

```toml
[[bin]]
name = "runtime-demo"
path = "src/bin/runtime-demo.rs"
```

Run: `cargo check -p messagehub-core --bin runtime-demo` — will fail because the binary file doesn't exist yet. That's expected; skip to Step 3.

- [ ] **Step 3: Create `core/messagehub.toml.example`**

```toml
# Example configuration for runtime-demo.
# Copy this file to `messagehub.toml` (same directory) and fill in real values.
# `messagehub.toml` is gitignored; this `.example` is committed.

# SQLCipher database path (created on first run) and master key.
database = "./messagehub.db"
password = "change-me-to-something-unique"

# Optional AI tier. Delete or set `enabled = false` to run ingest-only.
[ai]
enabled = false
# ollama_url = "http://localhost:11434"
# model = "llama3.2"

# One [[channels]] block per account. `kind` must be "email" or "telegram".
# Delete or add blocks freely.

[[channels]]
kind = "email"
label = "Personal Gmail"
poll_interval_secs = 30
enabled = true
[channels.credentials]
imap_host = "imap.gmail.com"
imap_port = 993
smtp_host = "smtp.gmail.com"
smtp_port = 587
username = "me@example.com"
# For Gmail: generate an app-specific password at myaccount.google.com/apppasswords.
password = "xxxxxxxxxxxxxxxx"
mailbox = "INBOX"

[[channels]]
kind = "telegram"
label = "My Bot"
poll_interval_secs = 5
enabled = true
[channels.credentials]
# Create a bot at @BotFather, paste the token here.
bot_token = "123456:ABC-DEF..."
```

- [ ] **Step 4: Update `.gitignore`**

Read the current `.gitignore`. Append these lines at the end:

```
# Plan 7a: runtime-demo local config and databases.
core/messagehub.toml
core/messagehub.db
core/messagehub.db-journal
core/messagehub.db-wal
messagehub.toml
messagehub.db
messagehub.db-journal
messagehub.db-wal
```

(Cover both the case where `runtime-demo` is run from `core/` and from the workspace root.)

- [ ] **Step 5: Commit**

```bash
git add core/Cargo.toml core/messagehub.toml.example .gitignore
git commit -m "feat(demo): scaffold runtime-demo Cargo target and config template"
```

---

### Task 2: Binary skeleton — stub `main` that compiles and exits

**Files:**
- Create: `core/src/bin/runtime-demo.rs`

`★ Why this matters:` Ship a compilable, runnable entry point first. Later tasks fill in the real logic while the binary keeps compiling. Use `tracing_subscriber` from the start so all subsequent dev can rely on `RUST_LOG`.

- [ ] **Step 1: Create the file with a minimal `main`**

```rust
//! runtime-demo — smoke-test harness for the Plan 6 Runtime.
//!
//! Reads `messagehub.toml`, opens a SQLCipher store, runs the Runtime
//! against real email + Telegram accounts, prints events to stdout,
//! exits cleanly on Ctrl-C. See
//! `docs/superpowers/specs/2026-04-19-plan7a-runtime-demo-design.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    init_tracing();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("runtime-demo: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_target(false).init();
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Subsequent tasks replace this body with the full Runtime flow.
    eprintln!("runtime-demo: scaffold — real flow lands in Task 3+");
    Ok(())
}
```

- [ ] **Step 2: Build it**

Run: `cargo build -p messagehub-core --bin runtime-demo`
Expected: clean build.

Run: `cargo run -p messagehub-core --bin runtime-demo`
Expected: stderr says "runtime-demo: scaffold — real flow lands in Task 3+", exit 0.

- [ ] **Step 3: Commit**

```bash
git add core/src/bin/runtime-demo.rs
git commit -m "feat(demo): add runtime-demo binary skeleton with tracing"
```

---

### Task 3: Config loader — serde structs + TOML parsing

**Files:**
- Modify: `core/src/bin/runtime-demo.rs`

`★ Why this matters:` All subsequent tasks depend on a validated config object. Keep the module single-file — no `mod.rs`, no separate config file.

- [ ] **Step 1: Add config structs above `fn main()`**

Insert these structs (between `init_tracing` and `fn run`):

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    database: String,
    password: String,
    #[serde(default)]
    ai: Option<AiConfig>,
    #[serde(default)]
    channels: Vec<ChannelEntry>,
}

#[derive(Debug, Deserialize)]
struct AiConfig {
    enabled: bool,
    ollama_url: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChannelEntry {
    kind: String,            // "email" | "telegram"
    label: String,
    poll_interval_secs: u32,
    enabled: bool,
    credentials: toml::Value, // kind-specific; deserialized lazily below
}

#[derive(Debug, Deserialize)]
struct EmailCredentials {
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
    username: String,
    password: String,
    #[serde(default = "default_mailbox")]
    mailbox: String,
}
fn default_mailbox() -> String { "INBOX".to_string() }

#[derive(Debug, Deserialize)]
struct TelegramCredentials {
    bot_token: String,
}
```

- [ ] **Step 2: Write a loader + CLI arg parser**

Add below the structs:

```rust
use std::path::PathBuf;

fn default_config_path() -> PathBuf { PathBuf::from("messagehub.toml") }

fn load_config(path: &std::path::Path) -> Result<Config, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config '{}': {}", path.display(), e))?;
    let config: Config = toml::from_str(&text)
        .map_err(|e| format!("failed to parse '{}': {}", path.display(), e))?;
    Ok(config)
}

fn parse_config_path(args: &[String]) -> PathBuf {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            if let Some(path) = iter.next() { return PathBuf::from(path); }
        }
    }
    default_config_path()
}
```

- [ ] **Step 3: Wire into `run()`**

Replace `run()` with:

```rust
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = parse_config_path(&args);
    let config = load_config(&config_path)?;
    eprintln!(
        "runtime-demo: loaded {} channel(s) from {}",
        config.channels.len(),
        config_path.display(),
    );
    Ok(())
}
```

- [ ] **Step 4: Manual smoke test**

Copy the example and try it:

```bash
cd core
cp messagehub.toml.example messagehub.toml
cargo run --bin runtime-demo
```

Expected: prints "runtime-demo: loaded 2 channel(s) from messagehub.toml" and exits 0.

Then test the error path:

```bash
cargo run --bin runtime-demo -- --config /nonexistent.toml
```

Expected: exits 1 with a readable error message containing the path.

Delete the local `messagehub.toml` before committing:

```bash
rm core/messagehub.toml
```

- [ ] **Step 5: Commit**

```bash
git add core/src/bin/runtime-demo.rs
git commit -m "feat(demo): parse messagehub.toml with serde"
```

---

### Task 4: Store open + channel reconciliation

**Files:**
- Modify: `core/src/bin/runtime-demo.rs`

`★ Why this matters:` Turning the config into rows in the `channels` table. Stable UUIDs ensure re-runs reuse the same row so `last_sync_at` is preserved.

- [ ] **Step 1: Add imports + a stable-UUID helper**

Near the top of the file (after the existing `use`s):

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
```

Below `load_config`, add:

```rust
fn stable_channel_id(kind: &str, label: &str) -> Uuid {
    // UUID v5 with NAMESPACE_OID gives a deterministic id from (kind, label).
    // Re-runs of runtime-demo always map the same TOML entry to the same DB row.
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:{}", kind, label).as_bytes())
}

fn channel_kind_from_toml(kind: &str) -> Result<Channel, Box<dyn std::error::Error>> {
    match kind {
        "email"    => Ok(Channel::Email),
        "telegram" => Ok(Channel::Telegram),
        other => Err(format!("unsupported channel kind '{}'", other).into()),
    }
}

fn credential_keychain_ref(entry: &ChannelEntry) -> Result<String, Box<dyn std::error::Error>> {
    match entry.kind.as_str() {
        "email" => {
            let c: EmailCredentials = entry.credentials.clone().try_into()
                .map_err(|e| format!("email channel '{}': bad credentials — {}", entry.label, e))?;
            // EmailAdapter::connect() reads config.keychain_ref as "user:password".
            Ok(format!("{}:{}", c.username, c.password))
        }
        "telegram" => {
            let c: TelegramCredentials = entry.credentials.clone().try_into()
                .map_err(|e| format!("telegram channel '{}': bad credentials — {}", entry.label, e))?;
            // TelegramAdapter::connect() reads config.keychain_ref as the raw bot token.
            Ok(c.bot_token)
        }
        other => Err(format!("unsupported channel kind '{}'", other).into()),
    }
}
```

- [ ] **Step 2: Add the reconciliation function**

```rust
fn reconcile_channels(
    store: &Mutex<Store>,
    entries: &[ChannelEntry],
) -> Result<HashMap<Uuid, String>, Box<dyn std::error::Error>> {
    let guard = store.lock().expect("runtime-demo: store mutex poisoned");
    let existing: Vec<ChannelConfig> = guard.list_channel_configs()?;
    let existing_ids: std::collections::HashSet<Uuid> =
        existing.iter().map(|c| c.id).collect();

    let mut labels = HashMap::new();
    for entry in entries {
        let id = stable_channel_id(&entry.kind, &entry.label);
        labels.insert(id, entry.label.clone());
        if existing_ids.contains(&id) { continue; }

        let channel = channel_kind_from_toml(&entry.kind)?;
        let keychain_ref = credential_keychain_ref(entry)?;
        let cfg = ChannelConfig {
            id,
            channel,
            label: entry.label.clone(),
            keychain_ref,
            enabled: entry.enabled,
            poll_interval_secs: entry.poll_interval_secs,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        };
        guard.insert_channel_config(&cfg)?;
    }
    Ok(labels)
}
```

- [ ] **Step 3: Wire it into `run()`**

Replace the body of `run()` with:

```rust
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = parse_config_path(&args);
    let config = load_config(&config_path)?;

    let store = Arc::new(Mutex::new(Store::open(
        std::path::Path::new(&config.database),
        &config.password,
    )?));

    let labels = reconcile_channels(&store, &config.channels)?;
    eprintln!("runtime-demo: {} channel(s) reconciled into store", labels.len());
    Ok(())
}
```

- [ ] **Step 4: Smoke test**

```bash
cd core
cp messagehub.toml.example messagehub.toml
cargo run --bin runtime-demo
```

Expected output: "runtime-demo: 2 channel(s) reconciled into store". Confirm `core/messagehub.db` now exists:

```bash
ls -la core/messagehub.db
```

Run it again:

```bash
cargo run --bin runtime-demo
```

Expected: same output. The DB already contains the two rows; no duplicates inserted.

Clean up before committing:

```bash
rm core/messagehub.toml core/messagehub.db core/messagehub.db-journal 2>/dev/null
```

- [ ] **Step 5: Commit**

```bash
git add core/src/bin/runtime-demo.rs
git commit -m "feat(demo): open SQLCipher store and reconcile channel rows from TOML"
```

---

### Task 5: Factory implementations

**Files:**
- Modify: `core/src/bin/runtime-demo.rs`

`★ Why this matters:` The `Runtime` dispatches to factories by `channel_type` string. Each factory needs per-channel credentials (IMAP host, port, etc. for email; nothing extra for telegram since bot_token is already in `keychain_ref`).

- [ ] **Step 1: Add email factory**

Append these below the reconciliation code, before `fn run`:

```rust
use async_trait::async_trait;
use messagehub_core::adapters::{
    email::{EmailAdapter, ImapSettings},
    telegram::TelegramAdapter,
    ChannelAdapter,
};
use messagehub_core::error::Result as CoreResult;
use messagehub_core::runtime::factory::AdapterFactory;

#[derive(Debug, Clone)]
struct EmailConnection {
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
}

struct EmailFactoryImpl { creds: HashMap<Uuid, EmailConnection> }

#[async_trait]
impl AdapterFactory for EmailFactoryImpl {
    async fn build(&self, row: &ChannelConfig) -> CoreResult<Box<dyn ChannelAdapter>> {
        let c = self.creds.get(&row.id).ok_or_else(|| {
            messagehub_core::error::CoreError::InvalidInput(format!(
                "runtime-demo: no email credentials for channel '{}'", row.label,
            ))
        })?;
        let adapter = EmailAdapter::with_settings(ImapSettings {
            host: c.imap_host.clone(),
            port: c.imap_port,
            smtp_host: c.smtp_host.clone(),
            smtp_port: c.smtp_port,
        });
        Ok(Box::new(adapter))
    }
}
```

- [ ] **Step 2: Add telegram factory**

```rust
struct TelegramFactoryImpl;

#[async_trait]
impl AdapterFactory for TelegramFactoryImpl {
    async fn build(&self, _row: &ChannelConfig) -> CoreResult<Box<dyn ChannelAdapter>> {
        // TelegramAdapter reads the bot token from row.keychain_ref during connect().
        Ok(Box::new(TelegramAdapter::new()))
    }
}
```

- [ ] **Step 3: Helper that builds factory instances from the TOML**

```rust
fn build_factories(
    entries: &[ChannelEntry],
) -> Result<(Arc<EmailFactoryImpl>, Arc<TelegramFactoryImpl>), Box<dyn std::error::Error>> {
    let mut email_creds = HashMap::new();
    for entry in entries.iter().filter(|e| e.kind == "email") {
        let id = stable_channel_id(&entry.kind, &entry.label);
        let c: EmailCredentials = entry.credentials.clone().try_into()
            .map_err(|e| format!("email channel '{}': {}", entry.label, e))?;
        email_creds.insert(id, EmailConnection {
            imap_host: c.imap_host,
            imap_port: c.imap_port,
            smtp_host: c.smtp_host,
            smtp_port: c.smtp_port,
        });
    }
    Ok((
        Arc::new(EmailFactoryImpl { creds: email_creds }),
        Arc::new(TelegramFactoryImpl),
    ))
}
```

- [ ] **Step 4: Build factories in `run()` (without wiring to Runtime yet)**

Update `run()` to call `build_factories` and log the counts:

```rust
let (email_factory, telegram_factory) = build_factories(&config.channels)?;
let email_count = email_factory.creds.len();
let telegram_count = config.channels.iter().filter(|c| c.kind == "telegram").count();
eprintln!(
    "runtime-demo: factories built — {} email, {} telegram",
    email_count, telegram_count,
);
// labels is still unused in this task; let the compiler warn.
let _ = labels;
```

- [ ] **Step 5: Build and run**

```bash
cd core
cp messagehub.toml.example messagehub.toml
cargo run --bin runtime-demo
```

Expected: "runtime-demo: factories built — 1 email, 1 telegram".

```bash
rm core/messagehub.toml core/messagehub.db core/messagehub.db-journal 2>/dev/null
```

- [ ] **Step 6: Commit**

```bash
git add core/src/bin/runtime-demo.rs
git commit -m "feat(demo): add EmailFactoryImpl + TelegramFactoryImpl backed by TOML creds"
```

---

### Task 6: Runtime wiring + event loop + Ctrl-C shutdown

**Files:**
- Modify: `core/src/bin/runtime-demo.rs`

`★ Why this matters:` This is the task that actually runs the pipeline. After it lands, `cargo run --bin runtime-demo` against a real account prints real events.

- [ ] **Step 1: Switch `main` to a tokio runtime**

Replace the top of the file. Change `main` to:

```rust
fn main() -> ExitCode {
    init_tracing();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime-demo: failed to start tokio runtime: {}", e);
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("runtime-demo: {}", e); ExitCode::FAILURE }
    }
}
```

And change `run` to `async fn run() -> Result<(), Box<dyn std::error::Error>>`.

- [ ] **Step 2: Add the event-printing helper**

Add below the factory types, above `fn run`:

```rust
use chrono::Local;
use messagehub_core::runtime::events::RuntimeEvent;
use tokio::sync::broadcast::Receiver;

async fn print_events(mut rx: Receiver<RuntimeEvent>, labels: HashMap<Uuid, String>) {
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let now = Local::now().format("%H:%M:%S%.3f");
                let label_of = |id: &Uuid| labels.get(id).cloned()
                    .unwrap_or_else(|| id.to_string());
                match ev {
                    RuntimeEvent::MessageIngested { id, channel_id } => {
                        println!("[{} ingested]  msg={} channel=\"{}\"",
                                 now, short(&id), label_of(&channel_id));
                    }
                    RuntimeEvent::MessageClassified { id, category, priority } => {
                        println!("[{} classified] msg={} category={} priority={}",
                                 now,
                                 short(&id),
                                 category.unwrap_or_else(|| "?".into()),
                                 priority.map(|p| p.value().to_string())
                                         .unwrap_or_else(|| "?".into()));
                    }
                    RuntimeEvent::SyncSucceeded { channel_id, count } => {
                        println!("[{} sync ok]   channel=\"{}\" count={}",
                                 now, label_of(&channel_id), count);
                    }
                    RuntimeEvent::SyncFailed { channel_id, error, attempt } => {
                        println!("[{} sync fail] channel=\"{}\" attempt={} error=\"{}\"",
                                 now, label_of(&channel_id), attempt, error);
                    }
                    RuntimeEvent::ChannelStatusChanged { channel_id, status } => {
                        println!("[{} status]    channel=\"{}\" {:?}",
                                 now, label_of(&channel_id), status);
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("runtime-demo: event stream lagged, dropped {} events", n);
            }
        }
    }
}

fn short(id: &Uuid) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}
```

- [ ] **Step 3: Wire Runtime into `run()`**

Replace the `run()` body with:

```rust
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    use messagehub_core::ai::{llm::LlmBackend, pipeline::AiPipeline, profile::UserProfile};
    use messagehub_core::runtime::Runtime;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = parse_config_path(&args);
    let config = load_config(&config_path)?;

    let store = Arc::new(Mutex::new(Store::open(
        std::path::Path::new(&config.database),
        &config.password,
    )?));

    let labels = reconcile_channels(&store, &config.channels)?;
    let (email_factory, telegram_factory) = build_factories(&config.channels)?;

    let mut builder = Runtime::builder(Arc::clone(&store))
        .with_factory("Email", email_factory)
        .with_factory("Telegram", telegram_factory);

    if let Some(ai) = &config.ai {
        if ai.enabled {
            let url   = ai.ollama_url.clone().unwrap_or_else(|| "http://localhost:11434".into());
            let model = ai.model     .clone().unwrap_or_else(|| "llama3.2".into());
            let llm = LlmBackend::ollama(&url, &model);
            let pipeline = AiPipeline::new(
                Arc::new(llm),
                None, // no retriever in Plan 7a — see spec §Non-Goals
                UserProfile { content: String::new() },
            );
            builder = builder.with_ai_pipeline(Arc::new(pipeline));
            eprintln!("runtime-demo: AI tier enabled (ollama at {}, model {})", url, model);
        } else {
            eprintln!("runtime-demo: AI tier disabled (ingest-only)");
        }
    } else {
        eprintln!("runtime-demo: no [ai] section in config — ingest-only");
    }

    let mut rt = builder.build();
    let events = rt.subscribe();
    rt.start().await?;
    eprintln!("runtime-demo: runtime started. Ctrl-C to stop.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nruntime-demo: Ctrl-C received, shutting down...");
        }
        _ = print_events(events, labels) => {
            // print_events only returns when the broadcast is closed, which
            // happens during shutdown. Nothing to do here.
        }
    }

    rt.shutdown().await;
    eprintln!("runtime-demo: shutdown complete");
    Ok(())
}
```

Verify the imports above are all pulled in. You may need to adjust the `LlmBackend::ollama` call to match the actual constructor signature — inspect `core/src/ai/llm.rs` if it doesn't compile as written.

- [ ] **Step 4: Build**

```bash
cargo build -p messagehub-core --bin runtime-demo
```

Expected: clean build.

- [ ] **Step 5: Dry-run smoke test with bogus credentials**

Create a TOML with deliberately bad credentials to exercise the failure path:

```bash
cd core
cat > messagehub.toml <<'EOF'
database = "./messagehub.db"
password = "demo-test"

[[channels]]
kind = "telegram"
label = "Broken Bot"
poll_interval_secs = 5
enabled = true
[channels.credentials]
bot_token = "111:NOPE"
EOF

cargo run --bin runtime-demo
```

Expected: binary starts, prints "runtime started", then either fails at connect (bad token) with a clear error and exits 1, OR if connect is lenient it reaches the polling loop and prints `[… sync fail] channel="Broken Bot" attempt=1 error=...` within a few seconds. Press Ctrl-C — should see "shutting down..." then "shutdown complete" and exit 0.

Clean up:

```bash
rm core/messagehub.toml core/messagehub.db core/messagehub.db-journal 2>/dev/null
```

- [ ] **Step 6: Commit**

```bash
git add core/src/bin/runtime-demo.rs
git commit -m "feat(demo): wire Runtime with event-printing loop and Ctrl-C shutdown"
```

---

### Task 7: Manual verification checklist

**Files:** (none — verification step)

`★ Why this matters:` Plan 7a exists specifically to dogfood. The test here is "did you actually use it?"

- [ ] **Verification 1: Build on the feature branch**

```bash
cargo build -p messagehub-core --bin runtime-demo
cargo test  -p messagehub-core
```

Expected: clean build, all 155 Plan 6 tests still pass.

- [ ] **Verification 2: Bogus config surfaces a readable error**

```bash
cd core
cargo run --bin runtime-demo -- --config /tmp/definitely-not-there.toml
```

Expected: exit 1, clean error message naming the path.

- [ ] **Verification 3: Run against at least one real account**

Copy the example, fill in real credentials for ONE channel (either email or telegram), run `cargo run --bin runtime-demo`. Expected:
- `runtime started. Ctrl-C to stop.`
- Within `poll_interval_secs` seconds, at least one of: `sync ok`, `sync fail`, or `ingested` lines on stdout.
- Ctrl-C exits cleanly.
- Re-running shows `last_sync_at` is preserved (no duplicate ingestion).

Record any issues discovered during dogfooding as new GitHub issues / plan items — **these do NOT go into Plan 7a.** 7a is done when the binary runs.

- [ ] **Verification 4: No secrets in the commit**

```bash
git log -p feat/runtime-demo ^master -- core/messagehub.toml
```

Expected: no output. `messagehub.toml` should never land in git.

---

## Notes for the executor

- **Branch:** create `feat/runtime-demo` off master before starting. Merge to master at the end (pattern matches Plan 6).
- **Do NOT add new runtime-demo-specific tests** to the library test suite. The binary's "test" is the manual verification checklist.
- **Do NOT move factory code into the `core` library** — it belongs in the binary. Moving it to `core` is scope creep for Plan 7c (UniFFI) when the API surface will be reconsidered.
- **If EmailAdapter's `with_settings` or `connect` behavior changed since this plan was written**, fix the factory in the binary, not the adapter. Adapter behavior is load-bearing for the existing library tests.
- **If `LlmBackend::ollama` has a different signature than `(url, model)`**, adapt the single call site.
- **If a channel kind in TOML doesn't match a registered factory**, the Runtime already logs a warning and skips that row — the binary surfaces this via the existing `tracing` output at `info` level. No extra handling needed.
