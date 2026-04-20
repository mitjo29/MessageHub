# Plan 7b.2 — Three-Panel Inbox Layout — Design Specification

**Date:** 2026-04-20
**Status:** Approved
**Author:** Jocelyn Moreau + Claude
**Depends on:** Plan 7b.1 merged on master (commit `9fd95b6` or later).

## Overview

Plan 7b.2 evolves the desktop app from "one flat scrollable list with
click-to-expand body" (7b.1) into the three-panel inbox envisioned by the
master design spec: a left sidebar of **views and channels**, a middle
**message list** scoped by the sidebar selection, and a right **detail pane**
that stays visible alongside the list.

7b.2 also introduces the app's **first write operation** — marking a message
read when it is opened — and **the first auto-refresh loop** (15-second poll
plus window-focus trigger). Everything else remains read-only; the reply
composer is 7b.3.

**What it is:** the first usable-as-a-daily-driver slice of the desktop UI.
~600–800 LOC of real code on top of the 7b.1 scaffolding.

**What it is not:** a finished app. No reply composer, no channel management,
no keychain, no search, no thread grouping, no attachment downloads, no
archive/delete/star, no theming. Those are later plans.

## Goals

1. On launch, a three-panel layout renders: `[sidebar] | [message list] | [detail pane]`.
2. The sidebar lists two sections — **Views** (All, Unread, Priority) and
   **Channels** (one row per configured channel) — each with a live count.
3. Clicking a sidebar item scopes the middle panel to messages matching that
   filter and refreshes counts.
4. Clicking a message in the middle panel loads its detail into the right
   pane. If the message was unread, it is marked read (DB write) and the
   unread counts update.
5. The app polls for new messages every 15 s and when the window regains focus,
   without disturbing the user's selection or scroll position.
6. Panels are user-resizable by dragging dividers; widths survive restarts via
   `localStorage`.
7. Keyboard navigation: `↑` / `↓` move the selection in the middle panel,
   `Enter` opens the selected message, `Esc` clears the selection and detail.
8. State is owned by a single `useReducer` at the context-provider level so
   7b.3's composer (and later channel CRUD in 7b.4) can slot in without
   reshaping the data flow.

## Non-Goals

- **No reply composer** — sending, drafting, attachments-on-send (7b.3).
- **No channel CRUD** — add/remove/edit a channel from the UI (7b.4).
- **No keychain / credential UI** (7b.5).
- **No archive, delete, star, label, snooze.** Only `mark_read` in 7b.2.
- **No thread grouping.** Middle panel shows one row per message, same row
  format as 7b.1 (timestamp · channel · sender · subject · preview · category
  · priority). `thread_id` remains an implementation detail exposed only in
  `MessageDetail`.
- **No search box** — client-side or FTS. Sidebar selection is the only
  filter.
- **No Gmail-style shortcuts** (`j`/`k`/`r`/`a`). Only `↑`/`↓`/`Enter`/`Esc`.
- **No infinite scroll.** Keep the "Load more" button from 7b.1.
- **No attachment download or preview.** Attachment list stays read-only.
- **No dark mode / theming.** Extend the 7b.1 plain-CSS style.
- **No new npm dependencies.** Reducer + Context + hand-rolled SplitPane only.
- **No auto-generated TS types** (e.g. `ts-rs`). Continue hand-writing DTOs.
  The API surface grows from 4 to 6 commands — still manageable.
- **No event streaming / `tauri://event`.** Refresh is poll-based; revisit
  when the Runtime runs inside the app (a later plan, not 7b.x).

## File Changes

```
MessageHub/
├── Cargo.toml                                   unchanged
├── core/
│   └── src/
│       └── store/
│           ├── messages.rs                      MODIFY: MessageFilter + new list_messages signature + count_messages
│           └── mod.rs                           MODIFY: re-export MessageFilter if public
├── core/src/runtime/classifier_worker.rs       MODIFY: migrate call-site to new list_messages signature
├── core/src/bin/runtime-demo/*                 MODIFY: migrate call-site(s) only
└── desktop/
    ├── src-tauri/
    │   └── src/
    │       └── commands.rs                      MODIFY: extend list_messages; add mark_read + sidebar_counts + DTOs
    └── src/
        ├── api.ts                               MODIFY: wire filter arg on listMessages; add markRead + sidebarCounts
        ├── types.ts                             MODIFY: add Filter, SidebarCounts, ChannelCount types
        ├── App.tsx                              REWRITE: ~60 LOC — renders <InboxProvider><InboxLayout/></InboxProvider>
        ├── App.css                              REWRITE: ~200 LOC — three-column grid + sidebar/list/detail styling
        ├── state/
        │   └── InboxContext.tsx                 CREATE: reducer, provider, useInbox() hook, side-effect hooks
        └── components/
            ├── Sidebar.tsx                      CREATE: views + channels
            ├── SplitPane.tsx                    CREATE: hand-rolled resize handle
            ├── MessageList.tsx                  CREATE: rows + pagination + keyboard nav
            └── MessageDetail.tsx                CREATE: detail pane + mark-read side effect
```

