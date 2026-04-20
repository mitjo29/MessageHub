# Plan 7b.1: Tauri Desktop Shell (Read-Only) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first slice of the desktop UI — a Tauri 2 app with a React/TypeScript frontend that opens the existing SQLCipher DB, queries it read-only via Rust commands, and renders a flat scrollable list of messages with click-to-expand detail. No Runtime in-app, no write operations, no three-panel layout.

**Architecture:** New workspace member `desktop/` with Tauri backend at `desktop/src-tauri/` (a Cargo crate depending on `messagehub-core` via path) and a Vite-built React frontend at `desktop/src/`. Four Rust `#[command]` async functions bridge the DB to TypeScript. The UI is one component tree under ~300 LOC. Config (DB path + SQLCipher password) reuses the existing `messagehub.toml` schema that `runtime-demo` uses.

**Tech Stack:** Tauri 2.x, Vite 5, React 18, TypeScript 5, `@tauri-apps/api` v2. Rust-side deps: `tauri`, `tauri-build`, `serde`, `serde_json`, `tokio`, `toml`, `uuid`, plus `messagehub-core` via workspace path. No new deps in `core`.

**Prerequisites:**
- Rust toolchain (already present).
- Node.js 20+ and `npm` on PATH. Verify with `node --version && npm --version`.
- System packages for Tauri 2 on Linux: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `libsoup-3.0-dev`, `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`. Ubuntu 24.04 / Fedora 40+ recommended. Verify with `pkg-config --exists webkit2gtk-4.1 && echo ok`.

**Spec:** `docs/superpowers/specs/2026-04-20-plan7b1-tauri-shell-design.md`.

---

## File Structure

```
MessageHub/
├── Cargo.toml                              # MODIFY: add workspace member
├── core/                                   # unchanged
└── desktop/                                # CREATE
    ├── .gitignore
    ├── package.json
    ├── vite.config.ts
    ├── tsconfig.json
    ├── tsconfig.node.json
    ├── index.html
    ├── src/
    │   ├── main.tsx
    │   ├── App.tsx
    │   ├── App.css
    │   ├── api.ts
    │   └── types.ts
    └── src-tauri/
        ├── Cargo.toml
        ├── build.rs
        ├── tauri.conf.json
        ├── capabilities/
        │   └── default.json
        ├── icons/
        │   ├── 32x32.png
        │   ├── 128x128.png
        │   ├── 128x128@2x.png
        │   └── icon.png
        └── src/
            ├── main.rs
            ├── config.rs
            ├── state.rs
            └── commands.rs
```

---

### Task 1: Preflight — verify toolchain + system packages

**Files:** (none — verification step)

`★ Why this matters:` Tauri 2's webkit2gtk-4.1 dependency is the #1 cause of "tauri dev fails with cryptic linker error" frustration. Confirming prerequisites now saves an hour later.

- [ ] **Step 1: Verify Rust + Node**

```bash
rustc --version   # any stable 1.75+
node --version    # 20.x or 22.x
npm --version     # 10.x
```

