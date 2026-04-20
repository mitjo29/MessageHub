# Plan 7b.2: Three-Panel Inbox Layout — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve the desktop app from 7b.1's single flat list into a three-panel inbox: a left sidebar of Views (All / Unread / Priority) plus configured Channels, a middle message list scoped by the sidebar selection, and a right detail pane that auto-marks-read on open. Add 15 s polling with window-focus refresh, draggable + persisted panel widths, and arrow-key navigation — all behind a single `useReducer` + Context that 7b.3+ can extend.

**Architecture:** Core grows a `MessageFilter` struct and a `count_messages` helper. Tauri gains three commands (`mark_read`, `sidebar_counts`, plus an extended `list_messages`). The React app breaks out of `App.tsx` into `state/InboxContext.tsx` (reducer + provider + side-effect hooks) and four child components (`Sidebar`, `SplitPane`, `MessageList`, `MessageDetail`). No new npm deps.

**Tech Stack:** Rust 1.75+, Tauri 2.x, React 18, TypeScript 5 (strict), Vite 5. On top of 7b.1's existing stack.

**Prerequisites:**
- Plan 7b.1 merged on master (commit `9fd95b6` or later — verify with `git log --oneline | head -5`).
- `runtime-demo` produces messages that land in `core/messagehub.db` (used to exercise 7b.2 end-to-end).
- Node + Rust toolchain already verified during 7b.1. No new system packages.

**Spec:** `docs/superpowers/specs/2026-04-20-plan7b2-three-panel-layout-design.md`.

---

## File Structure

```
MessageHub/
├── core/
│   ├── src/
│   │   └── store/
│   │       ├── messages.rs                        MODIFY: MessageFilter + new list_messages signature + count_messages
│   │       └── mod.rs                             MODIFY: re-export MessageFilter
│   └── tests/
│       └── store_messages_test.rs                 MODIFY: migrate existing test + add filter/count tests
└── desktop/
    ├── src-tauri/
    │   └── src/
    │       ├── commands.rs                        MODIFY: Filter + to_core + updated list_messages + mark_read + sidebar_counts + DTOs
    │       └── main.rs                            MODIFY: register two new commands
    └── src/
        ├── api.ts                                 MODIFY: new filter arg + markRead + sidebarCounts
        ├── types.ts                               MODIFY: Filter + SidebarCounts + ChannelCount
        ├── App.tsx                                REWRITE: ~60 LOC shell wrapping <InboxProvider>
        ├── App.css                                REWRITE: three-column grid + sidebar/list/detail
        ├── state/
        │   └── InboxContext.tsx                   CREATE: reducer + provider + hooks
        └── components/
            ├── Sidebar.tsx                        CREATE
            ├── SplitPane.tsx                      CREATE
            ├── MessageList.tsx                    CREATE
            └── MessageDetail.tsx                  CREATE
```

---

### Task 1: Preflight + feature branch

**Files:** (none — setup only)

`★ Why this matters:` Start clean. Confirm the working tree is at 7b.1 + the 7b.2 spec commit, then branch before any edits.

- [ ] **Step 1: Confirm repo state**

```bash
git status
git log --oneline | head -5
```

Expected: working tree clean (aside from ignored/untracked noise like `.remember/`, `desktop/src-tauri/messagehub.db-shm`); top commit is `205ba85 docs(spec): Plan 7b.2 …` (or newer). If untracked shared files show up, leave them alone — they're not part of 7b.2.

- [ ] **Step 2: Create the feature branch**

```bash
git checkout -b feat/tauri-threepane
```

- [ ] **Step 3: Confirm baseline compiles**

```bash
cargo build --workspace
```

Expected: clean build of `messagehub-core` and `messagehub-desktop`. If this fails, fix the underlying issue or abort before proceeding; you do not want to debug a pre-existing problem alongside new changes.

---

### Task 2: Core — `MessageFilter` + new `list_messages` signature

**Files:**
- Modify: `core/src/store/messages.rs`
- Modify: `core/src/store/mod.rs`
- Modify: `core/tests/store_messages_test.rs`
- Modify: `desktop/src-tauri/src/commands.rs` (call-site migration only)

`★ Why this matters:` One struct replaces a pair of positional args and sets up a single point of extension for 7b.2's Unread + Priority views. Call-sites migrate in lockstep.

- [ ] **Step 1: Write the failing tests first**

Open `core/tests/store_messages_test.rs`. Replace the body of `test_list_messages_by_channel` and add three new tests below. Keep the existing helper functions (`test_store`, `make_contact`, `make_thread`, `make_message`) as-is.

```rust
#[test]
fn test_list_messages_default_filter_returns_all() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..3 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let filter = MessageFilter::default();
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 3);
}

#[test]
fn test_list_messages_by_channel() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..3 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let filter = MessageFilter {
        channel: Some(Channel::Email),
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 3);

    let filter_sms = MessageFilter {
        channel: Some(Channel::Sms),
        ..Default::default()
    };
    let empty = store.list_messages(&filter_sms, 10, 0).unwrap();
    assert_eq!(empty.len(), 0);
}

#[test]
fn test_list_messages_unread_only() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    // Two unread, one read.
    let m1 = make_message(contact.id, thread.id);
    let m2 = make_message(contact.id, thread.id);
    let m3 = make_message(contact.id, thread.id);
    store.insert_message(&m1).unwrap();
    store.insert_message(&m2).unwrap();
    store.insert_message(&m3).unwrap();
    store.mark_read(&m2.id, true).unwrap();

    let filter = MessageFilter {
        unread_only: true,
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|m| !m.is_read));
}

#[test]
fn test_list_messages_min_priority() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    // priority is 3 by default via make_message; add one at 5 and one at 1.
    let mut low = make_message(contact.id, thread.id);
    low.priority = PriorityScore::new(1);
    let mut high = make_message(contact.id, thread.id);
    high.priority = PriorityScore::new(5);
    let mid = make_message(contact.id, thread.id); // priority=3

    store.insert_message(&low).unwrap();
    store.insert_message(&mid).unwrap();
    store.insert_message(&high).unwrap();

    let filter = MessageFilter {
        min_priority: Some(4),
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].priority.unwrap().value(), 5);
}
```

`PriorityScore` and `MessageFilter` must be in scope — add to the imports if needed:

```rust
use messagehub_core::store::MessageFilter;
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p messagehub-core --test store_messages_test
```

Expected: compile error (`MessageFilter` doesn't exist, `list_messages` signature doesn't match). That's the "red" part of red-green-refactor.

- [ ] **Step 3: Add `MessageFilter` + rewrite `list_messages`**

Open `core/src/store/messages.rs`. At the top of the file, under the `use` block, add the struct:

```rust
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    pub channel: Option<Channel>,
    pub unread_only: bool,
    /// Inclusive floor on `priority_score`. `None` = any priority (including unset).
    pub min_priority: Option<u8>,
    pub archived: bool,
}
```

Replace the existing `list_messages` impl with:

```rust
pub fn list_messages(
    &self,
    filter: &MessageFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<Message>> {
    let mut sql = String::from(
        "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, \
         content_subject, attachments_json, timestamp, metadata_json, priority_score, \
         category, is_read, is_archived FROM messages WHERE is_archived = ?1"
    );
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(filter.archived as i32)];

    if let Some(ch) = filter.channel {
        params_vec.push(Box::new(ch.to_db_str().to_owned()));
        sql.push_str(&format!(" AND channel_type = ?{}", params_vec.len()));
    }
    if filter.unread_only {
        sql.push_str(" AND is_read = 0");
    }
    if let Some(min_p) = filter.min_priority {
        params_vec.push(Box::new(min_p as i32));
        sql.push_str(&format!(" AND priority_score IS NOT NULL AND priority_score >= ?{}", params_vec.len()));
    }

    let limit_idx = params_vec.len() + 1;
    sql.push_str(&format!(
        " ORDER BY timestamp DESC LIMIT ?{} OFFSET ?{}",
        limit_idx,
        limit_idx + 1
    ));
    params_vec.push(Box::new(limit));
    params_vec.push(Box::new(offset));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = self.conn().prepare(&sql)?;
    let messages: Vec<Message> = stmt
        .query_map(param_refs.as_slice(), |row| Ok(row_to_message(row)))?
        .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
        .into_iter()
        .collect::<std::result::Result<Vec<_>, CoreError>>()?;
    Ok(messages)
}
```