No Cargo or package-lock changes. No new crate members. No new npm packages.

## Core Changes

`core/src/store/messages.rs` grows a filter shape and two methods.

### `MessageFilter`

```rust
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    pub channel: Option<Channel>,
    pub unread_only: bool,
    pub min_priority: Option<u8>,     // inclusive floor; None = any priority
    pub archived: bool,               // keep 7b.1 semantics — default false
}
```

All fields default to "don't constrain" so `MessageFilter::default()` equals
the existing `list_messages(None, false, _, _)` call.

### Signature change to `Store::list_messages`

```rust
pub fn list_messages(
    &self,
    filter: &MessageFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<Message>>
```

SQL assembly stays essentially the same — additional clauses append when the
filter fields are set. Ordering and paging unchanged.

### New `Store::count_messages`

```rust
pub fn count_messages(&self, filter: &MessageFilter) -> Result<u64>
```

Same WHERE clauses as `list_messages`, returning `COUNT(*)`. Used exclusively
by `sidebar_counts`.

### Migration of existing call-sites

Two (or three, if the demo uses it more than once) trivial one-liner
migrations. No behavior changes.

### `mark_read` is already present

`Store::mark_read(id: &Uuid, read: bool) -> Result<()>` already exists from
plan 1 and is used in tests. 7b.2 only adds a Tauri command that calls it.

## Tauri Application

### Managed state

The `AppState` struct from 7b.1 stays. No new fields — counts are re-queried
on every `sidebar_counts()` call, not cached.

### New / changed commands

`desktop/src-tauri/src/commands.rs` ends with **six** `#[tauri::command]`s:

1. **`list_messages(filter: Filter, limit: u32, offset: u32) -> Result<Vec<MessageRow>, String>`**
   Breaking change from 7b.1 (the `filter` parameter is new and non-optional).
   The Tauri-side `Filter` is a tagged enum mirroring the TS type; it is
   decoded via `serde::Deserialize` and converted into a `MessageFilter`
   before calling `Store::list_messages`.

2. **`get_message(id: String) -> Result<MessageDetail, String>`** — unchanged.

3. **`list_channels() -> Result<Vec<ChannelInfo>, String>`** — unchanged.

4. **`get_config() -> Result<UiConfig, String>`** — unchanged.

5. **`mark_read(id: String, read: bool) -> Result<(), String>`** — new.
   Parses the UUID, calls `Store::mark_read`, maps errors to strings.

6. **`sidebar_counts() -> Result<SidebarCounts, String>`** — new. Returns one
   struct containing all view counts plus per-channel totals; single
   round-trip.

### DTOs added

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Filter {
    All,
    Unread,
    PriorityHigh,                     // maps to min_priority = Some(4)
    Channel { channel_type: String }, // decoded to Channel enum
}

#[derive(serde::Serialize)]
pub struct ChannelCount {
    pub channel_type: String,
    pub total: u64,
    pub unread: u64,
}