If any are missing or too old, install them (rustup for Rust; use your distro's package manager or a version manager like `nvm` / `fnm` for Node). Do NOT proceed until all three report versions.

- [ ] **Step 2: Verify Tauri 2 system packages on Linux**

```bash
pkg-config --exists webkit2gtk-4.1 && echo "webkit2gtk: ok"
pkg-config --exists javascriptcoregtk-4.1 && echo "jscore: ok"
pkg-config --exists libsoup-3.0 && echo "libsoup: ok"
```

All three must print `ok`. If any fails, install (Ubuntu/Debian):

```bash
sudo apt install -y libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev \
  build-essential curl wget file libxdo-dev
```

Fedora equivalent uses `dnf` with `webkit2gtk4.1-devel` etc. Do NOT proceed until all three pkg-config checks pass.

- [ ] **Step 3: Create the feature branch**

```bash
git checkout -b feat/tauri-shell
```

Confirm you're at the latest master (plan 7a and B-001/B-002 merged):

```bash
git log --oneline -3
```

---

### Task 2: Workspace Cargo manifest + `desktop/` directory stub

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `desktop/.gitignore`

`★ Why this matters:` Register the new crate before creating it so the workspace builds still compile during bootstrap.

- [ ] **Step 1: Inspect the workspace root `Cargo.toml`**

Read the existing `Cargo.toml` at repo root to see how `core` is registered. Note the `members` array under `[workspace]`.

- [ ] **Step 2: Add the new member**

Edit the root `Cargo.toml`. Inside `[workspace]`, extend `members`:

```toml
[workspace]
members = [
    "core",
    "desktop/src-tauri",
]
resolver = "2"
```

(If `resolver = "2"` isn't already there, add it. Tauri 2 requires it.)

- [ ] **Step 3: Create the desktop directory and its root `.gitignore`**

```bash
mkdir -p desktop/src desktop/src-tauri/src desktop/src-tauri/capabilities desktop/src-tauri/icons
```

Create `desktop/.gitignore`:

```
node_modules/
dist/
dist-ssr/
*.local
.vite/

# Tauri-Rust build artifacts
src-tauri/target/
src-tauri/gen/

# Local Tauri config (if we introduce one later)
*.env
```

- [ ] **Step 4: Commit the scaffolding stub**

At this point `cargo build --workspace` will FAIL because `desktop/src-tauri` has no manifest yet. That's fixed in Task 3. Commit what we have so progress is recorded:

```bash
git add Cargo.toml desktop/.gitignore
git commit -m "feat(desktop): register desktop/src-tauri as a workspace member"
```

---

### Task 3: Tauri backend scaffolding

**Files:**
- Create: `desktop/src-tauri/Cargo.toml`
- Create: `desktop/src-tauri/build.rs`
- Create: `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/capabilities/default.json`
- Create: `desktop/src-tauri/icons/*.png` (placeholder set)
- Create: `desktop/src-tauri/src/main.rs` (minimal)

`★ Why this matters:` Get the Tauri Rust side compiling + runnable as an empty window before adding any real logic.

- [ ] **Step 1: `desktop/src-tauri/Cargo.toml`**

```toml
[package]
name = "messagehub-desktop"
version = "0.1.0"
description = "MessageHub desktop shell"
authors = ["MessageHub contributors"]
edition = "2021"
rust-version = "1.75"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "sync", "macros"] }
toml = "0.8"
uuid = { version = "1", features = ["v4", "v5", "serde"] }

messagehub-core = { path = "../../core" }

[features]
# Exposed for possible future Tauri plugins; empty in 7b.1.
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
```

- [ ] **Step 2: `desktop/src-tauri/build.rs`**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 3: `desktop/src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "MessageHub",
  "version": "0.1.0",
  "identifier": "com.messagehub.desktop",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "MessageHub",
        "width": 1000,
        "height": 700,
        "minWidth": 800,
        "minHeight": 500,
        "resizable": true,
        "fullscreen": false
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": false,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.png"
    ]
  }
}
```

- [ ] **Step 4: `desktop/src-tauri/capabilities/default.json`**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capabilities for the MessageHub desktop window",
  "windows": ["main"],
  "permissions": [
    "core:default"
  ]
}
```

- [ ] **Step 5: Placeholder icons**

Tauri refuses to build without icon files. Generate a simple solid-color 1024×1024 PNG and derive the required sizes. The fastest option is ImageMagick:

```bash
cd desktop/src-tauri/icons
convert -size 1024x1024 xc:'#1e6091' -fill white -gravity center \
  -pointsize 600 -annotate +0+0 'M' icon.png
convert icon.png -resize 32x32  32x32.png
convert icon.png -resize 128x128 128x128.png
convert icon.png -resize 256x256 '128x128@2x.png'
cd ../../..
```

If `convert` is not installed, any 1024×1024 PNG works — create `icon.png` and copy-resize manually, or use a 1×1 solid PNG scaled up. The icon doesn't matter for 7b.1; it just has to exist.

- [ ] **Step 6: Minimal `desktop/src-tauri/src/main.rs`**

```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 7: Verify the Rust side compiles**

```bash
cargo build --workspace
```

Expected: clean build. The Tauri binary won't launch yet (no frontend), but `cargo build` must succeed for both `core` and `messagehub-desktop`.

- [ ] **Step 8: Commit**

```bash
git add desktop/src-tauri/
git commit -m "feat(desktop): Tauri 2 backend scaffolding with empty window"
```

---

### Task 4: Frontend scaffolding

**Files:**
- Create: `desktop/package.json`
- Create: `desktop/vite.config.ts`
- Create: `desktop/tsconfig.json`
- Create: `desktop/tsconfig.node.json`
- Create: `desktop/index.html`
- Create: `desktop/src/main.tsx`
- Create: `desktop/src/App.tsx` (stub)
- Create: `desktop/src/App.css` (empty placeholder)

`★ Why this matters:` Get Vite + React compiling, and verify `npm run tauri dev` launches an actual desktop window with the React app inside.

- [ ] **Step 1: `desktop/package.json`**

```json
{
  "name": "messagehub-desktop-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "@tauri-apps/api": "^2.1.1"
  },
  "devDependencies": {
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.3",
    "typescript": "^5.6.3",
    "vite": "^5.4.11",
    "@tauri-apps/cli": "^2.1.0"
  }
}
```

- [ ] **Step 2: `desktop/vite.config.ts`**

```typescript
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 2 expects the dev server at a fixed port and strictPort = true
// so it can inject the webview at startup.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2021",
    minify: false,
    sourcemap: true,
  },
});
```

- [ ] **Step 3: `desktop/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2021",
    "useDefineForClassFields": true,
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

- [ ] **Step 4: `desktop/tsconfig.node.json`**

```json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 5: `desktop/index.html`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>MessageHub</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 6: `desktop/src/main.tsx`**

```typescript
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 7: `desktop/src/App.tsx` (stub)**

```typescript
export default function App() {
  return (
    <main style={{ padding: "2rem", fontFamily: "system-ui, sans-serif" }}>
      <h1>MessageHub</h1>
      <p>Scaffold — real list view lands in Task 7.</p>
    </main>
  );
}
```

- [ ] **Step 8: Empty placeholder `desktop/src/App.css`**

```css
/* 7b.1 styles land in Task 7. */
body { margin: 0; }
```

- [ ] **Step 9: Install deps + verify `npm run tauri dev` launches a window**

```bash
cd desktop
npm install
npm run tauri dev
```

Expected: a desktop window opens titled "MessageHub" showing the "Scaffold — real list view lands in Task 7" text. Close the window; the dev server exits.

If `npm run tauri dev` errors with `Error: could not find Cargo project at src-tauri`, confirm you are inside `desktop/` when running it.

- [ ] **Step 10: Commit**

```bash
cd ..
git add desktop/package.json desktop/package-lock.json desktop/vite.config.ts \
        desktop/tsconfig*.json desktop/index.html desktop/src/
git commit -m "feat(desktop): React + Vite frontend scaffolding; window launches"
```

---

### Task 5: Config loader + AppState in Tauri backend

**Files:**
- Create: `desktop/src-tauri/src/config.rs`
- Create: `desktop/src-tauri/src/state.rs`
- Modify: `desktop/src-tauri/src/main.rs`

`★ Why this matters:` Separate the concerns of "load TOML + open DB" from the Tauri builder so the startup path is testable-ish and so the error screen in Task 8 has a clean data shape to render.

- [ ] **Step 1: `desktop/src-tauri/src/config.rs`**

```rust
//! TOML config loader. Reuses the schema from runtime-demo — the
//! `[ai]` and `[[channels]]` sections are ignored here.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct DesktopConfig {
    pub database: String,
    pub password: String,
}

