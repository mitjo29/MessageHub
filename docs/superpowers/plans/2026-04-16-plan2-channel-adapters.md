# Plan 2: Channel Adapters — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `ChannelAdapter` async trait, a manager to coordinate multiple adapters, and concrete adapters for Telegram (simplest API), Email (IMAP/SMTP), and a mock adapter for testing.

**Architecture:** Each adapter lives in its own module under `core/src/adapters/` and implements a common `ChannelAdapter` trait. A `RawMessage` intermediate type captures pre-normalization data from external services, and a `normalize()` function converts it into the core `Message` type. An `AdapterManager` coordinates multiple adapters, running independent sync loops on tokio tasks with configurable polling intervals.

**Tech Stack:** `tokio` (async runtime), `async-trait` (async trait support), `reqwest` (HTTP client for Telegram), `async-imap` + `async-native-tls` (IMAP), `lettre` (SMTP sending), `mail-parser` (email parsing)

---

## File Structure

```
core/
├── Cargo.toml                          # MODIFY — add async deps
├── src/
│   ├── lib.rs                          # MODIFY — add `pub mod adapters;`
│   ├── error.rs                        # MODIFY — add adapter error variants
│   ├── adapters/
│   │   ├── mod.rs                      # CREATE — trait, RawMessage, normalize, re-exports
│   │   ├── manager.rs                  # CREATE — AdapterManager
│   │   ├── telegram.rs                 # CREATE — Telegram Bot API adapter
│   │   ├── email.rs                    # CREATE — IMAP/SMTP adapter
│   │   └── mock.rs                     # CREATE — Mock adapter for testing
│   └── types/
│       └── mod.rs                      # MODIFY — re-export RawMessage
└── tests/
    └── adapter_integration.rs          # CREATE — integration tests with mock adapter
```

---

### Task 1: ChannelAdapter Trait + RawMessage + Normalization
**Files:**
- Modify: `core/Cargo.toml`
- Modify: `core/src/lib.rs`
- Modify: `core/src/error.rs`
- Create: `core/src/adapters/mod.rs`

- [ ] **Step 1: Add async dependencies to `core/Cargo.toml`**

Add the new dependencies to the `[dependencies]` section:

```toml
[package]
name = "messagehub-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
rusqlite = { version = "0.31", features = ["bundled-sqlcipher", "vtab", "modern_sqlite"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
async-trait = "0.1"
reqwest = { version = "0.12", features = ["json"] }
async-imap = "0.9"
async-native-tls = "0.5"
lettre = { version = "0.11", features = ["tokio1-native-tls", "builder"] }
mail-parser = "0.9"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Add adapter error variants to `core/src/error.rs`**

Replace the full contents of `core/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("channel error: {0}")]
    Channel(String),

    #[error("connection error: {0}")]
    Connection(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("parse error: {0}")]
    Parse(String),
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::Network(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

- [ ] **Step 3: Add `pub mod adapters;` to `core/src/lib.rs`**

```rust
pub mod error;
pub mod types;
pub mod store;
pub mod adapters;
```

- [ ] **Step 4: Create `core/src/adapters/mod.rs` with trait, RawMessage, and normalize**

```rust
pub mod email;
pub mod manager;
pub mod mock;
pub mod telegram;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::Result;
use crate::types::{Channel, ChannelConfig, Message, MessageContent, Attachment};

/// A pre-normalization message from an external service.
/// Each adapter produces these; `normalize()` converts them into core `Message` values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessage {
    /// The external service's unique ID for this message.
    pub external_id: String,
    /// Which channel this message came from.
    pub channel: Channel,
    /// External thread/conversation identifier.
    pub external_thread_id: Option<String>,
    /// Sender display name.
    pub sender_name: String,
    /// Sender address (email, phone number, username, etc.).
    pub sender_address: String,
    /// Message body as plain text.
    pub text: Option<String>,
    /// Message body as HTML (email only).
    pub html: Option<String>,
    /// Subject line (email only).
    pub subject: Option<String>,
    /// Attachments metadata.
    pub attachments: Vec<RawAttachment>,
    /// When the message was sent.
    pub timestamp: DateTime<Utc>,
    /// Adapter-specific metadata (headers, message-ids, etc.).
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

/// The common interface every channel adapter must implement.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Establish a connection using the provided channel configuration.
    async fn connect(&mut self, config: &ChannelConfig) -> Result<()>;

    /// Fetch messages received since the given timestamp.
    /// If `since` is `None`, fetch the most recent batch.
    async fn fetch_messages(&self, since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>>;

    /// Send a reply within an existing thread.
    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()>;

    /// Disconnect and clean up resources.
    async fn disconnect(&mut self) -> Result<()>;

    /// Return the channel type this adapter handles.
    fn channel_type(&self) -> Channel;
}

/// Convert a `RawMessage` into a core `Message`.
///
/// The `sender_id` and `thread_id` are resolved by the caller (AdapterManager)
/// via contact lookup and thread matching before calling this function.
pub fn normalize(raw: RawMessage, sender_id: Uuid, thread_id: Uuid) -> Message {
    let attachments = raw
        .attachments
        .into_iter()
        .map(|a| Attachment {
            filename: a.filename,
            mime_type: a.mime_type,
            size_bytes: a.size_bytes,
            local_path: None,
        })
        .collect();

    Message {
        id: Uuid::new_v4(),
        channel: raw.channel,
        thread_id,
        sender_id,
        content: MessageContent {
            text: raw.text,
            html: raw.html,
            subject: raw.subject,
            attachments,
        },
        timestamp: raw.timestamp,
        metadata: raw.metadata,
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Channel;

    fn make_raw() -> RawMessage {
        RawMessage {
            external_id: "ext-123".to_string(),
            channel: Channel::Telegram,
            external_thread_id: Some("chat-456".to_string()),
            sender_name: "Alice".to_string(),
            sender_address: "alice_bot".to_string(),
            text: Some("Hello world".to_string()),
            html: None,
            subject: None,
            attachments: vec![RawAttachment {
                filename: "photo.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                size_bytes: 1024,
            }],
            timestamp: Utc::now(),
            metadata: HashMap::from([("chat_type".to_string(), "private".to_string())]),
        }
    }

    #[test]
    fn test_normalize_preserves_content() {
        let raw = make_raw();
        let ts = raw.timestamp;
        let sender_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();

        let msg = normalize(raw, sender_id, thread_id);

        assert_eq!(msg.channel, Channel::Telegram);
        assert_eq!(msg.sender_id, sender_id);
        assert_eq!(msg.thread_id, thread_id);
        assert_eq!(msg.content.text.as_deref(), Some("Hello world"));
        assert_eq!(msg.content.subject, None);
        assert_eq!(msg.content.attachments.len(), 1);
        assert_eq!(msg.content.attachments[0].filename, "photo.jpg");
        assert_eq!(msg.content.attachments[0].local_path, None);
        assert_eq!(msg.timestamp, ts);
        assert_eq!(msg.metadata.get("chat_type").unwrap(), "private");
        assert!(!msg.is_read);
        assert!(!msg.is_archived);
        assert!(msg.priority.is_none());
    }

    #[test]
    fn test_normalize_generates_unique_ids() {
        let raw1 = make_raw();
        let raw2 = make_raw();
        let sender = Uuid::new_v4();
        let thread = Uuid::new_v4();

        let msg1 = normalize(raw1, sender, thread);
        let msg2 = normalize(raw2, sender, thread);

        assert_ne!(msg1.id, msg2.id);
    }

    #[test]
    fn test_normalize_empty_attachments() {
        let mut raw = make_raw();
        raw.attachments.clear();
        let msg = normalize(raw, Uuid::new_v4(), Uuid::new_v4());
        assert!(msg.content.attachments.is_empty());
    }
}
```

- [ ] **Step 5: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 6: Run tests**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core -- adapters::tests
```

- [ ] **Step 7: Commit**

```bash
git add core/Cargo.toml core/src/lib.rs core/src/error.rs core/src/adapters/mod.rs
git commit -m "feat: add ChannelAdapter trait, RawMessage type, and normalize function

Defines the async ChannelAdapter trait with connect/fetch/send/disconnect,
the RawMessage intermediate type for pre-normalization data, and a
normalize() function to convert RawMessage into core Message.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: AdapterManager
**Files:**
- Create: `core/src/adapters/manager.rs`

- [ ] **Step 1: Write tests for AdapterManager**

These tests will use the mock adapter (Task 5), so we write the test expectations first and implement the mock later. For now, write the manager with inline test stubs.

Create `core/src/adapters/manager.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn, error};
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::types::{Channel, ChannelConfig};
use super::{ChannelAdapter, RawMessage};

/// Coordinates multiple channel adapters, running sync loops on background tasks.
pub struct AdapterManager {
    adapters: HashMap<Uuid, Arc<Mutex<Box<dyn ChannelAdapter>>>>,
    configs: HashMap<Uuid, ChannelConfig>,
    sync_handles: HashMap<Uuid, JoinHandle<()>>,
    /// Callback invoked for each batch of fetched raw messages.
    on_messages: Arc<dyn Fn(Vec<RawMessage>) + Send + Sync>,
}

impl AdapterManager {
    /// Create a new manager with a callback that receives fetched messages.
    pub fn new<F>(on_messages: F) -> Self
    where
        F: Fn(Vec<RawMessage>) + Send + Sync + 'static,
    {
        Self {
            adapters: HashMap::new(),
            configs: HashMap::new(),
            sync_handles: HashMap::new(),
            on_messages: Arc::new(on_messages),
        }
    }

    /// Register an adapter for a specific channel config.
    /// Connects the adapter and returns its config ID.
    pub async fn register(
        &mut self,
        mut adapter: Box<dyn ChannelAdapter>,
        config: ChannelConfig,
    ) -> Result<Uuid> {
        let config_id = config.id;

        adapter.connect(&config).await?;
        info!(
            channel = %config.channel,
            label = %config.label,
            "adapter connected"
        );

        self.adapters.insert(config_id, Arc::new(Mutex::new(adapter)));
        self.configs.insert(config_id, config);

        Ok(config_id)
    }

    /// Start the background sync loop for a registered adapter.
    pub fn start_sync(&mut self, config_id: Uuid) -> Result<()> {
        let config = self.configs.get(&config_id).ok_or_else(|| {
            CoreError::NotFound {
                entity: "ChannelConfig".to_string(),
                id: config_id.to_string(),
            }
        })?;

        if !config.enabled {
            warn!(config_id = %config_id, "adapter disabled, skipping sync");
            return Ok(());
        }

        let adapter = Arc::clone(
            self.adapters.get(&config_id).ok_or_else(|| {
                CoreError::NotFound {
                    entity: "Adapter".to_string(),
                    id: config_id.to_string(),
                }
            })?,
        );

        let poll_interval = std::time::Duration::from_secs(config.poll_interval_secs as u64);
        let last_sync = config.last_sync_at;
        let on_messages = Arc::clone(&self.on_messages);
        let channel = config.channel;

        let handle = tokio::spawn(async move {
            let mut since = last_sync;
            loop {
                {
                    let adapter = adapter.lock().await;
                    match adapter.fetch_messages(since).await {
                        Ok(messages) if !messages.is_empty() => {
                            info!(
                                channel = %channel,
                                count = messages.len(),
                                "fetched messages"
                            );
                            // Update cursor to latest message timestamp
                            if let Some(latest) = messages.iter().map(|m| m.timestamp).max() {
                                since = Some(latest);
                            }
                            (on_messages)(messages);
                        }
                        Ok(_) => {
                            // No new messages, nothing to do
                        }
                        Err(e) => {
                            error!(
                                channel = %channel,
                                error = %e,
                                "fetch failed"
                            );
                        }
                    }
                }
                tokio::time::sleep(poll_interval).await;
            }
        });

        self.sync_handles.insert(config_id, handle);
        Ok(())
    }

    /// Stop the sync loop for a specific adapter.
    pub fn stop_sync(&mut self, config_id: &Uuid) {
        if let Some(handle) = self.sync_handles.remove(config_id) {
            handle.abort();
            info!(config_id = %config_id, "sync stopped");
        }
    }

    /// Disconnect and remove an adapter.
    pub async fn unregister(&mut self, config_id: &Uuid) -> Result<()> {
        self.stop_sync(config_id);

        if let Some(adapter) = self.adapters.remove(config_id) {
            let mut adapter = adapter.lock().await;
            adapter.disconnect().await?;
        }

        self.configs.remove(config_id);
        info!(config_id = %config_id, "adapter unregistered");
        Ok(())
    }

    /// Stop all sync loops and disconnect all adapters.
    pub async fn shutdown(&mut self) -> Result<()> {
        let config_ids: Vec<Uuid> = self.adapters.keys().cloned().collect();
        for config_id in config_ids {
            self.unregister(&config_id).await?;
        }
        info!("all adapters shut down");
        Ok(())
    }

    /// Get a list of all registered config IDs.
    pub fn registered_configs(&self) -> Vec<Uuid> {
        self.configs.keys().cloned().collect()
    }

    /// Check if a specific adapter is registered and has an active sync loop.
    pub fn is_syncing(&self, config_id: &Uuid) -> bool {
        self.sync_handles
            .get(config_id)
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// Get a reference to an adapter for sending replies.
    pub fn get_adapter(&self, config_id: &Uuid) -> Option<Arc<Mutex<Box<dyn ChannelAdapter>>>> {
        self.adapters.get(config_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Channel;

    fn test_config() -> ChannelConfig {
        ChannelConfig {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "Test Telegram".to_string(),
            keychain_ref: "test-key".to_string(),
            enabled: true,
            poll_interval_secs: 1,
            last_sync_cursor: None,
            last_sync_at: None,
        }
    }

    #[tokio::test]
    async fn test_manager_register_and_unregister() {
        use crate::adapters::mock::MockAdapter;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let mut manager = AdapterManager::new(move |msgs| {
            counter_clone.fetch_add(msgs.len(), Ordering::Relaxed);
        });

        let adapter = Box::new(MockAdapter::new());
        let config = test_config();
        let config_id = config.id;

        let result = manager.register(adapter, config).await;
        assert!(result.is_ok());
        assert_eq!(manager.registered_configs().len(), 1);

        let result = manager.unregister(&config_id).await;
        assert!(result.is_ok());
        assert_eq!(manager.registered_configs().len(), 0);
    }

    #[tokio::test]
    async fn test_manager_start_and_stop_sync() {
        use crate::adapters::mock::MockAdapter;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let mut manager = AdapterManager::new(move |msgs| {
            counter_clone.fetch_add(msgs.len(), Ordering::Relaxed);
        });

        let mut adapter = MockAdapter::new();
        adapter.add_message(RawMessage {
            external_id: "msg-1".to_string(),
            channel: Channel::Telegram,
            external_thread_id: None,
            sender_name: "Bot".to_string(),
            sender_address: "bot123".to_string(),
            text: Some("Hello".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });

        let config = test_config();
        let config_id = config.id;

        manager.register(Box::new(adapter), config).await.unwrap();
        manager.start_sync(config_id).unwrap();

        assert!(manager.is_syncing(&config_id));

        // Let one poll cycle complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(counter.load(Ordering::Relaxed) >= 1);

        manager.stop_sync(&config_id);
        // The handle is aborted, is_syncing may still return true briefly
    }

    #[tokio::test]
    async fn test_manager_disabled_adapter_skips_sync() {
        use crate::adapters::mock::MockAdapter;

        let mut manager = AdapterManager::new(|_| {});

        let adapter = Box::new(MockAdapter::new());
        let mut config = test_config();
        config.enabled = false;

        let config_id = manager.register(adapter, config).await.unwrap();
        let result = manager.start_sync(config_id);
        assert!(result.is_ok());
        assert!(!manager.is_syncing(&config_id));
    }

    #[tokio::test]
    async fn test_manager_shutdown() {
        use crate::adapters::mock::MockAdapter;

        let mut manager = AdapterManager::new(|_| {});

        let config1 = test_config();
        let config2 = test_config();

        manager
            .register(Box::new(MockAdapter::new()), config1)
            .await
            .unwrap();
        manager
            .register(Box::new(MockAdapter::new()), config2)
            .await
            .unwrap();

        assert_eq!(manager.registered_configs().len(), 2);

        manager.shutdown().await.unwrap();
        assert_eq!(manager.registered_configs().len(), 0);
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 3: Run tests (will pass after Task 5 provides MockAdapter)**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core -- adapters::manager::tests
```

- [ ] **Step 4: Commit**

```bash
git add core/src/adapters/manager.rs
git commit -m "feat: add AdapterManager for coordinating channel adapters

Manages adapter registration, background sync loops with configurable
polling intervals, and graceful shutdown. Uses tokio tasks for
concurrent per-channel polling.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Telegram Adapter
**Files:**
- Create: `core/src/adapters/telegram.rs`

- [ ] **Step 1: Create the Telegram adapter**

Create `core/src/adapters/telegram.rs`:

```rust
use std::collections::HashMap;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use crate::error::{CoreError, Result};
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawMessage, RawAttachment};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// Telegram Bot API adapter using long-polling via `getUpdates`.
pub struct TelegramAdapter {
    client: Client,
    bot_token: Option<String>,
    last_update_id: Option<i64>,
}