#[derive(serde::Serialize)]
pub struct SidebarCounts {
    pub all: u64,
    pub unread: u64,
    pub priority_high: u64,
    pub by_channel: Vec<ChannelCount>,
}
```

Existing DTOs (`MessageRow`, `MessageDetail`, `AttachmentInfo`, `ChannelInfo`,
`UiConfig`) are unchanged.

### Filter → MessageFilter conversion

```rust
impl Filter {
    fn to_core(&self) -> Result<MessageFilter, String> {
        Ok(match self {
            Filter::All          => MessageFilter::default(),
            Filter::Unread       => MessageFilter { unread_only: true, ..Default::default() },
            Filter::PriorityHigh => MessageFilter { min_priority: Some(4), ..Default::default() },
            Filter::Channel { channel_type } => {
                let ch = Channel::from_db_str(channel_type)
                    .ok_or_else(|| format!("unknown channel_type: {}", channel_type))?;
                MessageFilter { channel: Some(ch), ..Default::default() }
            }
        })
    }
}
```

`Channel::from_db_str` already exists in `core/src/types/channel.rs`. The
`min_priority = 4` threshold lives in **this one place**; the TS side never
encodes the number 4.

## Frontend

### Component tree

```
App
└── InboxProvider                              (Context + reducer)
    └── InboxLayout                            (three-column grid)
        ├── Sidebar                            (views + channels + counts)
        ├── SplitPane handle                   (sidebar ↔ list)
        ├── MessageList                        (rows, pagination, keyboard nav)
        ├── SplitPane handle                   (list ↔ detail)
        └── MessageDetail                      (detail pane + mark-read effect)
```

`InboxLayout` owns the grid CSS and renders the banner for `state.error`.
`App.tsx` itself collapses to ~60 LOC — mostly `<InboxProvider><InboxLayout/></InboxProvider>`.

### State shape

In `state/InboxContext.tsx`:

```typescript
export type Filter =
  | { kind: "all" }
  | { kind: "unread" }
  | { kind: "priorityHigh" }
  | { kind: "channel"; channelType: string };

export type InboxState = {
  filter: Filter;
  channels: ChannelInfo[];
  counts: SidebarCounts | null;
  messages: MessageRow[];
  hasMore: boolean;
  selectedId: string | null;
  detail: MessageDetail | null;
  panelWidths: { sidebar: number; list: number };  // right pane fills remainder
  error: string | null;
  loading: boolean;
};

export type InboxAction =
  | { type: "SET_FILTER";            filter: Filter }
  | { type: "SET_CHANNELS";          channels: ChannelInfo[] }
  | { type: "SET_COUNTS";            counts: SidebarCounts }
  | { type: "LOAD_MESSAGES_SUCCESS"; messages: MessageRow[]; append: boolean; hasMore: boolean }
  | { type: "SELECT";                id: string | null }
  | { type: "LOAD_DETAIL_SUCCESS";   detail: MessageDetail }
  | { type: "MARK_READ_LOCAL";       id: string }
  | { type: "REVERT_MARK_READ_LOCAL"; id: string }
  | { type: "SET_PANEL_WIDTHS";      widths: { sidebar: number; list: number } }
  | { type: "SET_ERROR";             error: string | null }
  | { type: "SET_LOADING";           loading: boolean };
```

`SET_FILTER` clears `messages`, `selectedId`, `detail`, and resets paging. The
side-effect hook listens for `filter` changes and re-fetches page 0.

`MARK_READ_LOCAL` performs an optimistic update: flips `is_read` on the row
and, if the filter is `unread`, removes the row from the visible list;
decrements the relevant counts. If the Tauri `mark_read` command fails, the
detail-pane effect dispatches `REVERT_MARK_READ_LOCAL` with the same id —
this action re-inserts the row if it was removed, flips `is_read` back, and
re-increments the affected counts — then raises `SET_ERROR`.

### Side-effect hooks

In `state/InboxContext.tsx`, three `useEffect`s colocated with the provider:

- **Mount:** parallel fetch of `list_channels`, `sidebar_counts`, `list_messages(filter, 50, 0)`.
- **Filter change:** fetch `list_messages(filter, 50, 0)` + refresh counts.
- **Poll:** `setInterval(tick, 15_000)` + `visibilitychange` listener. `tick()`
  re-fetches counts + page 0 only. Preserves `selectedId` and `detail`.

`loadMore` is a callback the list component can call to fetch page N.

### Sidebar.tsx

Renders two sections: "Views" (fixed 3 items) and "Channels" (from
`state.channels`). Each row:

```
<div class="sidebar-item" aria-selected={active} onClick={() => setFilter(f)}>
  <span class="label">All</span>
  <span class="count">42</span>
