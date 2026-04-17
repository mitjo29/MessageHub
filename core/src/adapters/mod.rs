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