impl TelegramAdapter {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            bot_token: None,
            last_update_id: None,
        }
    }

    fn api_url(&self, method: &str) -> Result<String> {
        let token = self.bot_token.as_ref().ok_or_else(|| {
            CoreError::Connection("not connected: no bot token".to_string())
        })?;
        Ok(format!("{}/bot{}/{}", TELEGRAM_API_BASE, token, method))
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn connect(&mut self, config: &ChannelConfig) -> Result<()> {
        // In production, `keychain_ref` is used to look up the token from the OS keychain.
        // For now, we store it directly (will be replaced by keychain integration in a later plan).
        self.bot_token = Some(config.keychain_ref.clone());

        // Validate the token by calling getMe
        let url = self.api_url("getMe")?;
        let resp: TelegramResponse<TelegramUser> = self
            .client
            .get(&url)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Auth(format!(
                "Telegram getMe failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        let bot = resp.result.ok_or_else(|| {
            CoreError::Auth("Telegram getMe returned no result".to_string())
        })?;

        info!(bot_username = %bot.username.unwrap_or_default(), "Telegram connected");
        Ok(())
    }

    async fn fetch_messages(&self, _since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        let mut url = self.api_url("getUpdates")?;

        // Use offset to only get new updates
        let mut params = vec![("timeout", "5".to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        if let Some(last_id) = self.last_update_id {
            params.push(("offset", (last_id + 1).to_string()));
        }

        let resp: TelegramResponse<Vec<TelegramUpdate>> = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Channel(format!(
                "getUpdates failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        let updates = resp.result.unwrap_or_default();
        let mut raw_messages = Vec::new();

        for update in &updates {
            if let Some(ref msg) = update.message {
                let sender = msg.from.as_ref();
                let sender_name = sender
                    .map(|u| {
                        let mut name = u.first_name.clone();
                        if let Some(ref last) = u.last_name {
                            name.push(' ');
                            name.push_str(last);
                        }
                        name
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let sender_address = sender
                    .and_then(|u| u.username.clone())
                    .unwrap_or_else(|| {
                        sender.map(|u| u.id.to_string()).unwrap_or_default()
                    });

                let timestamp = DateTime::from_timestamp(msg.date, 0)
                    .unwrap_or_else(Utc::now);

                let mut metadata = HashMap::new();
                metadata.insert("chat_id".to_string(), msg.chat.id.to_string());
                metadata.insert("chat_type".to_string(), msg.chat.chat_type.clone());
                metadata.insert("update_id".to_string(), update.update_id.to_string());
                if let Some(ref title) = msg.chat.title {
                    metadata.insert("chat_title".to_string(), title.clone());
                }

                raw_messages.push(RawMessage {
                    external_id: msg.message_id.to_string(),
                    channel: Channel::Telegram,
                    external_thread_id: Some(msg.chat.id.to_string()),
                    sender_name,
                    sender_address,
                    text: msg.text.clone(),
                    html: None,
                    subject: None,
                    attachments: vec![],
                    timestamp,
                    metadata,
                });
            }
        }

        // Note: last_update_id should be updated by the caller (AdapterManager)
        // after successful processing. For now we track it via metadata.
        debug!(count = raw_messages.len(), "telegram messages fetched");
        Ok(raw_messages)
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        let url = self.api_url("sendMessage")?;

        let text = content.text.as_deref().ok_or_else(|| {
            CoreError::InvalidInput("message text is required for Telegram".to_string())
        })?;

        let body = serde_json::json!({
            "chat_id": thread_id,
            "text": text,
        });

        let resp: TelegramResponse<serde_json::Value> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Channel(format!(
                "sendMessage failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        info!(chat_id = %thread_id, "telegram message sent");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.bot_token = None;
        self.last_update_id = None;
        info!("Telegram disconnected");
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        Channel::Telegram
    }
}

// --- Telegram API response types ---

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    is_bot: bool,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    date: i64,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_adapter_channel_type() {
        let adapter = TelegramAdapter::new();
        assert_eq!(adapter.channel_type(), Channel::Telegram);
    }

    #[test]
    fn test_api_url_without_token() {
        let adapter = TelegramAdapter::new();
        let result = adapter.api_url("getMe");
        assert!(result.is_err());
    }

    #[test]
    fn test_api_url_with_token() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("123:ABC".to_string());
        let url = adapter.api_url("getMe").unwrap();
        assert_eq!(url, "https://api.telegram.org/bot123:ABC/getMe");
    }

    #[tokio::test]
    async fn test_disconnect_clears_state() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("token".to_string());
        adapter.last_update_id = Some(42);

        adapter.disconnect().await.unwrap();

        assert!(adapter.bot_token.is_none());
        assert!(adapter.last_update_id.is_none());
    }

    #[tokio::test]
    async fn test_send_reply_requires_text() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("fake-token".to_string());

        let content = MessageContent {
            text: None,
            html: None,
            subject: None,
            attachments: vec![],
        };

        let result = adapter.send_reply("123", &content).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("text is required"));
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 3: Run tests**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core -- adapters::telegram::tests
```

- [ ] **Step 4: Commit**

```bash
git add core/src/adapters/telegram.rs
git commit -m "feat: add Telegram Bot API adapter

Implements ChannelAdapter for Telegram using getUpdates long-polling
for receiving messages and sendMessage for replies. Includes Telegram
API response deserialization types.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Email Adapter (IMAP/SMTP)
**Files:**
- Create: `core/src/adapters/email.rs`

- [ ] **Step 1: Create the Email adapter**

Create `core/src/adapters/email.rs`:

```rust
use std::collections::HashMap;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::error::{CoreError, Result};
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawAttachment, RawMessage};

/// Email adapter using IMAP for fetching and SMTP for sending.
pub struct EmailAdapter {
    imap_host: Option<String>,
    imap_port: u16,
    smtp_host: Option<String>,
    smtp_port: u16,
    username: Option<String>,
    password: Option<String>,
    connected: bool,
}

/// IMAP connection settings parsed from channel config metadata.
#[derive(Debug, Clone)]
pub struct ImapSettings {
    pub host: String,
    pub port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}

impl Default for ImapSettings {
    fn default() -> Self {
        Self {
            host: "imap.gmail.com".to_string(),
            port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
        }
    }
}

impl EmailAdapter {
    pub fn new() -> Self {
        Self {
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            username: None,
            password: None,
            connected: false,
        }
    }

    pub fn with_settings(settings: ImapSettings) -> Self {
        Self {
            imap_host: Some(settings.host),
            imap_port: settings.port,
            smtp_host: Some(settings.smtp_host),
            smtp_port: settings.smtp_port,
            username: None,
            password: None,
            connected: false,
        }
    }

    /// Parse IMAP settings from the channel config label.
    /// Expected format: "user@example.com" — host is derived from domain.
    /// For Gmail: imap.gmail.com / smtp.gmail.com
    /// For custom: can be overridden via ImapSettings.
    fn derive_settings(email: &str) -> ImapSettings {
        let domain = email.split('@').nth(1).unwrap_or("gmail.com");
        match domain {
            "gmail.com" | "googlemail.com" => ImapSettings {
                host: "imap.gmail.com".to_string(),
                port: 993,
                smtp_host: "smtp.gmail.com".to_string(),
                smtp_port: 587,
            },
            "outlook.com" | "hotmail.com" | "live.com" => ImapSettings {
                host: "outlook.office365.com".to_string(),
                port: 993,
                smtp_host: "smtp.office365.com".to_string(),
                smtp_port: 587,
            },
            other => ImapSettings {
                host: format!("imap.{}", other),
                port: 993,
                smtp_host: format!("smtp.{}", other),
                smtp_port: 587,
            },
        }
    }

    async fn imap_fetch_since(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<RawMessage>> {
        let host = self.imap_host.as_ref().ok_or_else(|| {
            CoreError::Connection("IMAP host not set".to_string())
        })?;
        let username = self.username.as_ref().ok_or_else(|| {
            CoreError::Connection("username not set".to_string())
        })?;
        let password = self.password.as_ref().ok_or_else(|| {
            CoreError::Connection("password not set".to_string())
        })?;

        let tls = async_native_tls::TlsConnector::new();
        let client = async_imap::connect(
            (host.as_str(), self.imap_port),
            host.as_str(),
            tls,
        )
        .await
        .map_err(|e| CoreError::Connection(format!("IMAP connect failed: {}", e)))?;

        let mut session = client
            .login(username, password)
            .await
            .map_err(|(e, _)| CoreError::Auth(format!("IMAP login failed: {}", e)))?;

        session
            .select("INBOX")
            .await
            .map_err(|e| CoreError::Channel(format!("INBOX select failed: {}", e)))?;

        // Build IMAP search query
        let search_query = if let Some(since_dt) = since {
            let date_str = since_dt.format("%d-%b-%Y").to_string();
            format!("SINCE {}", date_str)
        } else {
            // Fetch last 50 messages if no since date
            "ALL".to_string()
        };

        let uids = session
            .uid_search(&search_query)
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP search failed: {}", e)))?;

        if uids.is_empty() {
            session.logout().await.ok();
            return Ok(vec![]);
        }

        // Limit to most recent 100 UIDs to avoid overloading
        let mut uid_list: Vec<u32> = uids.into_iter().collect();
        uid_list.sort();
        let uid_list: Vec<u32> = uid_list.into_iter().rev().take(100).collect();
        let uid_range = uid_list
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_range, "RFC822")
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP fetch failed: {}", e)))?;

        use futures::TryStreamExt;
        let fetched: Vec<_> = messages_stream
            .try_collect()
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP stream failed: {}", e)))?;

        let mut raw_messages = Vec::new();

        for fetch in &fetched {
            let body = match fetch.body() {
                Some(b) => b,
                None => continue,
            };

            let parsed = match mail_parser::MessageParser::default().parse(body) {
                Some(p) => p,
                None => {
                    warn!("failed to parse email body");
                    continue;
                }
            };

            let from = parsed.from();
            let (sender_name, sender_address) = if let Some(from_list) = from {
                let addr = from_list.first();
                match addr {
                    Some(a) => (
                        a.name().unwrap_or("Unknown").to_string(),
                        a.address().unwrap_or("").to_string(),
                    ),
                    None => ("Unknown".to_string(), String::new()),
                }
            } else {
                ("Unknown".to_string(), String::new())
            };

            let message_id = parsed
                .message_id()
                .unwrap_or("")
                .to_string();

            let subject = parsed.subject().map(|s| s.to_string());

            let text_body = parsed.body_text(0).map(|t| t.to_string());
            let html_body = parsed.body_html(0).map(|h| h.to_string());

            let timestamp = parsed
                .date()
                .map(|d| {
                    DateTime::from_timestamp(d.to_timestamp(), 0)
                        .unwrap_or_else(Utc::now)
                })
                .unwrap_or_else(Utc::now);

            // Extract In-Reply-To for threading
            let in_reply_to = parsed
                .in_reply_to()
                .as_text()
                .map(|s| s.to_string());

            let references = parsed
                .references()
                .as_text()
                .map(|s| s.to_string());

            let mut metadata = HashMap::new();
            metadata.insert("message_id".to_string(), message_id.clone());
            if let Some(ref irt) = in_reply_to {
                metadata.insert("in_reply_to".to_string(), irt.clone());
            }
            if let Some(ref refs) = references {
                metadata.insert("references".to_string(), refs.clone());
            }
            if let Some(uid) = fetch.uid {
                metadata.insert("imap_uid".to_string(), uid.to_string());
            }

            let attachments: Vec<RawAttachment> = parsed
                .attachments()
                .map(|a| RawAttachment {
                    filename: a
                        .attachment_name()
                        .unwrap_or("unnamed")
                        .to_string(),
                    mime_type: a
                        .content_type()
                        .map(|ct| {
                            let main = ct.ctype();
                            let sub = ct.subtype().unwrap_or("octet-stream");
                            format!("{}/{}", main, sub)
                        })
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    size_bytes: a.len() as u64,
                })
                .collect();

            // Thread ID: use References chain root, or In-Reply-To, or Message-ID itself
            let thread_id = references
                .as_ref()
                .and_then(|r| r.split_whitespace().next().map(|s| s.to_string()))
                .or(in_reply_to.clone())
                .unwrap_or_else(|| message_id.clone());

            raw_messages.push(RawMessage {
                external_id: message_id,
                channel: Channel::Email,
                external_thread_id: Some(thread_id),
                sender_name,
                sender_address,
                text: text_body,
                html: html_body,
                subject,
                attachments,
                timestamp,
                metadata,
            });
        }

        session.logout().await.ok();
        debug!(count = raw_messages.len(), "email messages fetched via IMAP");
        Ok(raw_messages)
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    async fn connect(&mut self, config: &ChannelConfig) -> Result<()> {
        let settings = if self.imap_host.is_some() {
            // Settings already configured via with_settings()
            ImapSettings {
                host: self.imap_host.clone().unwrap_or_default(),
                port: self.imap_port,
                smtp_host: self.smtp_host.clone().unwrap_or_default(),
                smtp_port: self.smtp_port,
            }
        } else {
            Self::derive_settings(&config.label)
        };

        self.imap_host = Some(settings.host);
        self.imap_port = settings.port;
        self.smtp_host = Some(settings.smtp_host);
        self.smtp_port = settings.smtp_port;

        // In production, keychain_ref is used to look up credentials.
        // For now we use it directly as "user:password" format.
        let creds = &config.keychain_ref;
        let parts: Vec<&str> = creds.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(CoreError::Auth(
                "credentials must be in 'user:password' format".to_string(),
            ));
        }
        self.username = Some(parts[0].to_string());
        self.password = Some(parts[1].to_string());
        self.connected = true;

        info!(
            imap_host = %self.imap_host.as_deref().unwrap_or(""),
            username = %self.username.as_deref().unwrap_or(""),
            "Email adapter configured"
        );
        Ok(())
    }

    async fn fetch_messages(&self, since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if !self.connected {
            return Err(CoreError::Connection("not connected".to_string()));
        }
        self.imap_fetch_since(since).await
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        if !self.connected {
            return Err(CoreError::Connection("not connected".to_string()));
        }

        let smtp_host = self.smtp_host.as_ref().ok_or_else(|| {
            CoreError::Connection("SMTP host not set".to_string())
        })?;
        let username = self.username.as_ref().ok_or_else(|| {
            CoreError::Connection("username not set".to_string())
        })?;
        let password = self.password.as_ref().ok_or_else(|| {
            CoreError::Connection("password not set".to_string())
        })?;

        let text = content.text.as_deref().ok_or_else(|| {
            CoreError::InvalidInput("email body text is required".to_string())
        })?;

        let subject = content
            .subject
            .as_deref()
            .unwrap_or("Re:");

        // thread_id is the recipient address for email replies
        let email = lettre::Message::builder()
            .from(username.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid from address: {}", e))
            })?)
            .to(thread_id.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid to address: {}", e))
            })?)
            .subject(subject)
            .body(text.to_string())
            .map_err(|e| CoreError::Channel(format!("failed to build email: {}", e)))?;

        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
        use lettre::transport::smtp::authentication::Credentials;

        let creds = Credentials::new(username.clone(), password.clone());

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
            .map_err(|e| CoreError::Connection(format!("SMTP connect failed: {}", e)))?
            .port(self.smtp_port)
            .credentials(creds)
            .build();

        mailer
            .send(email)
            .await
            .map_err(|e| CoreError::Channel(format!("SMTP send failed: {}", e)))?;

        info!(to = %thread_id, "email sent via SMTP");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        self.username = None;
        self.password = None;
        info!("Email adapter disconnected");
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        Channel::Email
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_adapter_channel_type() {
        let adapter = EmailAdapter::new();
        assert_eq!(adapter.channel_type(), Channel::Email);
    }

    #[test]
    fn test_derive_settings_gmail() {
        let settings = EmailAdapter::derive_settings("user@gmail.com");
        assert_eq!(settings.host, "imap.gmail.com");
        assert_eq!(settings.port, 993);
        assert_eq!(settings.smtp_host, "smtp.gmail.com");
        assert_eq!(settings.smtp_port, 587);
    }

    #[test]
    fn test_derive_settings_outlook() {
        let settings = EmailAdapter::derive_settings("user@outlook.com");
        assert_eq!(settings.host, "outlook.office365.com");
        assert_eq!(settings.smtp_host, "smtp.office365.com");
    }

    #[test]
    fn test_derive_settings_custom_domain() {
        let settings = EmailAdapter::derive_settings("user@company.io");
        assert_eq!(settings.host, "imap.company.io");
        assert_eq!(settings.smtp_host, "smtp.company.io");
    }

    #[tokio::test]
    async fn test_connect_parses_credentials() {
        let mut adapter = EmailAdapter::new();
        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Email,
            label: "test@gmail.com".to_string(),
            keychain_ref: "test@gmail.com:app-password-123".to_string(),
            enabled: true,
            poll_interval_secs: 30,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        adapter.connect(&config).await.unwrap();

        assert_eq!(adapter.username.as_deref(), Some("test@gmail.com"));
        assert_eq!(adapter.password.as_deref(), Some("app-password-123"));
        assert_eq!(adapter.imap_host.as_deref(), Some("imap.gmail.com"));
        assert!(adapter.connected);
    }

    #[tokio::test]
    async fn test_connect_rejects_bad_credentials_format() {
        let mut adapter = EmailAdapter::new();
        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Email,
            label: "test@gmail.com".to_string(),
            keychain_ref: "no-colon-here".to_string(),
            enabled: true,
            poll_interval_secs: 30,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        let result = adapter.connect(&config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("user:password"));
    }

    #[tokio::test]
    async fn test_fetch_without_connect_fails() {
        let adapter = EmailAdapter::new();
        let result = adapter.fetch_messages(None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_disconnect_clears_state() {
        let mut adapter = EmailAdapter::new();
        adapter.username = Some("user".to_string());
        adapter.password = Some("pass".to_string());
        adapter.connected = true;

        adapter.disconnect().await.unwrap();

        assert!(adapter.username.is_none());
        assert!(adapter.password.is_none());
        assert!(!adapter.connected);
    }

    #[test]
    fn test_with_settings() {
        let settings = ImapSettings {
            host: "mail.custom.com".to_string(),
            port: 143,
            smtp_host: "send.custom.com".to_string(),
            smtp_port: 25,
        };
        let adapter = EmailAdapter::with_settings(settings);
        assert_eq!(adapter.imap_host.as_deref(), Some("mail.custom.com"));
        assert_eq!(adapter.imap_port, 143);
        assert_eq!(adapter.smtp_host.as_deref(), Some("send.custom.com"));
        assert_eq!(adapter.smtp_port, 25);
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 3: Run tests**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core -- adapters::email::tests
```

- [ ] **Step 4: Commit**

```bash
git add core/src/adapters/email.rs
git commit -m "feat: add Email adapter with IMAP fetch and SMTP send

Implements ChannelAdapter for Email using async-imap for fetching
messages with UID-based search, mail-parser for RFC822 parsing, and
lettre for SMTP sending. Supports Gmail, Outlook, and custom domains
with automatic IMAP/SMTP host derivation.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Mock Adapter
**Files:**
- Create: `core/src/adapters/mock.rs`

- [ ] **Step 1: Create the mock adapter**

Create `core/src/adapters/mock.rs`:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawMessage};

/// A mock adapter for testing. Stores messages in memory and records all
/// method calls for assertion.
#[derive(Clone)]
pub struct MockAdapter {
    messages: Arc<Mutex<Vec<RawMessage>>>,
    sent_replies: Arc<Mutex<Vec<SentReply>>>,
    connected: Arc<Mutex<bool>>,
    channel: Channel,
    fail_connect: Arc<Mutex<bool>>,
    fail_fetch: Arc<Mutex<bool>>,
    fail_send: Arc<Mutex<bool>>,
}

/// A recorded reply sent through the mock adapter.
#[derive(Debug, Clone)]
pub struct SentReply {
    pub thread_id: String,
    pub content: MessageContent,
    pub sent_at: DateTime<Utc>,
}

impl MockAdapter {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            sent_replies: Arc::new(Mutex::new(Vec::new())),
            connected: Arc::new(Mutex::new(false)),
            channel: Channel::Telegram,
            fail_connect: Arc::new(Mutex::new(false)),
            fail_fetch: Arc::new(Mutex::new(false)),
            fail_send: Arc::new(Mutex::new(false)),
        }
    }

    pub fn with_channel(mut self, channel: Channel) -> Self {
        self.channel = channel;
        self
    }

    /// Add a message that will be returned by `fetch_messages`.
    pub fn add_message(&self, msg: RawMessage) {
        self.messages.lock().unwrap().push(msg);
    }

    /// Add multiple messages at once.
    pub fn add_messages(&self, msgs: Vec<RawMessage>) {
        self.messages.lock().unwrap().extend(msgs);
    }

    /// Get all replies sent through this adapter.
    pub fn sent_replies(&self) -> Vec<SentReply> {
        self.sent_replies.lock().unwrap().clone()
    }

    /// Check if the adapter is currently connected.
    pub fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    /// Configure the adapter to fail on connect.
    pub fn set_fail_connect(&self, fail: bool) {
        *self.fail_connect.lock().unwrap() = fail;
    }

    /// Configure the adapter to fail on fetch.
    pub fn set_fail_fetch(&self, fail: bool) {
        *self.fail_fetch.lock().unwrap() = fail;
    }

    /// Configure the adapter to fail on send.
    pub fn set_fail_send(&self, fail: bool) {
        *self.fail_send.lock().unwrap() = fail;
    }

    /// Clear all stored messages.
    pub fn clear_messages(&self) {
        self.messages.lock().unwrap().clear();
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelAdapter for MockAdapter {
    async fn connect(&mut self, _config: &ChannelConfig) -> Result<()> {
        if *self.fail_connect.lock().unwrap() {
            return Err(crate::error::CoreError::Connection(
                "mock connect failure".to_string(),
            ));
        }
        *self.connected.lock().unwrap() = true;
        Ok(())
    }

    async fn fetch_messages(&self, since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if *self.fail_fetch.lock().unwrap() {
            return Err(crate::error::CoreError::Channel(
                "mock fetch failure".to_string(),
            ));
        }

        let messages = self.messages.lock().unwrap();
        let filtered: Vec<RawMessage> = if let Some(since_dt) = since {
            messages
                .iter()
                .filter(|m| m.timestamp > since_dt)
                .cloned()
                .collect()
        } else {
            messages.clone()
        };

        Ok(filtered)
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        if *self.fail_send.lock().unwrap() {
            return Err(crate::error::CoreError::Channel(
                "mock send failure".to_string(),
            ));
        }

        self.sent_replies.lock().unwrap().push(SentReply {
            thread_id: thread_id.to_string(),
            content: content.clone(),
            sent_at: Utc::now(),
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.connected.lock().unwrap() = false;
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        self.channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Channel;

    fn sample_raw_message(id: &str) -> RawMessage {
        RawMessage {
            external_id: id.to_string(),
            channel: Channel::Telegram,
            external_thread_id: Some("thread-1".to_string()),
            sender_name: "Test Sender".to_string(),
            sender_address: "test@example.com".to_string(),
            text: Some(format!("Message {}", id)),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_mock_connect_disconnect() {
        let mut adapter = MockAdapter::new();
        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "Test".to_string(),
            keychain_ref: "key".to_string(),
            enabled: true,
            poll_interval_secs: 5,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        assert!(!adapter.is_connected());

        adapter.connect(&config).await.unwrap();
        assert!(adapter.is_connected());

        adapter.disconnect().await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_fetch_returns_added_messages() {
        let mut adapter = MockAdapter::new();
        adapter.add_message(sample_raw_message("1"));
        adapter.add_message(sample_raw_message("2"));

        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "Test".to_string(),
            keychain_ref: "key".to_string(),
            enabled: true,
            poll_interval_secs: 5,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        adapter.connect(&config).await.unwrap();
        let messages = adapter.fetch_messages(None).await.unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn test_mock_fetch_filters_by_since() {
        let adapter = MockAdapter::new();
        let now = Utc::now();
        let old_time = now - chrono::Duration::hours(2);

        let mut old_msg = sample_raw_message("old");
        old_msg.timestamp = old_time;
        adapter.add_message(old_msg);

        let mut new_msg = sample_raw_message("new");
        new_msg.timestamp = now;
        adapter.add_message(new_msg);

        let since = now - chrono::Duration::hours(1);
        let messages = adapter.fetch_messages(Some(since)).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].external_id, "new");
    }

    #[tokio::test]
    async fn test_mock_send_reply_records() {
        let adapter = MockAdapter::new();
        let content = MessageContent {
            text: Some("Hello!".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
        };

        adapter.send_reply("thread-1", &content).await.unwrap();

        let replies = adapter.sent_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].thread_id, "thread-1");
        assert_eq!(replies[0].content.text.as_deref(), Some("Hello!"));
    }

    #[tokio::test]
    async fn test_mock_fail_connect() {
        let mut adapter = MockAdapter::new();
        adapter.set_fail_connect(true);

        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "Test".to_string(),
            keychain_ref: "key".to_string(),
            enabled: true,
            poll_interval_secs: 5,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        let result = adapter.connect(&config).await;
        assert!(result.is_err());
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_mock_fail_fetch() {
        let adapter = MockAdapter::new();
        adapter.set_fail_fetch(true);

        let result = adapter.fetch_messages(None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_fail_send() {
        let adapter = MockAdapter::new();
        adapter.set_fail_send(true);

        let content = MessageContent {
            text: Some("test".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
        };

        let result = adapter.send_reply("thread-1", &content).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_with_channel() {
        let adapter = MockAdapter::new().with_channel(Channel::Email);
        assert_eq!(adapter.channel_type(), Channel::Email);
    }

    #[tokio::test]
    async fn test_mock_clear_messages() {
        let adapter = MockAdapter::new();
        adapter.add_message(sample_raw_message("1"));
        assert_eq!(adapter.fetch_messages(None).await.unwrap().len(), 1);

        adapter.clear_messages();
        assert_eq!(adapter.fetch_messages(None).await.unwrap().len(), 0);
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo check -p messagehub-core
```

- [ ] **Step 3: Run all adapter tests**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core -- adapters
```

- [ ] **Step 4: Commit**

```bash
git add core/src/adapters/mock.rs
git commit -m "feat: add MockAdapter for testing without real services

Provides a configurable mock that stores messages in memory, records
sent replies, and supports simulating connect/fetch/send failures.
Enables integration testing of AdapterManager without network access.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Integration Tests with Mock Adapter
**Files:**
- Create: `core/tests/adapter_integration.rs`

- [ ] **Step 1: Create integration test file**

Create `core/tests/adapter_integration.rs`:

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::manager::AdapterManager;
use messagehub_core::adapters::{normalize, RawMessage, ChannelAdapter};
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};

fn make_config(channel: Channel, label: &str) -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel,
        label: label.to_string(),
        keychain_ref: "test-key".to_string(),
        enabled: true,
        poll_interval_secs: 1,
        last_sync_cursor: None,
        last_sync_at: None,
    }
}

fn make_raw_message(channel: Channel, id: &str, text: &str) -> RawMessage {
    RawMessage {
        external_id: id.to_string(),
        channel,
        external_thread_id: Some("thread-1".to_string()),
        sender_name: "Alice".to_string(),
        sender_address: "alice@example.com".to_string(),
        text: Some(text.to_string()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_full_lifecycle_with_mock() {
    // 1. Create mock adapter with some messages
    let mock = MockAdapter::new().with_channel(Channel::Email);
    mock.add_message(make_raw_message(Channel::Email, "msg-1", "Hello from email"));
    mock.add_message(make_raw_message(Channel::Email, "msg-2", "Second email"));

    // 2. Register with manager
    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = Arc::clone(&received);

    let mut manager = AdapterManager::new(move |msgs| {
        received_clone.fetch_add(msgs.len(), Ordering::Relaxed);
    });

    let config = make_config(Channel::Email, "test@example.com");
    let config_id = manager.register(Box::new(mock), config).await.unwrap();

    // 3. Start sync and let it poll
    manager.start_sync(config_id).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // 4. Verify messages were received
    assert!(received.load(Ordering::Relaxed) >= 2);

    // 5. Send a reply
    let adapter = manager.get_adapter(&config_id).unwrap();
    {
        let adapter = adapter.lock().await;
        let content = MessageContent {
            text: Some("Reply content".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
        };
        adapter.send_reply("thread-1", &content).await.unwrap();
    }

    // 6. Shutdown cleanly
    manager.shutdown().await.unwrap();
    assert_eq!(manager.registered_configs().len(), 0);
}

#[tokio::test]
async fn test_multiple_adapters_independent() {
    let email_count = Arc::new(AtomicUsize::new(0));
    let telegram_count = Arc::new(AtomicUsize::new(0));
    let email_clone = Arc::clone(&email_count);
    let telegram_clone = Arc::clone(&telegram_count);

    let mut manager = AdapterManager::new(move |msgs| {
        for msg in &msgs {
            match msg.channel {
                Channel::Email => {
                    email_clone.fetch_add(1, Ordering::Relaxed);
                }
                Channel::Telegram => {
                    telegram_clone.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    });

    // Register email adapter
    let email_mock = MockAdapter::new().with_channel(Channel::Email);
    email_mock.add_message(make_raw_message(Channel::Email, "e1", "Email 1"));
    let email_config = make_config(Channel::Email, "email");
    let email_id = manager
        .register(Box::new(email_mock), email_config)
        .await
        .unwrap();

    // Register telegram adapter
    let tg_mock = MockAdapter::new().with_channel(Channel::Telegram);
    tg_mock.add_message(make_raw_message(Channel::Telegram, "t1", "Telegram 1"));
    tg_mock.add_message(make_raw_message(Channel::Telegram, "t2", "Telegram 2"));
    let tg_config = make_config(Channel::Telegram, "telegram");
    let tg_id = manager
        .register(Box::new(tg_mock), tg_config)
        .await
        .unwrap();

    // Start both
    manager.start_sync(email_id).unwrap();
    manager.start_sync(tg_id).unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(email_count.load(Ordering::Relaxed) >= 1);
    assert!(telegram_count.load(Ordering::Relaxed) >= 2);

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_adapter_failure_does_not_crash_manager() {
    let mut manager = AdapterManager::new(|_| {});

    let failing_mock = MockAdapter::new();
    failing_mock.set_fail_fetch(true);

    let config = make_config(Channel::Telegram, "failing");
    let config_id = manager
        .register(Box::new(failing_mock), config)
        .await
        .unwrap();

    // Start sync — the fetch will fail, but the sync loop should continue
    manager.start_sync(config_id).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Manager should still be healthy
    assert!(manager.is_syncing(&config_id));

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_connect_failure_prevents_registration() {
    let mut manager = AdapterManager::new(|_| {});

    let mut failing_mock = MockAdapter::new();
    failing_mock.set_fail_connect(true);

    let config = make_config(Channel::Telegram, "fail-connect");
    let result = manager.register(Box::new(failing_mock), config).await;

    assert!(result.is_err());
    assert_eq!(manager.registered_configs().len(), 0);
}

#[tokio::test]
async fn test_normalize_roundtrip() {
    let raw = make_raw_message(Channel::Email, "ext-1", "Test content");
    let sender_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();

    let message = normalize(raw, sender_id, thread_id);

    assert_eq!(message.channel, Channel::Email);
    assert_eq!(message.sender_id, sender_id);
    assert_eq!(message.thread_id, thread_id);
    assert_eq!(message.content.text.as_deref(), Some("Test content"));
    assert!(!message.is_read);
    assert!(!message.is_archived);
    assert!(message.priority.is_none());
}

#[tokio::test]
async fn test_mock_adapter_trait_object() {
    // Verify MockAdapter works as a trait object (dyn ChannelAdapter)
    let mock = MockAdapter::new().with_channel(Channel::Sms);
    let mut adapter: Box<dyn ChannelAdapter> = Box::new(mock);

    let config = make_config(Channel::Sms, "sms-test");
    adapter.connect(&config).await.unwrap();

    assert_eq!(adapter.channel_type(), Channel::Sms);

    let messages = adapter.fetch_messages(None).await.unwrap();
    assert!(messages.is_empty());

    adapter.disconnect().await.unwrap();
}
```

- [ ] **Step 2: Add `futures` to dev-dependencies for IMAP stream collection**

The email adapter uses `futures::TryStreamExt`. Add to `core/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

And add `futures` to the main dependencies (needed by `async-imap` stream handling):

```toml
futures = "0.3"
```

Add this line to the `[dependencies]` section of `core/Cargo.toml`.

- [ ] **Step 3: Run all tests**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo test -p messagehub-core
```

- [ ] **Step 4: Verify no warnings**

```bash
cd /home/jocelyn/Applications/MessageHub && cargo clippy -p messagehub-core -- -D warnings 2>&1 | head -50
```

- [ ] **Step 5: Commit**

```bash
git add core/tests/adapter_integration.rs core/Cargo.toml
git commit -m "feat: add integration tests for adapter system with mock adapter

Tests cover full lifecycle (register, sync, send, shutdown), multiple
independent adapters, failure isolation, connect failure handling,
normalization roundtrip, and trait object compatibility.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```