- [ ] **Step 4: Re-export `MessageFilter`**

Open `core/src/store/mod.rs` and add `MessageFilter` to the existing public re-exports alongside `Store`. Match the existing style — if you see `pub use messages::Store;`, add another line `pub use messages::MessageFilter;`. If there is already a glob re-export (`pub use messages::*;`), no change is needed.

- [ ] **Step 5: Migrate the Tauri call-site**

Open `desktop/src-tauri/src/commands.rs`. Find the `list_messages` body (~line 116) and replace the store call. Change:

```rust
let messages = store
    .list_messages(None, false, limit, offset)
    .map_err(|e| format!("list_messages failed: {}", e))?;
```

to:

```rust
let messages = store
    .list_messages(&messagehub_core::store::MessageFilter::default(), limit, offset)
    .map_err(|e| format!("list_messages failed: {}", e))?;
```

(This is a temporary bridge — Task 5 rewrites this command to take a real filter from TS.)

- [ ] **Step 6: Run tests to verify green**

```bash
cargo test -p messagehub-core --test store_messages_test
```

Expected: all four `test_list_messages_*` tests pass plus the existing `test_mark_message_read` still passes.

- [ ] **Step 7: Workspace build sanity**

```bash
cargo build --workspace
```

Expected: clean build. Warnings tolerable, errors not.

- [ ] **Step 8: Commit**

```bash
git add core/src/store/messages.rs core/src/store/mod.rs core/tests/store_messages_test.rs desktop/src-tauri/src/commands.rs
git commit -m "feat(core): introduce MessageFilter and migrate list_messages signature

Replaces the (channel, archived, limit, offset) positional args with
(&MessageFilter, limit, offset). Filter supports channel, unread_only,
min_priority, and archived — covering the new 'Unread' and 'Priority'
views in the desktop sidebar. Tauri call-site passes a default filter
for now; Plan 7b.2 Task 5 wires the real filter from the frontend."
```

---

### Task 3: Core — `count_messages`

**Files:**
- Modify: `core/src/store/messages.rs`
- Modify: `core/tests/store_messages_test.rs`

`★ Why this matters:` The sidebar needs per-view and per-channel counts. One SQL query per count is acceptable; the alternative (loading every row to count) scales badly.

- [ ] **Step 1: Add failing tests**

Append to `core/tests/store_messages_test.rs`:

```rust
#[test]
fn test_count_messages_default_matches_list_len() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..5 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let filter = MessageFilter::default();
    let count = store.count_messages(&filter).unwrap();
    let list_len = store.list_messages(&filter, 100, 0).unwrap().len() as u64;
    assert_eq!(count, list_len);
    assert_eq!(count, 5);
}

#[test]
fn test_count_messages_unread_only() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    let m1 = make_message(contact.id, thread.id);
    let m2 = make_message(contact.id, thread.id);
    store.insert_message(&m1).unwrap();
    store.insert_message(&m2).unwrap();
    store.mark_read(&m1.id, true).unwrap();

    let filter = MessageFilter {
        unread_only: true,
        ..Default::default()
    };
    assert_eq!(store.count_messages(&filter).unwrap(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p messagehub-core --test store_messages_test
```

Expected: compile error (`count_messages` doesn't exist).

- [ ] **Step 3: Implement `count_messages`**

Open `core/src/store/messages.rs`. Add the method inside the `impl Store` block, next to `list_messages`:

```rust
pub fn count_messages(&self, filter: &MessageFilter) -> Result<u64> {
    let mut sql = String::from(
        "SELECT COUNT(*) FROM messages WHERE is_archived = ?1"
    );
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(filter.archived as i32)];

    if let Some(ch) = filter.channel {
        params_vec.push(Box::new(ch.to_db_str().to_owned()));
        sql.push_str(&format!(" AND channel_type = ?{}", params_vec.len()));
    }
    if filter.unread_only {
        sql.push_str(" AND is_read = 0");
    }
    if let Some(min_p) = filter.min_priority {
        params_vec.push(Box::new(min_p as i32));
        sql.push_str(&format!(" AND priority_score IS NOT NULL AND priority_score >= ?{}", params_vec.len()));
    }

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let n: i64 = self.conn().query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
    Ok(n as u64)
}
```

- [ ] **Step 4: Run tests to verify green**

```bash
cargo test -p messagehub-core --test store_messages_test
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/store/messages.rs core/tests/store_messages_test.rs
git commit -m "feat(core): add Store::count_messages(&MessageFilter)

Single SQL COUNT query per call; WHERE clauses mirror list_messages
exactly so count ≡ list.len() for the same filter. Consumed by the
desktop sidebar via the upcoming sidebar_counts Tauri command."
```

---

### Task 4: Tauri backend — `Filter` enum + `to_core` + extended `list_messages` command

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`

`★ Why this matters:` This is the Rust/TS contract boundary for 7b.2. The tagged-enum serialization mirrors the TypeScript `Filter` type exactly, and `to_core` is the one place that encodes the "`PriorityHigh` means priority ≥ 4" threshold.

- [ ] **Step 1: Add the `Filter` enum + `to_core` helper**

Open `desktop/src-tauri/src/commands.rs`. Add imports if missing:

```rust
use messagehub_core::store::MessageFilter;
use messagehub_core::types::Channel;
use serde::Deserialize;
```

Add the enum below the existing DTOs:

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Filter {
    All,
    Unread,
    PriorityHigh,
    #[serde(rename_all = "camelCase")]
    Channel { channel_type: String },
}

impl Filter {
    fn to_core(&self) -> Result<MessageFilter, String> {
        Ok(match self {
            Filter::All => MessageFilter::default(),
            Filter::Unread => MessageFilter {
                unread_only: true,
                ..Default::default()
            },
            Filter::PriorityHigh => MessageFilter {
                min_priority: Some(4),
                ..Default::default()
            },
            Filter::Channel { channel_type } => {
                let ch = Channel::from_db_str(channel_type)
                    .ok_or_else(|| format!("unknown channel_type: {}", channel_type))?;
                MessageFilter {
                    channel: Some(ch),
                    ..Default::default()
                }
            }
        })
    }
}
```

- [ ] **Step 2: Update the `list_messages` command**

Find the existing `list_messages` command (the one you bridged with `MessageFilter::default()` in Task 2). Replace the full function body:

```rust
/// Return up to `limit` messages starting at `offset`, newest first, scoped
/// by the supplied filter.
#[tauri::command]
pub fn list_messages(
    filter: Filter,
    limit: u32,
    offset: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MessageRow>, String> {
    let core_filter = filter.to_core()?;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let messages = store
        .list_messages(&core_filter, limit, offset)
        .map_err(|e| format!("list_messages failed: {}", e))?;

    let rows = messages
        .iter()
        .map(|msg| {
            let sender_name = store
                .get_contact(&msg.sender_id)
                .map(|c| c.display_name)
                .unwrap_or_else(|_| msg.sender_id.to_string());
            build_message_row(msg, &state, sender_name)
        })
        .collect();

    Ok(rows)
}
```

- [ ] **Step 3: Add a unit test for `Filter::to_core`**

At the bottom of `desktop/src-tauri/src/commands.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_all_maps_to_default() {
        let core = Filter::All.to_core().unwrap();
        assert!(core.channel.is_none());
        assert!(!core.unread_only);
        assert!(core.min_priority.is_none());
        assert!(!core.archived);
    }

    #[test]
    fn filter_unread_sets_flag() {
        let core = Filter::Unread.to_core().unwrap();
        assert!(core.unread_only);
        assert!(core.min_priority.is_none());
    }

    #[test]
    fn filter_priority_high_sets_threshold_to_4() {
        let core = Filter::PriorityHigh.to_core().unwrap();
        assert_eq!(core.min_priority, Some(4));
    }

    #[test]
    fn filter_channel_resolves_known() {
        let core = Filter::Channel {
            channel_type: "Email".into(),
        }
        .to_core()
        .unwrap();
        assert_eq!(core.channel, Some(Channel::Email));
    }

    #[test]
    fn filter_channel_rejects_unknown() {
        let err = Filter::Channel {
            channel_type: "NotAChannel".into(),
        }
        .to_core()
        .unwrap_err();
        assert!(err.contains("unknown channel_type"));
    }
}
```

- [ ] **Step 4: Build + run backend tests**

```bash
cargo build -p messagehub-desktop
cargo test -p messagehub-desktop --lib
```

Expected: clean build, all five `filter_*` tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/commands.rs
git commit -m "feat(desktop): Filter enum + scoped list_messages command

Adds tagged-enum Filter (All, Unread, PriorityHigh, Channel{channelType})
with to_core() mapping to core::MessageFilter. The PriorityHigh≥4
threshold lives exclusively here; TS never encodes the number 4."
```

---

### Task 5: Tauri backend — `mark_read` command

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` First write operation from the UI. Small surface area, but it opens the door for 7b.3's reply composer; keep it narrow and disciplined.

- [ ] **Step 1: Add the command**

Append to `desktop/src-tauri/src/commands.rs` (above the `#[cfg(test)]` module):

```rust
/// Flip the `is_read` flag for a message.
#[tauri::command]
pub fn mark_read(
    id: String,
    read: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = Uuid::parse_str(&id).map_err(|e| format!("invalid id: {}", e))?;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    store
        .mark_read(&uuid, read)
        .map_err(|e| format!("mark_read failed: {}", e))
}
```

- [ ] **Step 2: Register the command in `main.rs`**

Open `desktop/src-tauri/src/main.rs`. Find the `invoke_handler` line and add `commands::mark_read` to the macro list. The existing block looks like:

```rust
.invoke_handler(tauri::generate_handler![
    commands::list_messages,
    commands::get_message,
    commands::list_channels,
    commands::get_config,
])
```

Change it to:

```rust
.invoke_handler(tauri::generate_handler![
    commands::list_messages,
    commands::get_message,
    commands::list_channels,
    commands::get_config,
    commands::mark_read,
])
```

Apply the same change to the fallback/error-case branch if `main.rs` has two builders (the init-failure path from 7b.1). If unsure, `git grep -n "generate_handler" desktop/src-tauri/src/main.rs` will surface every occurrence — update each.

- [ ] **Step 3: Build**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): mark_read Tauri command