/// Locate the config file. Checks `./messagehub.toml` first (if launched
/// from desktop/), then `../core/messagehub.toml` (if reusing
/// runtime-demo's config), then returns the first one that exists.
pub fn resolve_config_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("messagehub.toml"),
        PathBuf::from("../core/messagehub.toml"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn load_config(path: &Path) -> Result<DesktopConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
    toml::from_str::<DesktopConfig>(&text)
        .map_err(|e| format!("failed to parse '{}': {}", path.display(), e))
}
```

- [ ] **Step 2: `desktop/src-tauri/src/state.rs`**

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use messagehub_core::store::Store;
use uuid::Uuid;

/// Shared state registered with Tauri via `Builder::manage`.
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub label_by_channel_id: HashMap<Uuid, String>,
    pub db_path: String,
}

impl AppState {
    pub fn init(db_path: &str, password: &str) -> Result<Self, String> {
        let store = Store::open(std::path::Path::new(db_path), password)
            .map_err(|e| format!("failed to open store: {}", e))?;
        let channel_configs = store
            .list_channel_configs()
            .map_err(|e| format!("failed to list channels: {}", e))?;
        let label_by_channel_id = channel_configs
            .iter()
            .map(|c| (c.id, c.label.clone()))
            .collect::<HashMap<_, _>>();
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            label_by_channel_id,
            db_path: db_path.to_string(),
        })
    }
}
```

