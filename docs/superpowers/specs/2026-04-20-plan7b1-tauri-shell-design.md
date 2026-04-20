# Plan 7b.1 — Tauri Desktop Shell (Read-Only Message List) — Design Specification

**Date:** 2026-04-20
**Status:** Approved
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plans 1–6 + 7a merged on master (commit `3dbc085` or later).

## Overview

Plan 7b.1 is the first slice of the desktop UI. A Tauri 2 application with a
React/TypeScript frontend that opens a populated SQLCipher database, queries it
read-only via Rust `#[command]`s, and displays messages in a single flat list.
The user runs `runtime-demo` in a separate terminal to keep the DB updated; the
Tauri app re-fetches on demand.

**What it is:** the "can I see my email in a window that isn't a terminal"
milestone. ~500-700 LOC of scaffolding + ~300 LOC of real code.

**What it is not:** a usable daily driver. No three-panel layout, no reading
pane, no reply composer, no channel management, no Runtime in-app, no events
streaming. Those are 7b.2–7b.5.

## Goals

1. `cd desktop && npm run tauri dev` launches a desktop window showing messages
   from `../core/messagehub.db`.
2. Messages render in a single scrollable list with: timestamp, channel,
   sender display name, subject (or body preview), category, priority.
3. Clicking a message expands to show its full body inline — no router, no
   modal, no push navigation. A simple "Back to list" control collapses it.
4. A "Refresh" button re-runs the DB query. Paging via "Load more" at the
   bottom of the list.
5. The bridge pattern is solid enough to extend cleanly in 7b.2 — the Rust
   commands, the TS API surface, and the type definitions all have a good
   shape.

## Non-Goals

- No Runtime instance inside the Tauri app. Tauri is strictly a DB viewer.
- No write operations (reply, archive, mark-read). Purely read-only in 7b.1.
- No three-panel layout, no design system, no theming. Plain CSS, flex layout,
  one component tree.
- No event streaming, no WebSocket, no polling. Refresh is manual.
- No OAuth, no keychain, no credential UI. DB file + password come from
  `messagehub.toml` just like `runtime-demo`.
- No packaging / installer / code signing. `tauri dev` only.
- No auto-generated TS types (e.g., `ts-rs`). Hand-written for 7b.1; revisit
  in 7b.2+ if the API surface grows.