First write operation in the desktop app. UUID-parses the id, takes
the store lock, calls Store::mark_read. Consumed by MessageDetail's
optimistic mark-read effect in Task 12."
```

---

### Task 6: Tauri backend — `sidebar_counts` command + DTOs

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` One round-trip returns every number the sidebar needs. Doing this as three-plus separate invokes would flicker badly when the filter changes.

- [ ] **Step 1: Add the DTOs**

In `desktop/src-tauri/src/commands.rs`, after the existing `UiConfig` struct, add:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCount {
    pub channel_type: String,
    pub total: u64,
    pub unread: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarCounts {
    pub all: u64,
    pub unread: u64,
    pub priority_high: u64,
    pub by_channel: Vec<ChannelCount>,
}
```

- [ ] **Step 2: Implement the command**

Append to `desktop/src-tauri/src/commands.rs`:

```rust
/// Return a batched snapshot of sidebar counts: one entry per view + per
/// channel. Uses core::MessageFilter + count_messages — one SQL query per
/// field. For ~5 channels this is ~13 COUNT(*) calls; cheap against an
/// indexed column.
#[tauri::command]
pub fn sidebar_counts(state: State<'_, AppState>) -> Result<SidebarCounts, String> {
    use messagehub_core::store::MessageFilter;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let all = store
        .count_messages(&MessageFilter::default())
        .map_err(|e| format!("count all failed: {}", e))?;

    let unread = store
        .count_messages(&MessageFilter {
            unread_only: true,
            ..Default::default()
        })
        .map_err(|e| format!("count unread failed: {}", e))?;

    let priority_high = store
        .count_messages(&MessageFilter {
            min_priority: Some(4),
            ..Default::default()
        })
        .map_err(|e| format!("count priorityHigh failed: {}", e))?;

    let configs = store
        .list_channel_configs()
        .map_err(|e| format!("list_channel_configs failed: {}", e))?;

    let mut by_channel = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cfg in &configs {
        if !seen.insert(cfg.channel) {
            // Multiple configs per channel variant (e.g. two Email accounts)
            // — we roll up to the variant level so 7b.2's sidebar has one
            // row per channel. Multi-account UI is deferred.
            continue;
        }
        let total = store
            .count_messages(&MessageFilter {
                channel: Some(cfg.channel),
                ..Default::default()
            })
            .map_err(|e| format!("count channel {} failed: {}", cfg.channel, e))?;
        let chan_unread = store
            .count_messages(&MessageFilter {
                channel: Some(cfg.channel),
                unread_only: true,
                ..Default::default()
            })
            .map_err(|e| format!("count unread for channel {} failed: {}", cfg.channel, e))?;
        by_channel.push(ChannelCount {
            channel_type: cfg.channel.to_db_str().to_string(),
            total,
            unread: chan_unread,
        });
    }

    Ok(SidebarCounts {
        all,
        unread,
        priority_high,
        by_channel,
    })
}
```

- [ ] **Step 3: Register the command**

Open `desktop/src-tauri/src/main.rs`. Extend every `generate_handler!` block to include `commands::sidebar_counts`:

```rust
.invoke_handler(tauri::generate_handler![
    commands::list_messages,
    commands::get_message,
    commands::list_channels,
    commands::get_config,
    commands::mark_read,
    commands::sidebar_counts,
])
```

- [ ] **Step 4: Build**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): sidebar_counts Tauri command

One round-trip returns all/unread/priorityHigh totals plus per-channel
total+unread. Channels with multiple configs per variant fold into one
row for 7b.2; multi-account UI is deferred."
```

---

### Task 7: Frontend — `types.ts` and `api.ts` extensions

**Files:**
- Modify: `desktop/src/types.ts`
- Modify: `desktop/src/api.ts`

`★ Why this matters:` The TS types are hand-written to mirror the Rust DTOs. Getting the tagged-enum shape of `Filter` right is the only thing worth pausing over here.

- [ ] **Step 1: Extend `types.ts`**

Open `desktop/src/types.ts`. Keep the existing exports (`MessageRow`, `AttachmentInfo`, `MessageDetail`, `ChannelInfo`, `UiConfig`) untouched. Append:

```typescript
export type Filter =
  | { kind: "all" }
  | { kind: "unread" }
  | { kind: "priorityHigh" }
  | { kind: "channel"; channelType: string };

export type ChannelCount = {
  channelType: string;
  total: number;
  unread: number;
};

export type SidebarCounts = {
  all: number;
  unread: number;
  priorityHigh: number;
  byChannel: ChannelCount[];
};
```

(`u64` on the Rust side serializes as a JS `number`. For total counts under Number.MAX_SAFE_INTEGER we're fine; this is messages, not atoms.)

- [ ] **Step 2: Extend `api.ts`**

Replace the contents of `desktop/src/api.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";
import type {
  MessageRow,
  MessageDetail,
  ChannelInfo,
  UiConfig,
  Filter,
  SidebarCounts,
} from "./types";

export const api = {
  listMessages: (filter: Filter, limit: number, offset: number) =>
    invoke<MessageRow[]>("list_messages", { filter, limit, offset }),

  getMessage: (id: string) =>
    invoke<MessageDetail>("get_message", { id }),

  listChannels: () =>
    invoke<ChannelInfo[]>("list_channels"),

  getConfig: () =>
    invoke<UiConfig>("get_config"),

  markRead: (id: string, read: boolean) =>
    invoke<void>("mark_read", { id, read }),

  sidebarCounts: () =>
    invoke<SidebarCounts>("sidebar_counts"),
};
```

- [ ] **Step 3: Typecheck**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Expected: tsc **will fail** here — the old `App.tsx` still calls `api.listMessages(PAGE_SIZE, 0)` with the 2-arg signature, but `listMessages` now requires 3 args. This is intentional; Task 13 rewrites `App.tsx` and the chain re-greens. Do not try to patch the old App.tsx into compatibility — it's about to be deleted. Skip `tsc --noEmit` for Tasks 7 through 12 and rely on Task 13's typecheck as the gate.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/types.ts desktop/src/api.ts
git commit -m "feat(desktop): extend TS types + api with Filter, SidebarCounts, markRead

Mirrors the Tauri-side Filter tagged enum and SidebarCounts DTO. The
existing App.tsx still references the old single-arg listMessages and
will be rewritten in Task 12; typecheck may warn until then."
```

---

### Task 8: Frontend — `InboxContext` reducer skeleton

**Files:**
- Create: `desktop/src/state/InboxContext.tsx`

`★ Why this matters:` All 7b.2 state lives behind a single reducer + Context. Build the pure state-machine half first (no side effects); wire effects in Task 14. This split keeps the reducer testable by eye and makes the Task 14 diff small.

- [ ] **Step 1: Create the file**

Create `desktop/src/state/InboxContext.tsx` with the following complete contents:

```typescript
import {
  createContext,
  useContext,
  useReducer,
  type Dispatch,
  type ReactNode,
} from "react";
import type {
  MessageRow,
  MessageDetail,
  ChannelInfo,
  SidebarCounts,
  Filter,
} from "../types";

export const DEFAULT_PANEL_WIDTHS = { sidebar: 200, list: 360 };
export const PANEL_WIDTHS_KEY = "messagehub.desktop.panelWidths.v1";

export type InboxState = {
  filter: Filter;
  channels: ChannelInfo[];
  counts: SidebarCounts | null;
  messages: MessageRow[];
  hasMore: boolean;
  selectedId: string | null;
  detail: MessageDetail | null;
  panelWidths: { sidebar: number; list: number };
  error: string | null;
  loading: boolean;
};

export type InboxAction =
  | { type: "SET_FILTER"; filter: Filter }
  | { type: "SET_CHANNELS"; channels: ChannelInfo[] }
  | { type: "SET_COUNTS"; counts: SidebarCounts }
  | {
      type: "LOAD_MESSAGES_SUCCESS";
      messages: MessageRow[];
      append: boolean;
      hasMore: boolean;
    }
  | { type: "SELECT"; id: string | null }
  | { type: "LOAD_DETAIL_SUCCESS"; detail: MessageDetail }
  | { type: "MARK_READ_LOCAL"; id: string }
  | { type: "REVERT_MARK_READ_LOCAL"; id: string }
  | { type: "SET_PANEL_WIDTHS"; widths: { sidebar: number; list: number } }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "SET_LOADING"; loading: boolean };

export function loadInitialPanelWidths(): { sidebar: number; list: number } {
  try {
    const raw = window.localStorage.getItem(PANEL_WIDTHS_KEY);
    if (!raw) return DEFAULT_PANEL_WIDTHS;
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed.sidebar === "number" &&
      typeof parsed.list === "number"
    ) {
      return { sidebar: parsed.sidebar, list: parsed.list };
    }
    return DEFAULT_PANEL_WIDTHS;
  } catch {
    return DEFAULT_PANEL_WIDTHS;
  }
}