- [ ] **Step 3: Wire into `main.rs`**

Replace `desktop/src-tauri/src/main.rs` with:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod state;

use state::AppState;

fn main() {
    let init_result = try_init();
    match init_result {
        Ok(app_state) => {
            tauri::Builder::default()
                .manage(app_state)
                .run(tauri::generate_context!())
                .expect("error while running tauri application");
        }
        Err(err) => {
            // For 7b.1 we print to stderr and also open a Tauri window with a
            // plain error message. Commands will be unreachable because the
            // state never registered, but the user sees *something*.
            eprintln!("messagehub-desktop: {}", err);
            tauri::Builder::default()
                .manage(InitError(err))
                .run(tauri::generate_context!())
                .expect("error while running tauri application");
        }
    }
}

struct InitError(String);

fn try_init() -> Result<AppState, String> {
    let path = config::resolve_config_path()
        .ok_or_else(|| "messagehub.toml not found (checked ./ and ../core/)".to_string())?;
    let cfg = config::load_config(&path)?;
    AppState::init(&cfg.database, &cfg.password)
}
```

- [ ] **Step 4: Verify**

```bash
cd desktop
npm run tauri dev
```

Two scenarios to verify manually:

- With `../core/messagehub.toml` present + valid DB password: window opens, stderr stays quiet.
- With no config: window still opens; stderr prints `messagehub.toml not found …`.

- [ ] **Step 5: Commit**

```bash
cd ..
git add desktop/src-tauri/src/
git commit -m "feat(desktop): config loader + AppState managed by Tauri"
```

---

### Task 6: Rust commands — the TS API surface

**Files:**
- Create: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs` (register commands + invoke handler)

`★ Why this matters:` Four `#[tauri::command]`s are the entire Rust→TS bridge for 7b.1. Get the DTO shapes right; the TS side hand-writes matching types in Task 7.

- [ ] **Step 1: `desktop/src-tauri/src/commands.rs`**

```rust
use messagehub_core::store::Store;
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Serialize)]
pub struct MessageRow {
    pub id: String,
    pub timestamp: String,       // RFC3339
    pub channel: String,
    pub channel_label: Option<String>,
    pub sender_name: String,
    pub subject: Option<String>,
    pub preview: String,
    pub category: Option<String>,
    pub priority: Option<u8>,
    pub is_read: bool,
}

#[derive(Serialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct MessageDetail {
    #[serde(flatten)]
    pub row: MessageRow,
    pub body: String,
    pub html: Option<String>,
    pub thread_id: String,
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Serialize)]
pub struct ChannelInfo {
    pub id: String,
    pub channel_type: String,
    pub label: String,
    pub enabled: bool,
    pub status: String,
    pub last_sync_at: Option<String>,
}

#[derive(Serialize)]
pub struct UiConfig {
    pub db_path: String,
    pub channel_count: usize,
}

// ------------------ helpers ------------------

fn store_lock<'a>(state: &'a State<'a, AppState>) -> std::sync::MutexGuard<'a, Store> {
    state
        .store
        .lock()
        .expect("messagehub-desktop: store mutex poisoned")
}

fn build_message_row(
    state: &State<AppState>,
    store: &Store,
    msg: &messagehub_core::types::Message,
) -> MessageRow {
    // Contact display name is looked up lazily per row. 50 rows @ 1 query
    // each is fine for 7b.1; later plans can batch via a single JOIN helper.
    let sender_name = store
        .get_contact(&msg.sender_id)
        .map(|c| c.display_name)
        .unwrap_or_else(|_| "(unknown)".to_string());

    let preview = msg
        .content
        .text
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(140)
        .collect();

    let channel_label = state.label_by_channel_id.get(&msg.thread_id).cloned();
    // ^^ channel label is not keyed by thread_id — fix: look it up by the
    // channel_type column on the message. For 7b.1 we expose the channel
    // variant name instead, and let the UI optionally fall back to it.

    MessageRow {
        id: msg.id.to_string(),
        timestamp: msg.timestamp.to_rfc3339(),
        channel: msg.channel.to_db_str().to_string(),
        channel_label,
        sender_name,
        subject: msg.content.subject.clone(),
        preview,
        category: msg.category.clone(),
        priority: msg.priority.map(|p| p.value()),
        is_read: msg.is_read,
    }
}
```