</div>
```

Unread counts render as a bold-bordered badge next to the dimmer total count
when `unread > 0`. A view with `total === 0` renders `—` instead of `0` to
match the standard-email visual vocabulary.

Exact rules:
- Exactly one sidebar item is selected at any time (radio-group semantics).
  Clicking the currently-selected item is a no-op.
- Disabled channels (`enabled === false`) render greyed out but remain
  clickable — users may want to inspect historical data from a disabled
  channel.
- If `state.channels` is empty, the "Channels" section renders a muted "No
  channels" placeholder.

### MessageList.tsx

Row rendering identical to 7b.1 (time · channel · sender · subject · preview
· category · priority). One new CSS state: `.message-row.selected` receives a
persistent accent-tinted background so the current selection stays visible
even when the list scrolls.

Selected row is tracked in `state.selectedId`. Click selects + triggers
`LOAD_DETAIL_SUCCESS`. "Load more" button is unchanged from 7b.1 and only
appears while `state.hasMore`.

Keyboard handling — the list is a focusable container:

```typescript
<div class="message-list" tabIndex={0} onKeyDown={handleKey}>
  {rows.map(row => <Row ... />)}
</div>
```

`handleKey` dispatches:
- `ArrowUp` / `ArrowDown`: moves `selectedId` by ±1 in the current `messages`
  array. Stops at the boundaries (no wrap). Also triggers the detail fetch
  (same as a click) so nav feels live.
- `Enter`: no-op if already selected (the click handler already opened it);
  otherwise opens the current selection.
- `Escape`: dispatches `SELECT(null)` and clears `detail`.

Scrolling the selected row into view is best-effort via `element.scrollIntoView({ block: "nearest" })`.

Empty-state string: `"No messages in this view."` Different from 7b.1's
`"runtime-demo"` hint — the DB may well have messages; the filter just
excluded them.

### MessageDetail.tsx

Layout identical to 7b.1's `DetailView` minus the Back button (detail is
always visible in the right pane now). Empty state when `state.detail === null`:
centered placeholder `"Select a message."`.

The mark-read side-effect lives in `MessageDetail`:

```typescript
useEffect(() => {
  if (!detail) return;
  if (detail.is_read) return;
  dispatch({ type: "MARK_READ_LOCAL", id: detail.id });         // optimistic
  api.markRead(detail.id, true).catch((err) => {
    dispatch({ type: "REVERT_MARK_READ_LOCAL", id: detail.id }); // roll back
    dispatch({ type: "SET_ERROR", error: String(err) });
  });
}, [detail?.id]);
```

### SplitPane.tsx

A 6-pixel draggable handle rendered as a grid cell:

```
grid-template-columns: ${sidebar}px 6px ${list}px 6px 1fr;
```

On `mousedown`, the handle registers global `mousemove` / `mouseup` listeners
on `document`, applies the delta to its target panel, and clamps within
min/max:

| Panel    | Min | Max                                   | Default |
|----------|-----|---------------------------------------|---------|
| sidebar  | 160 | window.innerWidth - (260 + 320 + 12)  | 200     |
| list     | 260 | window.innerWidth - (sidebar + 320 + 12) | 360  |

Detail takes the remainder. The 12px term is the two handles.

During drag: `body.style.cursor = "col-resize"` and `body.style.userSelect = "none"`.
Release restores both.

### Persistence of panel widths

A `useEffect` in the provider watches `state.panelWidths` (debounced 200 ms)
and writes `JSON.stringify(widths)` to
`localStorage["messagehub.desktop.panelWidths.v1"]`.

On mount, hydrate from the same key with try/catch and schema validation
(`typeof widths.sidebar === "number" && typeof widths.list === "number"`).
Any failure → fall back to defaults silently.

Versioning in the key (`v1`) lets us change the shape in a future plan by
bumping to `v2`.

### Auto-refresh

One `useEffect` in `InboxContext` owns the refresh cycle:

```typescript
useEffect(() => {
  const tick = () => {
    api.sidebarCounts().then(c => dispatch({ type: "SET_COUNTS", counts: c }));
    api.listMessages(filter, 50, 0).then(rows =>
      dispatch({ type: "LOAD_MESSAGES_SUCCESS", messages: rows, append: false, hasMore: rows.length === 50 })
    );
  };
  const id = setInterval(tick, 15_000);
  const onVis = () => { if (document.visibilityState === "visible") tick(); };
  document.addEventListener("visibilitychange", onVis);
  return () => { clearInterval(id); document.removeEventListener("visibilitychange", onVis); };
}, [filter]);
```

Refetches only page 0. Older pages the user has already loaded stay in state
— if the DB advanced enough, there's a gap. Acceptable for 7b.2; surfaces as
a "Load more" button that may fetch into overlapping territory. A future
plan can dedupe by id.

Polling is silent (no spinner). Manual refresh still exists as a header
button and does show `state.loading`.

## Error Handling

All command rejections are caught in the side-effect or component that
initiated them and dispatched as `SET_ERROR`. A single red banner renders at
the top of `InboxLayout` whenever `state.error !== null`, with a dismiss
`×` that dispatches `SET_ERROR(null)`.

Any successful fetch clears the error automatically (first dispatch in the
success handler sets `error: null`).

The init-failure path from 7b.1 still works: if `AppState` never registers,
commands fail with `State<AppState>` unavailable, the banner shows "Not
configured — check messagehub.toml", and the sidebar renders empty. The
window stays open so the user can read the error.

## CSS

Extend the hand-written `App.css` from 7b.1 by ~200 LOC. No CSS-in-JS, no
Tailwind, no CSS Modules. Conventions:

- CSS Grid for the three-column layout at the `InboxLayout` level.
- Flex for the sidebar and row interiors.
- Color accents reuse the 7b.1 palette (`#3f5ab5` primary, `#f7f8fa` bg,
  `#e4e7ec` borders, `#667085` secondary text).