export const initialState: InboxState = {
  filter: { kind: "all" },
  channels: [],
  counts: null,
  messages: [],
  hasMore: false,
  selectedId: null,
  detail: null,
  panelWidths: DEFAULT_PANEL_WIDTHS, // hydrated on mount; see provider
  error: null,
  loading: false,
};

function bumpCounts(
  counts: SidebarCounts | null,
  row: MessageRow,
  delta: number,
): SidebarCounts | null {
  if (!counts) return counts;
  const byChannel = counts.byChannel.map((c) =>
    c.channelType === row.channel
      ? { ...c, unread: Math.max(0, c.unread + delta) }
      : c,
  );
  // `all` and `priorityHigh` are totals, not unread counts — mark-read
  // doesn't move them. Only `unread` (overall) and per-channel `unread` do.
  return {
    ...counts,
    unread: Math.max(0, counts.unread + delta),
    byChannel,
  };
}

export function inboxReducer(
  state: InboxState,
  action: InboxAction,
): InboxState {
  switch (action.type) {
    case "SET_FILTER":
      return {
        ...state,
        filter: action.filter,
        messages: [],
        hasMore: false,
        selectedId: null,
        detail: null,
        error: null,
      };

    case "SET_CHANNELS":
      return { ...state, channels: action.channels };

    case "SET_COUNTS":
      return { ...state, counts: action.counts };

    case "LOAD_MESSAGES_SUCCESS":
      return {
        ...state,
        messages: action.append
          ? [...state.messages, ...action.messages]
          : action.messages,
        hasMore: action.hasMore,
        loading: false,
        error: null,
      };

    case "SELECT":
      return {
        ...state,
        selectedId: action.id,
        detail: action.id === null ? null : state.detail,
      };

    case "LOAD_DETAIL_SUCCESS":
      return { ...state, detail: action.detail, error: null };

    case "MARK_READ_LOCAL": {
      const row = state.messages.find((m) => m.id === action.id);
      if (!row || row.is_read) return state;
      const markedRow: MessageRow = { ...row, is_read: true };
      const messages =
        state.filter.kind === "unread"
          ? state.messages.filter((m) => m.id !== action.id)
          : state.messages.map((m) => (m.id === action.id ? markedRow : m));
      return {
        ...state,
        messages,
        counts: bumpCounts(state.counts, row, -1),
        detail: state.detail && state.detail.id === action.id
          ? { ...state.detail, is_read: true }
          : state.detail,
      };
    }

    case "REVERT_MARK_READ_LOCAL": {
      const row =
        state.messages.find((m) => m.id === action.id) ||
        (state.detail && state.detail.id === action.id
          ? ({
              id: state.detail.id,
              timestamp: state.detail.timestamp,
              channel: state.detail.channel,
              channel_label: state.detail.channel_label,
              sender_name: state.detail.sender_name,
              subject: state.detail.subject,
              preview: state.detail.preview,
              category: state.detail.category,
              priority: state.detail.priority,
              is_read: true,
            } as MessageRow)
          : null);
      if (!row) return state;
      const revertedRow: MessageRow = { ...row, is_read: false };
      const messages =
        state.filter.kind === "unread" &&
        !state.messages.some((m) => m.id === action.id)
          ? [revertedRow, ...state.messages]
          : state.messages.map((m) =>
              m.id === action.id ? revertedRow : m,
            );
      return {
        ...state,
        messages,
        counts: bumpCounts(state.counts, revertedRow, +1),
        detail: state.detail && state.detail.id === action.id
          ? { ...state.detail, is_read: false }
          : state.detail,
      };
    }

    case "SET_PANEL_WIDTHS":
      return { ...state, panelWidths: action.widths };

    case "SET_ERROR":
      return { ...state, error: action.error };

    case "SET_LOADING":
      return { ...state, loading: action.loading };

    default:
      return state;
  }
}

const InboxContext = createContext<{
  state: InboxState;
  dispatch: Dispatch<InboxAction>;
} | null>(null);

export function InboxProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(inboxReducer, initialState, (s) => ({
    ...s,
    panelWidths: loadInitialPanelWidths(),
  }));
  return (
    <InboxContext.Provider value={{ state, dispatch }}>
      {children}
    </InboxContext.Provider>
  );
}