**Note to implementer:** the `build_message_row` helper's `channel_label` lookup is intentionally imperfect in the stub above. Fix by replacing the `state.label_by_channel_id.get(&msg.thread_id)` with a proper lookup: find the first channel config whose `channel` matches `msg.channel` — or, cleaner, make `AppState` hold a `label_by_channel_variant: HashMap<Channel, Vec<String>>` for 7b.1's display needs. Adapt whichever is simpler to implement and makes the UI readable.

- [ ] **Step 2: `list_messages` command**

```rust
#[tauri::command]
pub async fn list_messages(
    limit: u32,
    offset: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MessageRow>, String> {
    let guard = store_lock(&state);
    // Store::list_messages signature may differ — inspect core/src/store/messages.rs.
    // The contract here: return messages ordered by timestamp DESC, paginated.
    let msgs = guard
        .list_messages_paginated(limit, offset)
        .map_err(|e| e.to_string())?;
    let rows = msgs
        .iter()
        .map(|m| build_message_row(&state, &guard, m))
        .collect();
    Ok(rows)
}
```

**Implementer note:** if `Store::list_messages_paginated` does not exist, you have two options:

(a) Add a thin helper on `Store` in `core/src/store/messages.rs`:

```rust
pub fn list_messages_paginated(
    &self,
    limit: u32,
    offset: u32,
) -> Result<Vec<Message>> {
    // SELECT ... FROM messages ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2
}
```

This is a legitimate additive change to `core`; commit it as part of this task with a one-line note in the commit.

(b) Use whatever the existing `list_messages` API takes and adapt. If it requires a query struct, build a default query with a `limit` that respects the argument.

Pick whichever minimizes churn.

- [ ] **Step 3: `get_message`, `list_channels`, `get_config` commands**

```rust
#[tauri::command]
pub async fn get_message(
    id: String,
    state: State<'_, AppState>,
) -> Result<MessageDetail, String> {
    let uuid = Uuid::parse_str(&id).map_err(|e| format!("bad id: {}", e))?;
    let guard = store_lock(&state);
    let msg = guard.get_message(&uuid).map_err(|e| e.to_string())?;
    let row = build_message_row(&state, &guard, &msg);
    let attachments = msg
        .content
        .attachments
        .iter()
        .map(|a| AttachmentInfo {
            filename: a.filename.clone(),
            size_bytes: a.size_bytes,
        })
        .collect();
    Ok(MessageDetail {
        row,
        body: msg.content.text.unwrap_or_default(),
        html: msg.content.html,
        thread_id: msg.thread_id.to_string(),
        attachments,
    })
}

#[tauri::command]
pub async fn list_channels(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelInfo>, String> {
    let guard = store_lock(&state);
    let configs = guard.list_channel_configs().map_err(|e| e.to_string())?;
    Ok(configs
        .into_iter()
        .map(|c| ChannelInfo {
            id: c.id.to_string(),
            channel_type: c.channel.to_db_str().to_string(),
            label: c.label,
            enabled: c.enabled,
            status: format!("{:?}", c.status),
            last_sync_at: c.last_sync_at.map(|t| t.to_rfc3339()),
        })
        .collect())
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<UiConfig, String> {
    Ok(UiConfig {
        db_path: state.db_path.clone(),
        channel_count: state.label_by_channel_id.len(),
    })
}
```