- No Windows/macOS-specific tuning. Linux-first (matches the user's env);
  should also launch on macOS via `webkit2gtk` equivalent; Windows deferred.

## Workspace Restructure

```
MessageHub/
├── Cargo.toml                    # MODIFY: add `desktop/src-tauri` as workspace member
├── core/                         # unchanged
└── desktop/                      # CREATE
    ├── .gitignore                # CREATE: node_modules, dist, src-tauri/target
    ├── package.json              # CREATE: React + Vite + Tauri deps
    ├── vite.config.ts            # CREATE: Vite config for Tauri
    ├── tsconfig.json             # CREATE
    ├── tsconfig.node.json        # CREATE
    ├── index.html                # CREATE
    ├── src/                      # CREATE
    │   ├── main.tsx
    │   ├── App.tsx
    │   ├── App.css
    │   ├── api.ts                # thin wrapper around invoke()
    │   └── types.ts              # hand-written TS types
    └── src-tauri/                # CREATE
        ├── Cargo.toml            # depends on messagehub-core via path
        ├── build.rs
        ├── tauri.conf.json       # app name, window config
        ├── capabilities/
        │   └── default.json      # Tauri 2 capability model
        ├── icons/                # PLACEHOLDER png set
        └── src/
            ├── main.rs           # entry
            └── commands.rs       # #[command]s
```

The `core` crate is **not modified** by this plan. 7b.1 is a pure consumer.

## Tauri Application

### Runtime stack

- **Tauri 2.x** (latest stable on the `tauri@2` tag).
- Linux backend: `webkit2gtk-4.1` (matches Ubuntu 24.04, Fedora 40+). No action
  needed beyond installing system packages on the dev machine.
- No Tauri plugins in 7b.1. Just the core runtime.

### Managed state

The Tauri app's main thread constructs:

```rust
struct AppState {
    store: Arc<Mutex<Store>>,
    label_by_channel_id: HashMap<Uuid, String>,
}
```

built once at startup from the `messagehub.toml` config and stashed via
`tauri::Builder::manage(state)`. Commands access it via
`tauri::State<'_, AppState>`.

### Rust commands (the TS API surface)

Four `#[tauri::command] async fn`s in `src-tauri/src/commands.rs`:

1. **`list_messages(limit: u32, offset: u32) -> Result<Vec<MessageRow>, String>`**
   Returns a flat list sorted by `timestamp DESC`. Each `MessageRow` is:

   ```rust
   #[derive(serde::Serialize)]
   struct MessageRow {
       id: String,                 // UUID as string
       timestamp: String,          // RFC3339
       channel: String,            // "Email" / "Telegram" / etc.
       channel_label: Option<String>, // human-readable, from channels table
       sender_name: String,        // from contacts table
       subject: Option<String>,
       preview: String,            // first 140 chars of body
       category: Option<String>,
       priority: Option<u8>,       // 1-5 if classified
       is_read: bool,
   }
   ```

2. **`get_message(id: String) -> Result<MessageDetail, String>`**
   Full body view. `MessageDetail` includes everything `MessageRow` has plus:

   ```rust
   struct MessageDetail {
       #[serde(flatten)]
       row: MessageRow,
       body: String,                       // full content_text
       html: Option<String>,               // content_html, if any
       thread_id: String,
       attachments: Vec<AttachmentInfo>,   // filename + size
   }
   ```

3. **`list_channels() -> Result<Vec<ChannelInfo>, String>`**
   ```rust
   struct ChannelInfo {
       id: String,
       channel_type: String,
       label: String,
       enabled: bool,
       status: String,          // "Healthy" | "Degraded" | "Failed"
       last_sync_at: Option<String>,
   }
   ```

4. **`get_config() -> Result<UiConfig, String>`**
   Tiny helper returning `{ db_path: String, channel_count: usize }` so the UI
   can display a little status footer.

### Command error handling

All commands return `Result<T, String>`. Rust errors are mapped via
`.map_err(|e| e.to_string())`. This is Tauri-idiomatic for 7b.1's narrow
surface. 7b.2+ may introduce a richer error enum serialized to TS.

### Config loading

Tauri reads its config from `./messagehub.toml` (working directory when
`tauri dev` runs, which is `desktop/`). We deliberately **reuse the same
schema** as `runtime-demo`, so a single TOML file can be pointed at by both
processes:

```toml
database = "../core/messagehub.db"
password = "..."
# [ai] and [[channels]] sections are ignored by the Tauri app in 7b.1.
```

If the file is missing or malformed, the Tauri app window shows an error
screen (not a crash) with a message pointing to the expected path.

### No Runtime

The Tauri app does NOT start a `Runtime`. It uses only:

- `Store::open(path, password)` — read path
- `Store::list_messages`, `Store::get_message`, `Store::list_channel_configs`,
  `Store::get_contact` — read-only queries

Write operations, polling, classification — all out of scope.

## React Frontend

### Build stack

- **Vite 5** with the React TypeScript template.
- **React 18** (not 19 — 19 has some Tauri-side edge cases still settling as
  of the spec's write-up date).
- **TypeScript 5**.
- No state manager (React's built-in `useState` + `useEffect` are plenty).
- Plain CSS. A single `App.css` file. No Tailwind yet.

### Components (entire tree)

```
<App>
  <Header>                          // "MessageHub — db: ./messagehub.db • 2 channels"
  <MessageList messages>
    <MessageRow message />          // one per message
  </MessageList>
  <MessageDetail message | null />  // only rendered when a row is clicked
  <Footer>                          // error bar if set
</App>
```

Four files under `src/`:

- `App.tsx` — top-level state, fetches messages, passes to children.
- `types.ts` — hand-written TS types mirroring the Rust DTOs.
- `api.ts` — thin wrapper over `@tauri-apps/api/core::invoke`. Typed.
- `App.css` — ~100 lines. Layout is a vertical flex: header, scrolling list,
  footer.

### Interactions

- On mount: `App` calls `api.listMessages(50, 0)` and `api.listChannels()`.
  Both populate state.
- Row click → toggles `selectedMessageId`. If non-null, render `MessageDetail`
  above the list (full-screen modal-ish) with a "Back" button that clears the
  selection.
- "Refresh" in the header → re-runs `listMessages(50, 0)`.
- "Load more" at the bottom → `listMessages(50, offset)` and appends.
- "Mark as read" and similar write operations are **not** in 7b.1.

### Type contracts

`types.ts` mirrors the Rust DTOs by hand:

```typescript
export type MessageRow = {
  id: string;
  timestamp: string;
  channel: string;
  channel_label: string | null;
  sender_name: string;
  subject: string | null;
  preview: string;
  category: string | null;
  priority: number | null;
  is_read: boolean;
};
```

Same for `MessageDetail`, `ChannelInfo`, `UiConfig`. ~40 lines total.

### Styling

Rough visual:

```
┌─────────────────────────────────────────────────────┐
│ MessageHub                      [Refresh] db: ...db │
├─────────────────────────────────────────────────────┤
│ ● 14:32  [Email]    Alice Smith                     │
│         Invoice #42 — please find attached…        │
│         Finance • P3                                │
├─────────────────────────────────────────────────────┤
│   13:55  [Telegram] @team_updates                   │
│         Deploy looks good, ready to merge…         │
│         Work • P4                                   │
├─────────────────────────────────────────────────────┤
│         [Load more]                                 │
└─────────────────────────────────────────────────────┘
```

Unread rows get a leading bullet. Channel name in brackets; priority as
`P<n>`. Body text-wraps at ~3 lines with `-webkit-line-clamp`.

## Data Flow

```
┌──────────────┐     IPC        ┌───────────────┐     SQL
│ React        │ ──────────────▶│ Rust command  │ ──────▶ SQLCipher DB
│ (browser)    │                │ #[command]    │ ◀──────
│              │ ◀──────────────│               │     rows
└──────────────┘   JSON DTOs    └───────────────┘
```

Every command hit serializes to JSON. ~50 messages per page; each row ~500
bytes → ~25KB per refresh. Fine.

## Error Handling

| Failure | Behavior |
|---|---|
| `messagehub.toml` missing | Tauri app window boots to an error screen with the expected path and a hint to copy from the `core/` example. |
| Bad DB password / corrupt DB | Error screen with the message. User must close and fix. |
| Command returns `Err(String)` | Frontend sets a top-of-screen error banner; list rendering continues to work with stale data. |
| Store is momentarily locked | Std `std::sync::Mutex` contention; commands queue. Given the UI is read-only and low-frequency, this is a non-issue. |
| DB doesn't exist yet (first run before `runtime-demo`) | Empty list, header shows "0 channels". User runs `runtime-demo`, clicks Refresh. |

## Testing

- **No new automated tests.** Manual verification:

  1. `cd desktop && npm install` succeeds.
  2. `cd desktop && npm run tauri dev` launches a window.
  3. Without `messagehub.toml` → error screen visible.
  4. With a valid `messagehub.toml` (reusing `core/messagehub.toml`) → messages
     render; Refresh works; clicking a row shows body; Back returns.
  5. Running `runtime-demo` in a separate terminal: new messages appear in
     the Tauri app after Refresh.

- No unit tests on the Rust side — commands are thin wrappers over existing
  Store methods that already have coverage.

- No frontend tests. 7b.2 or later adds a testing framework once the component
  tree is non-trivial.

## Out of Scope (Future Plans)

- **7b.2** — Three-panel layout + sidebar nav + reading pane + separate
  message-list component with per-channel filtering.
- **7b.3** — Reply composer + Plan 5 cloud-draft button.
- **7b.4** — Channel CRUD UI (add/edit/remove channels in-app).
- **7b.5** — Keychain integration + credentials UI.
- **Packaging/distribution** — Tauri 2 `tauri build` + AppImage/MSI/DMG bundles.
- **Auto-reload from Runtime events** — requires running the Runtime inside
  Tauri, deferred to 7b.2.

---

*Spec approved 2026-04-20.*