export function useInbox() {
  const ctx = useContext(InboxContext);
  if (!ctx) {
    throw new Error("useInbox must be used inside <InboxProvider>");
  }
  return ctx;
}
```

- [ ] **Step 2: Commit**

```bash
git add desktop/src/state/InboxContext.tsx
git commit -m "feat(desktop): InboxContext reducer + provider skeleton

Pure state machine covering filter, channels, counts, messages,
selection, detail, panel widths, error, loading. Optimistic
MARK_READ_LOCAL + REVERT_MARK_READ_LOCAL actions handle the
mark-read round-trip. No side effects yet — hooks land in Task 14."
```

---

### Task 9: Frontend — `SplitPane` component

**Files:**
- Create: `desktop/src/components/SplitPane.tsx`

`★ Why this matters:` Tiny hand-rolled resize handle. Drives the widths in the reducer via dispatch. No library, no measured-layout nonsense — just a drag handle that mutates a prop.

- [ ] **Step 1: Create the file**

Create `desktop/src/components/SplitPane.tsx`:

```typescript
import { useCallback, useRef } from "react";

type Props = {
  /** "sidebar" resizes the sidebar↔list gap; "list" resizes the list↔detail gap. */
  target: "sidebar" | "list";
  /** Called on every mousemove with the proposed new width (clamped). */
  onResize: (width: number) => void;
  /** Current width of the *target* column so we can compute deltas. */
  currentWidth: number;
  /** Min/max for the target column. */
  min: number;
  max: number;
};

export function SplitPane({
  target,
  onResize,
  currentWidth,
  min,
  max,
}: Props) {
  const startXRef = useRef(0);
  const startWidthRef = useRef(0);

  const onMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      startXRef.current = e.clientX;
      startWidthRef.current = currentWidth;
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const onMove = (me: MouseEvent) => {
        const delta = me.clientX - startXRef.current;
        const next = Math.max(min, Math.min(max, startWidthRef.current + delta));
        onResize(next);
      };
      const onUp = () => {
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [currentWidth, min, max, onResize],
  );

  return (
    <div
      className="split-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={target === "sidebar" ? "Resize sidebar" : "Resize message list"}
      onMouseDown={onMouseDown}
    />
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add desktop/src/components/SplitPane.tsx
git commit -m "feat(desktop): SplitPane resize handle

6px draggable handle. Parent supplies currentWidth + min/max; onResize
fires on every mousemove with the clamped next width. Body cursor +
user-select swap for the drag duration, restored on mouseup."
```

---

### Task 10: Frontend — `Sidebar` component

**Files:**
- Create: `desktop/src/components/Sidebar.tsx`

`★ Why this matters:` First component that reads from the reducer. Also the simplest — pure function of `state.channels` + `state.counts` + `state.filter`.

- [ ] **Step 1: Create the file**

Create `desktop/src/components/Sidebar.tsx`:

```typescript
import { useInbox } from "../state/InboxContext";
import type { Filter } from "../types";

type ItemProps = {
  active: boolean;
  label: string;
  total: number | null;
  unread?: number | null;
  onClick: () => void;
  disabled?: boolean;
};

function Item({ active, label, total, unread, onClick, disabled }: ItemProps) {
  const totalText =
    total === null ? "—" : total === 0 ? "—" : total.toString();
  return (
    <div
      className={`sidebar-item${active ? " active" : ""}${disabled ? " disabled" : ""}`}
      aria-selected={active}
      role="option"
      onClick={onClick}
    >
      <span className="sidebar-label">{label}</span>
      <span className="sidebar-counts">
        {unread != null && unread > 0 && (
          <span className="sidebar-unread">{unread}</span>
        )}
        <span className="sidebar-total">{totalText}</span>
      </span>
    </div>
  );
}

function filtersEqual(a: Filter, b: Filter): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "channel" && b.kind === "channel") {
    return a.channelType === b.channelType;
  }
  return true;
}

