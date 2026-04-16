# MessageHub — Design Specification

**Date:** 2026-04-16
**Status:** Approved
**Author:** Jocelyn Moreau + Claude

## Overview

MessageHub is an open-source, local-first unified inbox with a knowledge-aware personal assistant for individual professionals. It aggregates messages from Email, SMS/WhatsApp, MS Teams, and Telegram into a single AI-powered smart inbox. A personal assistant learns from the user's Obsidian vault to provide context-aware prioritization, categorization, and reply drafting.

**Core value:** Fast unified view — speed and reliability above all.

**Target user:** Individual professionals (freelancers, consultants, entrepreneurs) who manage multiple communication channels and want a single place to read and reply to everything.

## Architecture

### Approach: Monorepo with Shared Rust Core

One repository with three packages:

- **`core/`** — Rust library containing all business logic: channel adapters, message store, AI pipeline, knowledge engine. Compiles as a native library for both platforms.
- **`desktop/`** — Tauri application. Rust backend (thin, calls into core) + React/TypeScript frontend.
- **`mobile/`** — React Native application. Calls into core via UniFFI-generated native module bridge (Kotlin/Swift bindings).

### Monorepo Structure

```
messagehub/
├── core/
│   ├── adapters/       # Channel adapters (Email, WhatsApp, SMS, Teams, Telegram)
│   ├── store/          # SQLite message store + sqlite-vss vector store
│   ├── ai/
│   │   ├── local/      # llama.cpp bindings, priority scoring, classification
│   │   ├── cloud/      # Claude/GPT API client, opt-in actions
│   │   └── rag/        # RAG pipeline: vault retrieval → prompt assembly
│   ├── knowledge/      # Vault ingestion, file watcher, embedding, people extraction
│   └── lib.rs          # Public API via UniFFI
├── desktop/            # Tauri app (React/TS frontend)
├── mobile/             # React Native app (UniFFI bridge)
└── shared/             # Shared TS types (optional)
```

### Seven Components

1. **Channel Adapters** — fetch/send messages via external APIs. Each adapter implements a common `ChannelAdapter` trait with `connect()`, `fetch_messages()`, `send_reply()`, `disconnect()`.
2. **Message Store** — SQLite + FTS5 for messages, threads, contacts. WAL mode for concurrent read/write.
3. **Vector Store** — sqlite-vss for vault embeddings and semantic search.
4. **Knowledge Engine** — vault ingestion, file watching, people extraction, incremental indexing.
5. **AI Pipeline** — local classification (Tier 1) + cloud RAG drafts/summaries (Tier 2).
6. **UniFFI Bridge** — exposes core API to both desktop and mobile.
7. **UI Layer** — three-panel inbox, thin, platform-specific (React for Tauri, React Native for mobile).

### Data Flow

1. Channel adapters poll/listen to external services (IMAP, MS Graph API, Telegram Bot API, Twilio).
2. Messages are normalized into a unified `Message` struct.
3. AI pipeline scores priority and classifies each message (using vault context for enrichment).
4. Messages are persisted to local SQLite database.
5. UI layer queries the store and renders the smart inbox.

## Channel Adapters

### Supported Channels (v1)

| Channel | Protocol | Auth | Polling Interval |
|---------|----------|------|-----------------|
| Email | IMAP/SMTP + MS Graph API | OAuth2 or app password | 30s |
| SMS | Twilio REST API | API key | 30s |
| WhatsApp | Twilio WhatsApp API | API key | 5s |
| MS Teams | MS Graph API | OAuth2 (Azure AD) | 5s |
| Telegram | Telegram Bot API | Bot token | 5s (long-polling) |

### Common Trait

```rust
trait ChannelAdapter {
    async fn connect(&mut self, credentials: Credentials) -> Result<()>;
    async fn fetch_messages(&self, since: DateTime) -> Result<Vec<RawMessage>>;
    async fn send_reply(&self, thread_id: &str, content: MessageContent) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
}
```

### Message Normalization