- [ ] **Step 4: Register commands in `main.rs`**

Edit `desktop/src-tauri/src/main.rs`. Add `mod commands;` to the module declarations and wire the `invoke_handler`:

```rust
use tauri::Manager;

tauri::Builder::default()
    .manage(app_state)
    .invoke_handler(tauri::generate_handler![
        commands::list_messages,
        commands::get_message,
        commands::list_channels,
        commands::get_config,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

Apply the same `invoke_handler` registration to the error-case branch so the UI can at least call `get_config` (or its own "why did init fail" command, which 7b.1 doesn't add).

- [ ] **Step 5: Build**

```bash
cargo build -p messagehub-desktop
```

Expected: clean build. Warnings about unused variants are OK at this stage; errors are not.

- [ ] **Step 6: Commit**

```bash
git add desktop/src-tauri/src/ core/src/store/messages.rs
git commit -m "feat(desktop): four #[tauri::command]s bridging Store to TS

- list_messages, get_message, list_channels, get_config
- MessageRow / MessageDetail / ChannelInfo / UiConfig DTOs
- If Store needed a new list_messages_paginated helper, it lands here"
```

(Omit `core/src/store/messages.rs` from the `git add` if you didn't need to touch it.)

---

### Task 7: Frontend — types, api wrapper, message list, click-to-expand

**Files:**
- Create: `desktop/src/types.ts`
- Create: `desktop/src/api.ts`
- Modify: `desktop/src/App.tsx`
- Modify: `desktop/src/App.css`

`★ Why this matters:` The only user-facing task. After this commit, `npm run tauri dev` actually shows messages.

- [ ] **Step 1: `desktop/src/types.ts`**

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

export type AttachmentInfo = {
  filename: string;
  size_bytes: number;
};

export type MessageDetail = MessageRow & {
  body: string;
  html: string | null;
  thread_id: string;
  attachments: AttachmentInfo[];
};

export type ChannelInfo = {
  id: string;
  channel_type: string;
  label: string;
  enabled: boolean;
  status: string;
  last_sync_at: string | null;
};

export type UiConfig = {
  db_path: string;
  channel_count: number;
};
```

- [ ] **Step 2: `desktop/src/api.ts`**

```typescript
import { invoke } from "@tauri-apps/api/core";
import type { MessageRow, MessageDetail, ChannelInfo, UiConfig } from "./types";

export const api = {
  listMessages: (limit: number, offset: number) =>
    invoke<MessageRow[]>("list_messages", { limit, offset }),

  getMessage: (id: string) =>
    invoke<MessageDetail>("get_message", { id }),

  listChannels: () =>
    invoke<ChannelInfo[]>("list_channels"),

  getConfig: () =>
    invoke<UiConfig>("get_config"),
};
```

- [ ] **Step 3: Replace `desktop/src/App.tsx`**