export function Sidebar() {
  const { state, dispatch } = useInbox();
  const { channels, counts, filter } = state;

  const setFilter = (next: Filter) => {
    if (filtersEqual(filter, next)) return;
    dispatch({ type: "SET_FILTER", filter: next });
  };

  return (
    <nav className="sidebar" aria-label="Inbox navigation">
      <div className="sidebar-section-label">Views</div>
      <Item
        active={filter.kind === "all"}
        label="All"
        total={counts?.all ?? null}
        onClick={() => setFilter({ kind: "all" })}
      />
      <Item
        active={filter.kind === "unread"}
        label="Unread"
        total={counts?.unread ?? null}
        unread={counts?.unread ?? null}
        onClick={() => setFilter({ kind: "unread" })}
      />
      <Item
        active={filter.kind === "priorityHigh"}
        label="Priority"
        total={counts?.priorityHigh ?? null}
        onClick={() => setFilter({ kind: "priorityHigh" })}
      />

      <div className="sidebar-section-label">Channels</div>
      {channels.length === 0 ? (
        <div className="sidebar-empty">No channels</div>
      ) : (
        channels.map((c) => {
          const cc = counts?.byChannel.find(
            (x) => x.channelType === c.channel_type,
          );
          const active =
            filter.kind === "channel" && filter.channelType === c.channel_type;
          return (
            <Item
              key={c.id}
              active={active}
              label={c.label || c.channel_type}
              total={cc?.total ?? null}
              unread={cc?.unread ?? null}
              disabled={!c.enabled}
              onClick={() =>
                setFilter({
                  kind: "channel",
                  channelType: c.channel_type,
                })
              }
            />
          );
        })
      )}
    </nav>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add desktop/src/components/Sidebar.tsx
git commit -m "feat(desktop): Sidebar with Views + Channels + live counts

Reads from InboxContext. Three fixed view items (All/Unread/Priority)
plus one row per configured channel. Counts come from state.counts;
clicking a row dispatches SET_FILTER. Clicking the currently-selected
row is a no-op (radio semantics)."
```

---

### Task 11: Frontend — `MessageList` component + keyboard nav

**Files:**
- Create: `desktop/src/components/MessageList.tsx`

`★ Why this matters:` Middle panel. Rows look the same as 7b.1 but the container is now focusable for keyboard nav, and selection is a persistent visual state rather than a route change.

- [ ] **Step 1: Create the file**

Create `desktop/src/components/MessageList.tsx`:

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { useInbox } from "../state/InboxContext";
import type { MessageRow } from "../types";

const PAGE_SIZE = 50;

function formatTime(isoString: string): string {
  const d = new Date(isoString);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function MessageList() {
  const { state, dispatch } = useInbox();
  const { messages, selectedId, hasMore, loading, filter } = state;
  const [loadingMore, setLoadingMore] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  const openMessage = useCallback(
    async (id: string) => {
      dispatch({ type: "SELECT", id });
      try {
        const d = await api.getMessage(id);
        dispatch({ type: "LOAD_DETAIL_SUCCESS", detail: d });
      } catch (err) {
        dispatch({ type: "SET_ERROR", error: String(err) });
      }
    },
    [dispatch],
  );

  const onClickRow = (row: MessageRow) => {
    void openMessage(row.id);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (messages.length === 0) return;
    const currentIdx = selectedId
      ? messages.findIndex((m) => m.id === selectedId)
      : -1;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = Math.min(messages.length - 1, currentIdx + 1);
      if (next !== currentIdx) void openMessage(messages[next].id);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, currentIdx - 1);
      if (currentIdx === -1) {
        void openMessage(messages[0].id);
      } else if (next !== currentIdx) {
        void openMessage(messages[next].id);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (currentIdx >= 0) void openMessage(messages[currentIdx].id);
    } else if (e.key === "Escape") {
      e.preventDefault();
      dispatch({ type: "SELECT", id: null });
    }
  };

  useEffect(() => {
    if (!selectedId || !listRef.current) return;
    const el = listRef.current.querySelector<HTMLElement>(
      `[data-row-id="${selectedId}"]`,
    );
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  const loadMore = async () => {
    if (loadingMore) return;
    setLoadingMore(true);
    try {
      const next = await api.listMessages(filter, PAGE_SIZE, messages.length);
      dispatch({
        type: "LOAD_MESSAGES_SUCCESS",
        messages: next,
        append: true,
        hasMore: next.length === PAGE_SIZE,
      });
    } catch (err) {
      dispatch({ type: "SET_ERROR", error: String(err) });
    } finally {
      setLoadingMore(false);
    }
  };

  return (
    <div
      className="message-list"
      tabIndex={0}
      role="listbox"
      aria-activedescendant={selectedId ?? undefined}
      onKeyDown={onKeyDown}
      ref={listRef}
    >
      {messages.length === 0 && !loading && (
        <div className="empty">No messages in this view.</div>
      )}

      {messages.map((m) => (
        <div
          key={m.id}
          id={m.id}
          data-row-id={m.id}
          role="option"
          aria-selected={selectedId === m.id}
          className={`message-row${m.is_read ? "" : " unread"}${selectedId === m.id ? " selected" : ""}`}
          onClick={() => onClickRow(m)}
        >
          <div className="row-main">
            <span className="time">{formatTime(m.timestamp)}</span>
            <span className="channel">[{m.channel_label ?? m.channel}]</span>
            <span className="sender">{m.sender_name}</span>
          </div>
          <div className="row-subject">{m.subject ?? "(no subject)"}</div>
          <div className="row-preview">{m.preview}</div>
          <div className="row-meta">
            {m.category ?? "—"}
            {m.priority !== null ? ` · P${m.priority}` : ""}
          </div>
        </div>
      ))}

      {hasMore && (
        <button className="load-more" onClick={loadMore} disabled={loadingMore}>
          {loadingMore ? "Loading…" : "Load more"}
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add desktop/src/components/MessageList.tsx
git commit -m "feat(desktop): MessageList with keyboard nav + persistent selection

↑/↓ move the selection (also triggers detail fetch so the right pane
stays live), Enter opens, Esc clears. Container is listbox-role +
tabIndex=0 so focus-visible hints land on the list itself. Selected
row keeps an accent background independent of hover."
```

---

### Task 12: Frontend — `MessageDetail` component + mark-read effect

**Files:**
- Create: `desktop/src/components/MessageDetail.tsx`

`★ Why this matters:` First write path from the UI. Optimistic flip → `api.markRead` → revert on error. Keep the effect small; the reducer owns the actual state transitions.

- [ ] **Step 1: Create the file**

Create `desktop/src/components/MessageDetail.tsx`:

```typescript
import { useEffect } from "react";
import { api } from "../api";
import { useInbox } from "../state/InboxContext";

function formatTime(isoString: string): string {
  const d = new Date(isoString);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function MessageDetail() {
  const { state, dispatch } = useInbox();
  const { detail } = state;

  useEffect(() => {
    if (!detail) return;
    if (detail.is_read) return;

    const id = detail.id;
    dispatch({ type: "MARK_READ_LOCAL", id });

    let cancelled = false;
    api.markRead(id, true).catch((err) => {
      if (cancelled) return;
      dispatch({ type: "REVERT_MARK_READ_LOCAL", id });
      dispatch({ type: "SET_ERROR", error: String(err) });
    });
    return () => {
      cancelled = true;
    };
  }, [detail?.id, detail?.is_read, dispatch]);

  if (!detail) {
    return (
      <div className="detail-pane empty">
        <p>Select a message.</p>
      </div>
    );
  }

  return (
    <div className="detail-pane">
      <div className="detail-head">
        <span className="channel">
          [{detail.channel_label ?? detail.channel}]
        </span>
        <span className="sender">{detail.sender_name}</span>
        <span className="time">{formatTime(detail.timestamp)}</span>
      </div>
      <h2 className="detail-subject">{detail.subject ?? "(no subject)"}</h2>
      <div className="detail-meta">
        {detail.category ?? "—"}
        {detail.priority !== null ? ` · P${detail.priority}` : ""}
      </div>
      <pre className="detail-body">{detail.body}</pre>
      {detail.attachments.length > 0 && (
        <div className="detail-attachments">
          <strong>Attachments:</strong>
          <ul>
            {detail.attachments.map((a) => (
              <li key={a.filename}>
                {a.filename} ({(a.size_bytes / 1024).toFixed(1)} KB)
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add desktop/src/components/MessageDetail.tsx
git commit -m "feat(desktop): MessageDetail with optimistic mark-read

When an unread detail loads, dispatch MARK_READ_LOCAL synchronously
(UI updates immediately) and fire api.markRead. On rejection, dispatch
REVERT_MARK_READ_LOCAL + SET_ERROR so the row flips back and the
banner shows. The cancelled-flag guards against stale callbacks when
the user moves to another message mid-flight."
```

---

### Task 13: Frontend — `App.tsx` rewrite + `InboxLayout`

**Files:**
- Modify: `desktop/src/App.tsx`

`★ Why this matters:` The shell. After this task the components render in their panels but the app doesn't load data yet — that's Task 14. Type-check must pass.

- [ ] **Step 1: Replace `App.tsx` entirely**

Overwrite `desktop/src/App.tsx` with:

```typescript
import { InboxProvider, useInbox } from "./state/InboxContext";
import { Sidebar } from "./components/Sidebar";
import { SplitPane } from "./components/SplitPane";
import { MessageList } from "./components/MessageList";
import { MessageDetail } from "./components/MessageDetail";

const MIN = { sidebar: 160, list: 260, detail: 320 };
const HANDLE_PX = 6;

function InboxLayout() {
  const { state, dispatch } = useInbox();
  const { panelWidths, error } = state;
  const winW = typeof window !== "undefined" ? window.innerWidth : 1000;

  const sidebarMax = Math.max(
    MIN.sidebar,
    winW - (MIN.list + MIN.detail + HANDLE_PX * 2),
  );
  const listMax = Math.max(
    MIN.list,
    winW - (panelWidths.sidebar + MIN.detail + HANDLE_PX * 2),
  );

  const gridCols = `${panelWidths.sidebar}px ${HANDLE_PX}px ${panelWidths.list}px ${HANDLE_PX}px 1fr`;

  const setSidebar = (w: number) =>
    dispatch({
      type: "SET_PANEL_WIDTHS",
      widths: { ...panelWidths, sidebar: w },
    });
  const setList = (w: number) =>
    dispatch({
      type: "SET_PANEL_WIDTHS",
      widths: { ...panelWidths, list: w },
    });

  return (
    <div className="app">
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button
            className="error-dismiss"
            onClick={() => dispatch({ type: "SET_ERROR", error: null })}
            aria-label="Dismiss error"
          >
            ×
          </button>
        </div>
      )}
      <div className="inbox-grid" style={{ gridTemplateColumns: gridCols }}>
        <Sidebar />
        <SplitPane
          target="sidebar"
          onResize={setSidebar}
          currentWidth={panelWidths.sidebar}
          min={MIN.sidebar}
          max={sidebarMax}
        />
        <MessageList />
        <SplitPane
          target="list"
          onResize={setList}
          currentWidth={panelWidths.list}
          min={MIN.list}
          max={listMax}
        />
        <MessageDetail />
      </div>
    </div>
  );
}

export default function App() {
  return (
    <InboxProvider>
      <InboxLayout />
    </InboxProvider>
  );
}
```

- [ ] **Step 2: Typecheck**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Expected: no errors. If any error references missing `formatTime` or similar, it means the old helper in `App.tsx` was referenced externally — unlikely; investigate the exact error. No unused imports should remain now.

- [ ] **Step 3: Commit**

```bash
git add desktop/src/App.tsx
git commit -m "feat(desktop): App shell wires InboxProvider + three-column layout

CSS Grid with template-columns built from state.panelWidths. Two
SplitPane handles dispatch SET_PANEL_WIDTHS with clamped values
(min/max computed from window.innerWidth). Error banner shows when
state.error is set; dismiss × clears it. No data loading yet — that
lands in Task 14."
```

---

### Task 14: Frontend — side-effect hooks (mount / filter / poll / persistence)

**Files:**
- Modify: `desktop/src/state/InboxContext.tsx`

`★ Why this matters:` This is the task that makes the app come alive. Four effects in `InboxProvider`: initial load, filter-change reload, 15 s + focus polling, and panel-width persistence.

- [ ] **Step 1: Extend the provider**

Open `desktop/src/state/InboxContext.tsx`. At the top, add `useEffect` + `useRef` to the React imports:

```typescript
import {
  createContext,
  useContext,
  useEffect,
  useReducer,
  useRef,
  type Dispatch,
  type ReactNode,
} from "react";
```

Import the api client at the top of the file:

```typescript
import { api } from "../api";
```

Replace the current `InboxProvider` function with this expanded version (the rest of the file stays the same):

```typescript
const PAGE_SIZE = 50;
const POLL_INTERVAL_MS = 15_000;
const PERSIST_DEBOUNCE_MS = 200;

export function InboxProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(inboxReducer, initialState, (s) => ({
    ...s,
    panelWidths: loadInitialPanelWidths(),
  }));

  // One-time channels fetch on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const channels = await api.listChannels();
        if (!cancelled) dispatch({ type: "SET_CHANNELS", channels });
      } catch (err) {
        if (!cancelled) dispatch({ type: "SET_ERROR", error: String(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Load messages + counts whenever the filter changes (and on mount).
  const filterRef = useRef(state.filter);
  filterRef.current = state.filter;

  useEffect(() => {
    let cancelled = false;
    dispatch({ type: "SET_LOADING", loading: true });
    (async () => {
      try {
        const [rows, counts] = await Promise.all([
          api.listMessages(state.filter, PAGE_SIZE, 0),
          api.sidebarCounts(),
        ]);
        if (cancelled) return;
        dispatch({
          type: "LOAD_MESSAGES_SUCCESS",
          messages: rows,
          append: false,
          hasMore: rows.length === PAGE_SIZE,
        });
        dispatch({ type: "SET_COUNTS", counts });
      } catch (err) {
        if (!cancelled) dispatch({ type: "SET_ERROR", error: String(err) });
      } finally {
        if (!cancelled) dispatch({ type: "SET_LOADING", loading: false });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [state.filter]);

  // Poll every POLL_INTERVAL_MS + on window focus. Refetches counts + page 0
  // only; preserves selection and already-loaded older pages.
  useEffect(() => {
    const tick = async () => {
      try {
        const [rows, counts] = await Promise.all([
          api.listMessages(filterRef.current, PAGE_SIZE, 0),
          api.sidebarCounts(),
        ]);
        dispatch({
          type: "LOAD_MESSAGES_SUCCESS",
          messages: rows,
          append: false,
          hasMore: rows.length === PAGE_SIZE,
        });
        dispatch({ type: "SET_COUNTS", counts });
      } catch {
        // Silent — polling shouldn't spam the banner.
      }
    };
    const id = window.setInterval(tick, POLL_INTERVAL_MS);
    const onVis = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, []);

  // Persist panel widths with debounce.
  useEffect(() => {
    const handle = window.setTimeout(() => {
      try {
        window.localStorage.setItem(
          PANEL_WIDTHS_KEY,
          JSON.stringify(state.panelWidths),
        );
      } catch {
        // localStorage can throw in private-browsing modes; ignore.
      }
    }, PERSIST_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [state.panelWidths]);

  return (
    <InboxContext.Provider value={{ state, dispatch }}>
      {children}
    </InboxContext.Provider>
  );
}
```

- [ ] **Step 2: Typecheck**

```bash
cd desktop
npx tsc --noEmit
cd ..
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add desktop/src/state/InboxContext.tsx
git commit -m "feat(desktop): mount, filter, poll, and persistence effects

Four useEffects inside InboxProvider:
1. Mount: list_channels.
2. Filter change: list_messages(filter, 50, 0) + sidebar_counts.
3. Poll (15s + window-focus): same as (2) but silent on error.
4. Panel-width persistence: debounced 200ms write to localStorage.

filterRef keeps the polling callback reading the current filter
without forcing a re-subscribe when the filter changes (the
filter-change effect already handled the immediate refetch)."
```

---

### Task 15: Frontend — CSS rewrite

**Files:**
- Modify: `desktop/src/App.css`

`★ Why this matters:` The app renders and fetches data at this point — but without the grid, sidebar, and selection styling, it looks broken. This task is mostly mechanical but required for the manual verification pass to succeed.

- [ ] **Step 1: Replace `App.css` entirely**

Overwrite `desktop/src/App.css` with:

```css
* { box-sizing: border-box; }
body {
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: #f7f8fa;
  color: #1a1f2b;
}

:root {
  --accent: #3f5ab5;
  --accent-tint: #eaefff;
  --bg: #f7f8fa;
  --surface: #ffffff;
  --border: #e4e7ec;
  --text-secondary: #667085;
  --text-muted: #94a3b8;
  --danger-bg: #fff1f3;
  --danger-border: #ffd3d8;
  --danger-fg: #a4203a;
}

.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
}

.error-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--danger-bg);
  color: var(--danger-fg);
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--danger-border);
  font-size: 0.85rem;
}
.error-dismiss {
  background: transparent;
  border: 0;
  color: var(--danger-fg);
  font-size: 1.1rem;
  cursor: pointer;
  line-height: 1;
  padding: 0 6px;
}

/* ── Three-column grid ────────────────────────────────────────────────── */

.inbox-grid {
  display: grid;
  grid-template-rows: 100%;
  flex: 1;
  min-height: 0;
  /* grid-template-columns set inline from state.panelWidths */
}

.split-handle {
  background: var(--border);
  cursor: col-resize;
}
.split-handle:hover {
  background: var(--accent);
}

/* ── Sidebar ──────────────────────────────────────────────────────────── */

.sidebar {
  background: var(--surface);
  border-right: 1px solid var(--border);
  overflow-y: auto;
  padding: 0.5rem 0;
}
.sidebar-section-label {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  padding: 0.6rem 0.75rem 0.25rem;
}
.sidebar-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 5px 10px;
  margin: 1px 6px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.9rem;
  user-select: none;
}
.sidebar-item:hover { background: #f1f3f7; }
.sidebar-item.active {
  background: var(--accent-tint);
  color: var(--accent);
  font-weight: 600;
}
.sidebar-item.disabled { color: var(--text-muted); }
.sidebar-counts {
  display: flex;
  gap: 6px;
  align-items: center;
  font-size: 0.78rem;
}
.sidebar-unread {
  background: var(--accent);
  color: #fff;
  border-radius: 8px;
  padding: 0 6px;
  min-width: 18px;
  text-align: center;
  font-weight: 600;
}
.sidebar-total { color: var(--text-muted); }
.sidebar-item.active .sidebar-total { color: var(--accent); }
.sidebar-empty {
  padding: 6px 12px;
  color: var(--text-muted);
  font-size: 0.85rem;
  font-style: italic;
}

/* ── Message list ─────────────────────────────────────────────────────── */

.message-list {
  background: var(--bg);
  overflow-y: auto;
  outline: none;
}
.message-list:focus-visible {
  box-shadow: inset 0 0 0 2px var(--accent);
}
.message-row {
  padding: 0.7rem 1rem;
  border-bottom: 1px solid #eef0f4;
  cursor: pointer;
  background: var(--surface);
}
.message-row:hover { background: #fbfcfd; }
.message-row.selected,
.message-row.selected:hover {
  background: var(--accent-tint);
}
.message-row.unread .row-subject { font-weight: 600; }
.row-main {
  display: flex;
  gap: 0.8rem;
  font-size: 0.8rem;
  color: var(--text-secondary);
  margin-bottom: 2px;
}
.row-main .time { width: 60px; flex: 0 0 60px; }
.row-main .channel { color: var(--accent); }
.row-main .sender { color: #1a1f2b; font-weight: 500; }
.row-subject { font-size: 0.95rem; }
.row-preview {
  font-size: 0.85rem;
  color: var(--text-secondary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.row-meta {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-top: 2px;
}
.load-more {
  display: block;
  margin: 0.75rem auto 1.25rem;
  padding: 6px 16px;
  cursor: pointer;
  border: 1px solid var(--border);
  background: var(--surface);
  border-radius: 4px;
}
.empty {
  padding: 3rem 1rem;
  text-align: center;
  color: var(--text-secondary);
}

/* ── Detail pane ──────────────────────────────────────────────────────── */

.detail-pane {
  background: var(--surface);
  overflow-y: auto;
  padding: 1.25rem 1.5rem;
}
.detail-pane.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
}
.detail-head {
  display: flex;
  gap: 0.8rem;
  color: var(--text-secondary);
  font-size: 0.85rem;
  margin-bottom: 0.3rem;
}
.detail-head .channel { color: var(--accent); }
.detail-subject { margin: 0 0 0.3rem; font-size: 1.25rem; }
.detail-meta {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin-bottom: 1.2rem;
}
.detail-body {
  white-space: pre-wrap;
  word-wrap: break-word;
  font-family: inherit;
  font-size: 0.95rem;
  line-height: 1.5;
  background: var(--bg);
  padding: 1rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  margin: 0;
}
.detail-attachments {
  margin-top: 1rem;
  font-size: 0.85rem;
}
```

- [ ] **Step 2: Launch + smoke-test**

```bash
cd desktop
npm run tauri dev
```

Expected: window opens with three columns. Sidebar renders "Views" (All/Unread/Priority) and "Channels" (or "No channels"). If the DB has messages, they load into the middle panel; clicking opens them into the right pane and the unread count in the sidebar decrements when an unread message is opened.

If the window loads but messages don't show, inspect stderr in the terminal running `tauri dev`; the error banner should also display a command error.

Close the window to stop the dev server.

- [ ] **Step 3: Commit**

```bash
cd ..
git add desktop/src/App.css
git commit -m "feat(desktop): three-column grid layout + sidebar/list/detail CSS

CSS Grid for the three columns, driven by inline template-columns
from App.tsx. Selected row keeps a persistent accent-tint bg.
Focus-visible ring on the message list so keyboard nav discoverable.
Unread badge uses accent fill; sidebar active item uses accent-tint."
```

---

### Task 16: Manual verification + merge

**Files:** (none — verification + merge)

`★ Why this matters:` 7b.2's value is UX. The automated tests cover Rust correctness; only the manual matrix covers whether the three-panel experience actually works.

- [ ] **Step 1: Full sweep build**

```bash
cargo build --workspace
cargo test -p messagehub-core
cargo test -p messagehub-desktop --lib
cd desktop && npm run build && cd ..
```

Expected: all green. `npm run build` runs `tsc --noEmit && vite build` and must produce zero TS errors.

- [ ] **Step 2: Run the manual verification matrix**

Start `runtime-demo` in one terminal to populate the DB; in another, `cd desktop && npm run tauri dev`.

Step through these checks in order:

1. Window opens with three columns: sidebar · list · "Select a message." placeholder in detail.
2. Sidebar shows "Views" (All / Unread / Priority) with totals. "Channels" lists one row per configured channel (or "No channels").
3. Cross-check `counts.all` against `sqlite3 core/messagehub.db "SELECT COUNT(*) FROM messages"` (inside the decrypted DB — if SQLCipher-protected, skip this and trust visually).
4. Click a channel row — middle panel reloads scoped to that channel; counts stay consistent.
5. Click **Unread** — only unread rows. Open one — row's subject becomes non-bold, Unread count decrements by 1, the opened row disappears from the Unread view.
6. Click **Priority** — only rows with priority ≥ 4 appear (cross-check with `SELECT COUNT(*) FROM messages WHERE priority_score >= 4`).
7. Keyboard: click inside the list so it has focus, press `↓` — selection moves down, detail updates. `↑` reverses. `Enter` is a no-op on the selected row. `Esc` clears the selection and closes detail.
8. Drag the sidebar↔list divider — sidebar resizes, clamps at min 160 / max ~ window-(list+detail+handles). Drag list↔detail — same for list.
9. Close the window, relaunch (`npm run tauri dev`) — panel widths restored.
10. In DevTools / browser console inside the Tauri app (right-click → Inspect), run `localStorage.removeItem("messagehub.desktop.panelWidths.v1")`, refresh — defaults apply (200 / 360).
11. Let the app idle 15 s while `runtime-demo` ingests at least one new message — the row appears in the middle panel and the sidebar counts update, with no user interaction. No visible spinner.
12. Focus away from the Tauri window, wait for `runtime-demo` to ingest, re-focus — new rows appear within ~1 s of focusing back.
13. Kill `runtime-demo`. In the filesystem, rename `core/messagehub.toml` to `core/messagehub.toml.bak`. Relaunch the Tauri app — window opens, banner shows a command error, sidebar renders empty (no channels, counts null). Restore the toml after testing.
14. Resize the window below the combined min widths — panels clamp; the detail pane absorbs the loss.

If any check fails, file the failure as a backlog item (`B-NNN`) in `docs/backlog.md` and either fix before merging or explicitly call it out in the merge commit message.

- [ ] **Step 3: Final commit (if any fixes)**

If Step 2 revealed fixable issues, fix them, run Step 1 again, and commit with `fix(desktop): …` messages. If no fixes were needed, skip this step.

- [ ] **Step 4: Merge to master**

```bash
git checkout master
git merge --no-ff feat/tauri-threepane -m "Merge branch 'feat/tauri-threepane': Plan 7b.2 — three-panel layout

Evolves the desktop app from a flat list + detail (7b.1) to a true
three-panel inbox: sidebar with Views (All/Unread/Priority) +
Channels with live counts, middle message list scoped by sidebar
selection, right detail pane with optimistic mark-read.

Adds the first write command (mark_read), the first polling loop
(15s + window-focus), and draggable + persisted panel widths. State
centralizes behind InboxContext (useReducer + Context) so 7b.3's
reply composer and 7b.4's channel CRUD slot in as additional
actions and commands, not rewrites.

Core gains MessageFilter + count_messages. No new npm deps."
```

- [ ] **Step 5: Push (optional)**

If the user wants master pushed:

```bash
git push origin master
```

---

## Notes for the executor

- **Do not deviate from the filter thresholds.** `PriorityHigh` = `min_priority: Some(4)` lives in exactly one place (`Filter::to_core` in `commands.rs`). If a reviewer wants to change it, one-line edit.
- **Do not batch commits.** Each task commits on its own so a broken step is easy to bisect. Writing-plans discipline: TDD red → green → commit, then the next step.
- **If TypeScript complains between tasks**, it's most likely because `App.tsx` still references the old single-arg `listMessages` before Task 13. The Task 7 commit message even warns about this. Don't "fix" it in an earlier task; Task 13 cleans it up.
- **If `MessageFilter::default()` fails to compile** in Task 2, confirm the `#[derive(Default)]` is on the struct. Every field has a `Default` impl already (`Option` → `None`, `bool` → `false`).
- **If `npm run tauri dev` launches with an empty sidebar**, `AppState` init probably failed — check stderr. Look for `messagehub-desktop: …`. The spec calls this out as an intentional error path (banner + empty sidebar).
- **Don't add new npm deps.** The spec and plan commit to hand-rolled SplitPane + Context. Introducing `react-resizable-panels` or `zustand` now is a scope violation.
- **If `list_channel_configs` returns multiple configs per Channel variant** (e.g., two Email accounts), the `sidebar_counts` command rolls them up to one row per variant. This is intentional for 7b.2; multi-account UI is 7b.4+.
- **`bumpCounts` only adjusts `unread` totals** (overall + per-channel). `all` and `priorityHigh` are kept as-is because mark-read doesn't change them. If a later plan adds archive/delete, extend `bumpCounts` (or introduce a sibling helper) to cover those totals.
