# Plan 6 — Channel Runtime — Design Specification

**Date:** 2026-04-19
**Status:** Approved
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plan 1 (core storage), Plan 2 (channel adapters), Plan 3 (knowledge engine), Plan 4 (local AI classification).

## Overview

MessageHub today is a library of parts: adapters that can fetch, a store that can persist, an `AiPipeline` that can classify, a `CloudActions` facade that can draft. Nothing runs. Plan 6 turns `core` into a self-driving pipeline by adding a `Runtime` that polls adapters on their configured intervals, ingests messages into the store, dispatches each new message to the local classifier, publishes events, and tracks per-channel health with exponential backoff.

The existing `core/src/adapters/manager.rs` (a Plan-2 proof-of-concept with an `Fn(Vec<RawMessage>)` callback, no persistence, no classification, no backoff) is deleted. Its intent — per-config adapter registration and a background sync loop — reappears inside `Runtime` in cleaner form.

**Out of scope for Plan 6:** new channel adapters (SMS, WhatsApp, Teams), UniFFI export, auto-draft on ingest, knowledge-engine file-watcher coupling.

## Goals

1. A single `Runtime` that owns the full `adapter → store → classifier` pipeline.
2. DB-driven channel configuration via the existing `channels` table, using adapter factories so `core` stays adapter-agnostic.
3. An event stream (`tokio::sync::broadcast`) that lets consumers react to ingestion, classification, and sync state changes without polling the DB.
4. Isolated failure: one failing adapter does not affect others; a stuck LLM does not stall polling.
5. Graceful degradation: `Runtime` works with or without `AiPipeline`; classifier errors do not hide messages from the user.
6. Graceful shutdown: cancellation propagates to all tasks; adapters disconnect cleanly; pending work drains within a bounded timeout.

## Non-Goals

- No new channel adapters. Existing `EmailAdapter` and `TelegramAdapter` are the test surface.
- No changes to the `ChannelAdapter` trait. Cursor semantics remain timestamp-based (`since: Option<DateTime<Utc>>`).
- No UniFFI bindings for `Runtime`. The event stream's FFI shape is deferred until there is a consumer driving the requirements.
- No automatic cloud actions. Tier 2 stays strictly opt-in per the product spec.
- No file-watcher-driven vault reindex. The knowledge engine remains standalone in Plan 6.

## Architecture

### Module Layout

```
core/src/runtime/
├── mod.rs                  # Runtime + RuntimeBuilder + subscribe/start/shutdown/reload
├── factory.rs              # AdapterFactory trait + factory registry
├── channel_task.rs         # Per-channel poll loop (one tokio task per enabled channel)
├── ingestor.rs             # RawMessage → contact/thread resolution → Message insert
├── classifier_worker.rs    # Drains mpsc, runs AiPipeline::process, updates row
├── events.rs               # RuntimeEvent enum + broadcast channel wrapper
└── status.rs               # ChannelStatus state machine + backoff math
```

Shutdown cancellation uses `tokio_util::sync::CancellationToken` directly — the root lives on `Runtime`, every spawned task receives a `child_token()`. No dedicated module; wiring is a few lines per task.

### Task Topology

```
  ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
  │  ChannelTask A  │     │  ChannelTask B  │ ... │  ChannelTask N  │
  │  (one per enabl.│     │                 │     │                 │
  │   channel row)  │     │                 │     │                 │
  └────────┬────────┘     └────────┬────────┘     └────────┬────────┘
           │ mpsc<IngestJob>       │                       │
           └───────────────────────┴───────────────────────┘
                                   │
                                   ▼
                          ┌─────────────────┐
                          │    Ingestor     │     (single task)
                          │  contact + thread│
                          │  + insert + emit│
                          └────────┬────────┘
                                   │ mpsc<Uuid>
                                   ▼
                          ┌─────────────────┐
                          │ClassifierWorker │     (single task, skipped if no AiPipeline)
                          │  AiPipeline     │
                          │  + update + emit│
                          └─────────────────┘

     broadcast<RuntimeEvent>   ◀─── events flow from all components
```