```typescript
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "./api";
import type { MessageRow, MessageDetail, UiConfig } from "./types";

const PAGE_SIZE = 50;

export default function App() {
  const [config, setConfig] = useState<UiConfig | null>(null);
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [detail, setDetail] = useState<MessageDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const loadInitial = useCallback(async () => {
    setError(null);
    setLoading(true);
    try {
      const [cfg, rows] = await Promise.all([
        api.getConfig(),
        api.listMessages(PAGE_SIZE, 0),
      ]);
      setConfig(cfg);
      setMessages(rows);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadInitial();
  }, [loadInitial]);

  const loadMore = useCallback(async () => {
    try {
      const next = await api.listMessages(PAGE_SIZE, messages.length);
      setMessages((m) => [...m, ...next]);
    } catch (err) {
      setError(String(err));
    }
  }, [messages.length]);

  const openDetail = useCallback(async (id: string) => {
    setError(null);
    try {
      const d = await api.getMessage(id);
      setDetail(d);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  const dbPath = config?.db_path ?? "(no config)";
  const channelCount = config?.channel_count ?? 0;

  if (detail) {
    return (
      <DetailView detail={detail} onBack={() => setDetail(null)} />
    );
  }

  return (
    <div className="app">
      <header className="header">
        <div className="brand">MessageHub</div>
        <div className="meta">
          db: <code>{dbPath}</code> · {channelCount} channel
          {channelCount === 1 ? "" : "s"}
        </div>
        <button onClick={loadInitial} disabled={loading}>
          {loading ? "Loading..." : "Refresh"}
        </button>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <ul className="message-list">
        {messages.map((m) => (
          <li
            key={m.id}
            className={`row ${m.is_read ? "" : "unread"}`}
            onClick={() => openDetail(m.id)}
          >
            <div className="row-main">
              <span className="time">{formatTime(m.timestamp)}</span>
              <span className="channel">
                [{m.channel_label ?? m.channel}]
              </span>
              <span className="sender">{m.sender_name}</span>
            </div>
            <div className="row-subject">
              {m.subject ?? "(no subject)"}
            </div>
            <div className="row-preview">{m.preview}</div>
            <div className="row-meta">
              {m.category ?? "—"}
              {m.priority !== null ? ` · P${m.priority}` : ""}
            </div>
          </li>
        ))}
      </ul>

      {messages.length > 0 && messages.length % PAGE_SIZE === 0 && (
        <button className="load-more" onClick={loadMore}>
          Load more
        </button>
      )}
      {messages.length === 0 && !loading && (
        <div className="empty">No messages yet. Run <code>runtime-demo</code> to populate the DB.</div>
      )}
    </div>
  );
}

function DetailView({
  detail,
  onBack,
}: {
  detail: MessageDetail;
  onBack: () => void;
}) {
  return (
    <div className="detail">
      <button onClick={onBack} className="back">← Back</button>
      <div className="detail-head">
        <span className="channel">[{detail.channel_label ?? detail.channel}]</span>
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

function formatTime(isoString: string): string {
  const d = new Date(isoString);
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
```

- [ ] **Step 4: `desktop/src/App.css`**

```css
* { box-sizing: border-box; }
body {
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: #f7f8fa;
  color: #1a1f2b;
}

.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
}

.header {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.75rem 1rem;
  background: #ffffff;
  border-bottom: 1px solid #e4e7ec;
}
.brand { font-weight: 600; font-size: 1.1rem; }
.meta { flex: 1; color: #667085; font-size: 0.85rem; }
.meta code { background: #eef0f4; padding: 0 4px; border-radius: 3px; }
.header button { cursor: pointer; padding: 4px 10px; }

.error-banner {
  background: #fff1f3;
  color: #a4203a;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid #ffd3d8;
  font-size: 0.85rem;
}

.message-list {
  list-style: none;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  flex: 1;
}
.row {
  padding: 0.7rem 1rem;
  border-bottom: 1px solid #eef0f4;
  cursor: pointer;
}
.row:hover { background: #ffffff; }
.row.unread .row-subject { font-weight: 600; }
.row-main {
  display: flex;
  gap: 0.8rem;
  font-size: 0.8rem;
  color: #667085;
  margin-bottom: 2px;
}
.row-main .time { width: 60px; }
.row-main .channel { color: #3f5ab5; }
.row-main .sender { color: #1a1f2b; font-weight: 500; }
.row-subject { font-size: 0.95rem; }
.row-preview {
  font-size: 0.85rem;
  color: #667085;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.row-meta { font-size: 0.75rem; color: #94a3b8; margin-top: 2px; }

.load-more {
  margin: 0.5rem auto 1rem;
  padding: 6px 16px;
  align-self: center;
  cursor: pointer;
}

.empty {
  padding: 3rem 1rem;
  text-align: center;
  color: #667085;
}
.empty code { background: #eef0f4; padding: 0 4px; border-radius: 3px; }

.detail {
  padding: 1.5rem;
  max-width: 800px;
  margin: 0 auto;
  overflow-y: auto;
}
.back {
  border: 1px solid #d7dae0;
  background: #ffffff;
  padding: 4px 10px;
  cursor: pointer;
  margin-bottom: 1rem;
}
.detail-head {
  display: flex;
  gap: 0.8rem;
  color: #667085;
  font-size: 0.85rem;
  margin-bottom: 0.3rem;
}
.detail-head .channel { color: #3f5ab5; }
.detail-subject { margin: 0 0 0.3rem; font-size: 1.25rem; }
.detail-meta { color: #94a3b8; font-size: 0.85rem; margin-bottom: 1.2rem; }
.detail-body {
  white-space: pre-wrap;
  word-wrap: break-word;
  font-family: inherit;
  font-size: 0.95rem;
  line-height: 1.5;
  background: #ffffff;
  padding: 1rem;
  border: 1px solid #e4e7ec;
  border-radius: 4px;
}
.detail-attachments {
  margin-top: 1rem;
  font-size: 0.85rem;
}
```