- Selected / active state uses a 10 % tint of the primary accent for the
  background (`#eaefff`).
- `.split-handle` is 6 px wide, transparent, `cursor: col-resize` on hover.
- `.message-list` has `overflow-y: auto` and keyboard focus styling
  (`outline: 2px solid var(--accent)` on `:focus-visible`).

## Testing

### Rust — `cargo test -p messagehub-core`

- `list_messages` with each `MessageFilter` variant returns the expected
  rows against a populated test DB.
- `count_messages` matches `list_messages(...).len()` for the same filter
  (property-style check).
- `list_messages` with `min_priority = Some(4)` excludes messages with score
  `1..=3` and `None`.
- `mark_read` round-trips: read=true then read=false, with `get_message`
  reflecting each step.

### Tauri command layer

- Small unit test for `Filter::to_core` covering each variant, including the
  `Channel::from_db_str` error path.

### Frontend

No automated tests in 7b.2 — matches 7b.1's posture. Manual verification
matrix below carries the weight.

### Manual verification matrix

Launch the app with `runtime-demo` populating the DB in a second terminal.

1. Cold start renders sidebar + empty list (if DB empty) or first page of
   messages, + "Select a message" placeholder in the right pane.
2. Counts in the sidebar match an independent SQL query against the DB.
3. Clicking the "Email · …" channel scopes the middle panel; switching to
   "SMS" shows SMS-only messages.
4. Clicking **Unread** shows only unread rows; opening one (click) decrements
   the Unread count live and the row flips from bold to normal.
5. Clicking **Priority** shows only messages with score ≥ 4; crosscheck
   with `SELECT COUNT(*) FROM messages WHERE priority_score >= 4`.
6. Keyboard nav: focus the list, use `↑`/`↓` to move, `Enter` to open,
   `Esc` to clear. No mouse touches required for the round-trip.
7. Drag the sidebar↔list divider; drag list↔detail. Close + reopen the app;
   widths persisted.
8. Clear `localStorage["messagehub.desktop.panelWidths.v1"]`; relaunch. Defaults apply.
9. Wait 15 s with the app idle; verify a new row ingested by `runtime-demo`
   appears without clicking Refresh.
10. Unfocus the window, let `runtime-demo` ingest, re-focus — new rows
    appear within ~1 s of refocus.
11. Kill `runtime-demo`, delete `messagehub.toml`, relaunch desktop app:
    banner shows "Not configured — check messagehub.toml"; sidebar renders
    empty; window stays open.
12. Resize window below minimum widths: panels clamp to their `min-width`
    and the detail pane absorbs the loss.

## Interactions with other plans

- **7b.3 (reply composer):** the reducer already carries the selected
  message's `detail`; the composer becomes another consumer of `useInbox()`
  and adds a `send_reply` command + `SEND_REPLY_*` actions.
- **7b.4 (channel CRUD):** the sidebar already renders `state.channels`; the
  CRUD plan adds an "Add channel" affordance and `add_channel` /
  `remove_channel` commands. No layout changes expected.
- **Thread grouping** (no plan number yet): will revise `MessageFilter` and
  add a `by_thread` toggle on `list_messages`. The middle panel component
  can remain, just switching its row shape.

## Backlog discovered during design

None so far. File any findings against `docs/backlog.md` during
implementation with the usual `B-NNN` numbering.