Exactly one ingestor (serializes contact identity-merging naturally). Exactly one classifier worker (one at a time respects the local LLM's single-inference-at-a-time cost model). One channel task per enabled row in the `channels` table.

## Public API

```rust
// Construction
let rt = Runtime::builder(store.clone())
    .with_ai_pipeline(pipeline)               // optional — graceful degradation
    .with_factory("email",    Arc::new(EmailFactory::new(keychain.clone())))
    .with_factory("telegram", Arc::new(TelegramFactory::new(keychain.clone())))
    .event_buffer(1024)                        // default 1024
    .classifier_queue(256)                     // default 256
    .shutdown_timeout(Duration::from_secs(30)) // default 30s
    .build();

// Lifecycle
rt.start().await?;                             // reads `channels` table, spawns tasks for enabled rows
let mut events = rt.subscribe();               // tokio::sync::broadcast::Receiver<RuntimeEvent>
rt.reload_channels().await?;                   // re-read DB (add/remove/enable/disable at runtime)
rt.shutdown().await;                           // cancel → drain → disconnect → join (bounded)
```

### `AdapterFactory` trait (new)

```rust
#[async_trait]
pub trait AdapterFactory: Send + Sync {
    /// Build a fresh, unconnected adapter from a persisted channel row.
    /// Credential resolution (keychain lookup, OAuth refresh) happens here.
    /// The Runtime calls `connect()` after build.
    async fn build(&self, row: &ChannelRow) -> Result<Box<dyn ChannelAdapter>>;
}
```

`ChannelRow` is the existing DB row shape for `channels` (via a new `store::channels::list_channels` helper if one doesn't yet exist). Registering a factory keyed by `channel_type` string lets `Runtime` instantiate adapters for any channel kind the binary has linked in, without `core` holding a `match` on concrete types.

## Components

### `Runtime`

Holds:
- `store: Arc<Store>` (or whichever top-level store handle exists)
- `factories: HashMap<String, Arc<dyn AdapterFactory>>`
- `ai_pipeline: Option<Arc<AiPipeline>>`
- `event_tx: broadcast::Sender<RuntimeEvent>`
- `channel_tasks: HashMap<Uuid, ChannelTaskHandle>` (one per running channel)
- `ingestor_handle: Option<JoinHandle<()>>`
- `classifier_handle: Option<JoinHandle<()>>`
- `shutdown: CancellationToken` (root token; every task gets a child)
- Config: event buffer size, classifier queue capacity, shutdown timeout.

`start()`:
1. Create the root `CancellationToken`.
2. Create the ingestor `mpsc` (bounded; default capacity = max(16, 2 × registered channels)) and the classifier `mpsc` (bounded, default 256). Backpressure in either direction ultimately parks the producing channel task — intended.
3. Spawn the ingestor task. Spawn the classifier worker task iff `ai_pipeline.is_some()`.
4. Call `reload_channels()` to spawn one `ChannelTask` per enabled row.

`reload_channels()`:
1. Read all rows from `channels`.
2. For each row: if enabled and no running task, spawn one. If disabled and running, cancel its child token and join.
3. No-op for unchanged rows. Factory lookup by `channel_type`; missing factory logs a warning and skips the row.

`shutdown()`:
1. Cancel root `CancellationToken`. All tasks see it in their `select!` and exit cleanly.
2. Drop the ingestor sender. Ingestor drains remaining jobs, then exits.
3. Drop the classifier sender. Classifier drains remaining ids, then exits.
4. Await `disconnect()` on every adapter owned by a channel task.
5. Join all `JoinHandle`s with `shutdown_timeout`; abort anything still alive.

### `ChannelTask`

One `tokio::spawn` per enabled channel row. Owns its `Box<dyn ChannelAdapter>` exclusively — no `Arc<Mutex<_>>` on the hot path.

Task body:
```
loop {
    select! {
        _ = token.cancelled() => break,
        _ = sleep(next_delay) => {
            match adapter.fetch_messages(last_sync_at).await {
                Ok(batch) => {
                    on_success(batch)   // push to ingestor, update DB, reset backoff
                }
                Err(e) => {
                    on_failure(e)       // bump backoff, emit SyncFailed, update status
                }
            }
        }
    }
}
adapter.disconnect().await; // on exit
```

`next_delay` is the channel's `poll_interval_secs` when healthy, or the current backoff value when degraded/failed.

### `Ingestor`

Single task. Drains `mpsc::Receiver<IngestJob>` where `IngestJob = { channel_id: Uuid, batch: Vec<RawMessage> }`.

Per message in a batch:
1. Resolve or insert sender `Contact` (`store::contacts::find_or_create_by_address(channel, address, display_name)` — add if missing from Plan 1 helpers).
2. Resolve or insert `Thread` (match by `(channel, external_thread_id)`, create if missing).
3. Call `adapters::normalize(raw, sender_id, thread_id)` to produce a `Message`.
4. Insert via `store::messages::insert_message`.
5. Emit `MessageIngested { id, channel_id }` on the broadcast channel.
6. Push the message id into the classifier mpsc (if the classifier exists). This is the only backpressure point: if the classifier queue is full, `send().await` parks the ingestor, which in turn parks the channel task that pushed the batch. That is the intended behavior.

Store errors on a single row are logged and skipped; the ingestor survives so one bad message cannot stall a batch.

### `ClassifierWorker`

Single task, spawned only if the builder received an `AiPipeline`. Drains `mpsc::Receiver<Uuid>`.

Per id:
1. Load the `Message` from store.
2. `pipeline.process(&msg).await` → returns `(category, priority)` plus internally logs to `action_log`.
3. Update row: `store::messages::set_classification(id, category, priority)` (add if missing from Plan 4 helpers).
4. Emit `MessageClassified { id, category, priority }`.

On error (LLM down, timeout, parse failure), the worker logs, writes `category=Unknown, priority=Low` to the row, and still emits `MessageClassified { category: Unknown, priority: Low }` so the message surfaces in UI. The spec is explicit: "AI failures: local pipeline defaults to no-priority (message still appears)."

### `Events`

```rust
#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    MessageIngested      { id: Uuid, channel_id: Uuid },
    MessageClassified    { id: Uuid, category: Category, priority: Priority },
    SyncSucceeded        { channel_id: Uuid, count: usize },
    SyncFailed           { channel_id: Uuid, error: String, attempt: u32 },
    ChannelStatusChanged { channel_id: Uuid, status: ChannelStatus },
}
```

Wrapped in a thin struct holding `broadcast::Sender<RuntimeEvent>`. `Runtime::subscribe()` returns a `broadcast::Receiver`. Lossy by design — a slow subscriber dropping events is preferable to the runtime stalling. Default buffer: 1024.

### `Status` + backoff

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelStatus {
    Healthy,
    Degraded { attempt: u32 },          // 1..=3 consecutive failures
    Failed   { last_error: String },    // 4+ consecutive failures
}
```

Backoff formula (per channel):
```
base = poll_interval_secs
delay = min(base * 2^attempt, 600) seconds, with ±20% jitter
```

A channel in `Failed` keeps trying at 10-minute max backoff — the user decides to disable it through the UI (future plan) or DB. Every transition emits `ChannelStatusChanged` and is persisted to the `channels` table.

## Data Storage

### Migration `005_runtime.sql`

```sql
ALTER TABLE channels ADD COLUMN status TEXT NOT NULL DEFAULT 'healthy';
ALTER TABLE channels ADD COLUMN last_error TEXT;
ALTER TABLE channels ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
```

`status` is one of `'healthy'`, `'degraded'`, `'failed'`. Simple string over an enum table; the authoritative enum lives in Rust.

### Reused columns

- `channels.last_sync_at` — updated by `ChannelTask` on successful fetch to the max timestamp in the batch.
- `channels.last_sync_cursor` — left untouched in Plan 6. Adapters currently take a `DateTime<Utc>` in `fetch_messages`; opaque-cursor support is a later plan if any adapter needs non-timestamp state.
- `channels.poll_interval_secs` — read at task spawn; hot-reload requires `reload_channels()`.
- `channels.enabled` — disabled rows are not spawned; disabling a running row causes `reload_channels()` to cancel it.

### New store helpers (in `store::channels`)

```rust
pub fn list_channels(conn: &Connection) -> Result<Vec<ChannelRow>>;
pub fn update_sync_state(conn: &Connection, id: Uuid, last_sync_at: DateTime<Utc>) -> Result<()>;
pub fn update_status(conn: &Connection, id: Uuid, status: &ChannelStatus) -> Result<()>;
```

`ChannelRow` mirrors the table schema as a Rust struct.

## Error Handling

| Failure mode | Behavior |
|---|---|
| `adapter.fetch_messages` error | Log, bump `consecutive_failures`, compute backoff, emit `SyncFailed { attempt }`, transition status, persist. **Channel task does not die.** |
| Persistent failure (e.g. 401 after token refresh) | Reaches `Failed` status after 4 consecutive errors; keeps trying at 10-min max. Visible to user via `ChannelStatusChanged`. |
| Ingestor store error on a single row | Log with row id, skip, continue batch. Ingestor never dies. |
| Ingestor channel closed (shutdown) | Drain remaining jobs, exit. |
| Classifier / LLM error | Log, write `(Unknown, Low)` to row, emit `MessageClassified { category: Unknown, priority: Low }`. Message still surfaces. |
| Classifier queue full | `send().await` parks the ingestor, which parks the pushing channel task. Backpressure propagates correctly. |
| `AiPipeline` absent at build time | Classifier worker is not spawned. Ingest events fire, classify events never do. |
| `shutdown()` join timeout | Tasks still running after timeout are aborted. Logged as warnings, not errors. |
| Factory missing for a `channel_type` | `reload_channels()` logs a warning and skips that row. Runtime continues. |

## Testing Strategy

All tests use `MockAdapter` (already exists in `core/src/adapters/mock.rs`), an in-memory `Store`, and `tokio::time::pause` where timing matters. No real network. No ignored tests introduced by Plan 6.

### Unit tests (in-module)

- `status.rs`: table-driven tests for the backoff math and state transitions (healthy → degraded → failed → healthy after one success).
- `events.rs`: construction and `Clone` round-trip (guards against accidentally non-Clone fields).

### Integration tests (`core/tests/runtime_*.rs`)

1. `runtime_full_loop.rs` — register a `MockFactory`, seed messages, `start()`, assert both `MessageIngested` and `MessageClassified` arrive, assert DB rows present with populated `category` and `priority`.
2. `runtime_graceful_degradation.rs` — build without `AiPipeline`; assert `MessageIngested` fires but `MessageClassified` never does; assert stored messages have `category = None`.
3. `runtime_backoff.rs` — `MockAdapter` scripted to fail N times then succeed. Assert events fire in order: `SyncFailed(attempt=1) → ChannelStatusChanged(Degraded{1}) → ... → SyncFailed(attempt=4) → ChannelStatusChanged(Failed) → SyncSucceeded → ChannelStatusChanged(Healthy)`. Uses `tokio::time::pause` + `advance`.
4. `runtime_shutdown.rs` — `start()`, seed work, call `shutdown()`, assert all tasks joined within the timeout and `disconnect()` was called on every adapter.
5. `runtime_reload.rs` — insert a new enabled row into `channels`, call `reload_channels()`, assert a new task is running for it; then set `enabled = 0` on that row, call `reload_channels()`, assert the task stops.
6. `runtime_classifier_failure.rs` — stub `AiPipeline` returns `Err`. Assert the message is still inserted, event is still emitted, row has `category=Unknown, priority=Low`.
7. `runtime_one_channel_does_not_affect_another.rs` — two channels, one fails repeatedly, the other succeeds. Assert the healthy one continues polling on schedule and its events fire normally while the other progresses through backoff.

## Deletions

- `core/src/adapters/manager.rs` — the entire file, including its tests. `grep -rn "AdapterManager"` on the tree confirms it is not referenced outside the adapters module (verified during brainstorming).
- Remove `pub mod manager;` from `core/src/adapters/mod.rs`.
- The `MockAdapter` struct in `core/src/adapters/mock.rs` is preserved — it is the test surface for Plan 6 integration tests.

## Migration Checklist

1. New migration `005_runtime.sql` adds the three `channels` columns.
2. New `core/src/runtime/` module compiled and exported via `core/src/lib.rs` (`pub mod runtime;`).
3. `core/src/adapters/manager.rs` deleted.
4. `core/src/adapters/mod.rs` no longer declares `pub mod manager`.
5. New `core/tests/runtime_*.rs` integration tests land green.
6. All existing tests continue to pass (confirms nothing upstream depended on `AdapterManager`).

## Dependencies

No new runtime dependencies. Plan 6 uses:
- `tokio` — already present (mpsc, broadcast, select, spawn, CancellationToken via `tokio-util` — `tokio-util = { version = "0.7", features = ["rt"] }` is the only addition if not already pulled in transitively).
- `tracing` — already present.
- `async-trait` — already present.
- `rand` — for ±20% jitter on backoff. Check if already a dep; if not, add `rand = "0.8"`.

Dev-only: `tokio = { version = "1", features = ["test-util"] }` for `tokio::time::pause` — already used by Plan 4 tests.

## Open Questions Resolved During Design

- **Factory registry vs DB-driven factory vs caller-registered adapters?** Hybrid — runtime owns a registry of factories keyed by `channel_type`; DB holds the rows; runtime instantiates adapters via the registry. Keeps `core` adapter-agnostic without forcing callers to reimplement the load loop.
- **Inline classification vs fan-out worker?** Fan-out worker. Protects the pipeline from a stuck LLM and gives the UI a two-phase render (raw → classified).
- **One ingestor or one per channel?** One. Serializes contact identity-merging naturally.
- **Cursor: `last_sync_at` or `last_sync_cursor`?** `last_sync_at` for now. Adapter trait is timestamp-based; opaque-cursor support is deferred.
- **Manager: keep, refactor, or replace?** Replace. The `Fn` callback shape does not compose with async ingestion; the refactor cost is lower than the wrapping cost.

---

*Spec approved 2026-04-19.*