- [ ] **Step 5: Run the full app**

```bash
cd desktop
npm run tauri dev
```

Expected:
- Window opens showing messages from `../core/messagehub.db` (if populated).
- Empty-state message shows if the DB has no messages.
- Click a row → body view. Click Back → back to list.
- Refresh button re-fetches.
- If DB grows beyond 50 messages, "Load more" appears.

- [ ] **Step 6: Commit**

```bash
cd ..
git add desktop/src/
git commit -m "feat(desktop): read-only message list with click-to-expand detail view"
```

---

### Task 8: Manual verification checklist + merge

**Files:** (none — verification + merge)

- [ ] **Step 1: Full build sweep**

```bash
cargo build --workspace
cargo test -p messagehub-core
cd desktop && npm run build && cd ..
```

Expected: all green. No new Rust warnings in `messagehub-desktop` (Tauri's own scaffolding may emit a few; ignore those).

- [ ] **Step 2: Launch smoke test**

```bash
# Terminal 1 — keep runtime-demo populating messages
cd core
cargo run --bin runtime-demo

# Terminal 2 — launch Tauri app
cd desktop
npm run tauri dev
```

Confirm manually:
- Window opens without error.
- Messages from a previous `runtime-demo` run are visible.
- Clicking a row shows the full body.
- "Back" returns to the list.
- "Refresh" picks up a newly ingested message after waiting one poll interval.
- Close the window → process exits cleanly.

- [ ] **Step 3: Missing-config error path**

Temporarily rename or move `core/messagehub.toml` and any `desktop/messagehub.toml`. Re-launch `npm run tauri dev`. Expected: window still opens; stderr prints `messagehub.toml not found …`. Commands will fail because AppState isn't registered; the error banner shows the string.

Restore the config after testing.

- [ ] **Step 4: Merge to master**

```bash
git checkout master
git merge --no-ff feat/tauri-shell -m "Merge branch 'feat/tauri-shell': Plan 7b.1 — Tauri read-only shell

First slice of the desktop UI. Tauri 2 app with React/TS frontend
that reads messages from the SQLCipher DB via four #[command]s
and renders a scrollable list with click-to-expand body view.

Read-only — no Runtime in-app. runtime-demo keeps the DB fresh
alongside. 7b.2 adds the three-panel layout; 7b.3 the reply
composer; 7b.4 channel CRUD; 7b.5 keychain integration."
git push origin master
```

---

## Notes for the executor

- **Do not `npm install` globally.** All frontend deps live in `desktop/`.
- **Do not run `npm run tauri build`** in 7b.1 — bundling is deferred. Only `npm run tauri dev` (which runs `vite` + hot-reloads Rust).
- **If `npm install` warns about peer deps**, ignore unless it actually fails.
- **If `cargo build -p messagehub-desktop` complains about icon files**, generate real PNGs per Task 3 Step 5 — Tauri validates the paths in `tauri.conf.json` at build time.
- **If a Store method name doesn't match this plan** (e.g., `list_messages_paginated` doesn't exist), pick the closest existing method, commit the adaptation, and note it in the Task 6 commit message.
- **TypeScript errors are blockers.** `npm run build` must succeed (it runs `tsc --noEmit`). Loosening `strict` is not on the table.
- **If webkit2gtk linking fails on Linux**, the error message is usually "error: linking with `cc` failed" — almost always means missing system packages. Re-run Task 1 Step 2.