Every adapter converts its native format into:

```rust
struct Message {
    id: Uuid,
    channel: Channel,
    thread_id: String,
    sender: Contact,
    content: MessageContent,
    timestamp: DateTime<Utc>,
    metadata: HashMap<String, String>,
    priority: Option<PriorityScore>,
}
```

Each adapter runs on its own background thread with configurable polling interval and tracks its own sync cursor.

## Local Storage

### Database: SQLite + SQLCipher

Single encrypted file per device. No server, no sync.

**Core tables:**

- **messages** — normalized messages with FTS5 full-text search index. Columns: id, channel, thread_id, sender_id, content, timestamp, priority_score, category, is_read, is_archived.
- **contacts** — deduplicated contacts across channels with identity merging (email address + Telegram username + Teams user → one person).
- **channels** — configured channel connections. Sync state (last cursor, last fetch time, connection status). Credentials stored in OS keychain, not in SQLite.
- **threads** — conversation threads grouping related messages.
- **vault_chunks** — embedded vault content for vector search (via sqlite-vss).
- **vault_people** — structured people data extracted from vault frontmatter.
- **action_log** — every AI decision with reasoning (future-proofing for semi-autonomous mode).

### Design Choices

- **FTS5** for instant full-text search across all channels.
- **WAL mode** for concurrent reads (UI) alongside writes (adapter sync).
- **OS keychain** for credentials (macOS Keychain, libsecret on Linux, Windows Credential Manager).
- **SQLCipher** for encryption at rest. Master password via Argon2 key derivation. Optional biometric unlock.
- **Retention:** messages kept indefinitely by default. Configurable retention policy (auto-archive after N days).

## AI Pipeline

### Tier 1 — Local Classification (Always On)

Runs on-device via `llama.cpp` bindings (`llama-cpp-rs` crate). Small model (~3B parameters, e.g., Phi-3 Mini).

**Responsibilities:**

- **Priority scoring** (1-5) — based on sender importance (from vault contacts), keywords, urgency signals, past interaction patterns.
- **Category tagging** — work, personal, finance, family, notifications, newsletters, spam. Categories derived from the user's PARA vault structure.
- **Thread grouping** — detecting which messages belong to the same conversation.

**Performance target:** <500ms per message on modest hardware.

### Tier 2 — Cloud API (Opt-in Per Action)

Calls Claude API (or configurable provider) only when the user explicitly requests it.

**Responsibilities:**

- **Summarize thread** — with vault context (knows the people and projects involved).
- **Draft reply** — context-aware, in the correct language and tone for the relationship.
- **Smart search** — natural language queries leveraging vault knowledge.

### Privacy Guardrails

- Local model runs fully offline.
- Cloud calls send only relevant message content (user can review before sending).
- Clear UI indicator when data is being sent to an API.
- Option to redact named entities before sending.
- User can disable cloud tier entirely.
- All AI features degrade gracefully — app works without any AI.

## Knowledge Engine (Personal Assistant)

### Vault Ingestion Pipeline

```
Obsidian vault (PARA structure)
    ↓ FileWatcher (inotify/FSEvents)
    ↓ Markdown parser (extracts YAML frontmatter + body)
    ↓ Chunker (semantic chunks ~500 tokens)
    ↓ Embedding model (local, all-MiniLM-L6-v2 via candle)
    ↓ Vector store (sqlite-vss)
```

- **Initial index:** Full vault processed on first launch. ~154 files completes in under a minute.
- **Incremental updates:** File watcher detects changes, re-indexes only modified files. Deletions remove vectors.
- **Structured extraction:** People files (`05-People/*.md`) get special treatment — frontmatter (name, role, tags, relationships) parsed into `vault_people` enrichment table for sender cross-referencing.

### RAG Pipeline

When processing an incoming message:

1. **Sender lookup** — match sender against `05-People/` profiles and contact table. Get relationship, context, language preference.
2. **Topic retrieval** — embed the message, retrieve top-5 relevant vault chunks. Surfaces project context, prior emails, related notes.
3. **User profile injection** — always include `user-profile.md` context: languages (EN/FR/DE), role, tone, life areas.
4. **Prompt assembly** — local LLM (priority/category) or cloud API (drafts/summaries) receives message + retrieved context + user profile.

### Practical Example

> Incoming: Email from `mail@schulmanager-mail1.de` about "Elternbrief"
>
> 1. Sender lookup → no direct people match, but vault has prior email from same sender tagged with `alix, ecole`
> 2. Topic retrieval → finds `05-People/Alix Moreau.md` (daughter, Realschule Rain), prior archived school email
> 3. User profile → trilingual, this is family/personal area
> 4. Result: Priority = medium, Category = family/school, Draft reply language = German, suggested action = "read letter on Schulmanager, decide on attendance"

### UI Capabilities

- "Why is this prioritized?" → explains reasoning with vault references.
- "Draft a reply" → context-aware draft in the right language and tone for the relationship.
- "Summarize this thread" → summary referencing your relationship with the sender.
- "What do I know about this person?" → surfaces vault notes, past messages, relationship context.

### Future-Proofing for Semi-Autonomous Mode

Architecture includes inactive hooks:

- `confidence_score: f32` on every draft (ready for auto-send threshold).
- `action_log` table records every AI decision with reasoning.
- `AutoReplyRule` trait (empty implementation, ready for rule definitions).

## UI — Three-Panel Smart Inbox

### Desktop (Tauri)

Three-panel layout:

- **Left sidebar** (60px) — navigation icons: Inbox, Starred, Archive, Settings, Search.
- **Message list** (260px) — chronological, AI-sorted. Each entry shows: sender, subject/preview, timestamp, channel badge (color-coded), priority indicator (left border color: red=urgent, yellow=work, green=personal, gray=low).
- **Reading pane** (remaining) — full message content with reply composer at bottom. Reply composer includes "AI Draft" button (cloud, opt-in) and send button. Channel-aware: "Reply to Sarah via Email..."

### Mobile (React Native)

Adapts naturally:

- Message list is the main view (full screen).
- Tap opens reading pane (push navigation).
- Pull-to-refresh triggers adapter sync.
- Swipe gestures: left=archive, right=star.

### Smart Inbox Features

- AI priority sorting (local model).
- Channel badges with color coding per channel type.
- Category tags (work, personal, finance, family, etc.).
- Unread indicators and counts.
- Search (FTS5 + semantic via vault vectors).

## Security

- **Credentials:** OS keychain (never in SQLite or config files). OAuth2 via system browser redirect.
- **Encryption at rest:** SQLCipher with Argon2 key derivation from master password.
- **Biometric unlock:** optional, on supported devices (Touch ID, fingerprint).
- **Cloud AI:** HTTPS only, explicit opt-in per action, visual indicator showing what's being sent, optional entity redaction.
- **Logging:** structured via `tracing` crate, rotated daily, 7-day retention, debug export strips message content.

## Testing Strategy

- **Core (Rust):** unit tests per adapter (mock servers), storage layer tests, AI pipeline tests. Integration tests with test containers (greenmail for IMAP/SMTP).
- **Desktop (Tauri):** integration tests for command bridge. Vitest for React frontend components.
- **Mobile (React Native):** React Native Testing Library for UI. XCTest (iOS) and JUnit (Android) for UniFFI bridge.

## Error Handling

- Channel failures: exponential backoff retry, degraded status indicator (yellow → red → "Reconnect"). Each channel independent — one failure doesn't affect others.
- AI failures: local pipeline defaults to no-priority (message still appears). Cloud failures show clear error with retry option.
- Database corruption: recovery mode offering rebuild from channel re-sync.

## Distribution

- Open source (license TBD).
- Desktop: distributed via GitHub releases (Tauri bundles for macOS, Windows, Linux).
- Mobile: F-Droid (Android), TestFlight → App Store (iOS).

---

*Spec approved: 2026-04-16*
