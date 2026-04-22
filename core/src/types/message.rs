use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub channel: Channel,
    pub thread_id: Uuid,
    pub sender_id: Uuid,
    pub content: MessageContent,
    pub timestamp: DateTime<Utc>,
    pub metadata: std::collections::HashMap<String, String>,
    pub priority: Option<PriorityScore>,
    pub category: Option<String>,
    pub is_read: bool,
    pub is_archived: bool,
    #[serde(default)]
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContent {
    pub text: Option<String>,
    pub html: Option<String>,
    pub subject: Option<String>,
    pub attachments: Vec<Attachment>,
    /// Set on outbound messages the Tauri layer produces for replies;
    /// always `None` for messages ingested from adapters.
    #[serde(default)]
    pub reply_headers: Option<ReplyHeaders>,
}

/// Threading metadata for an outbound reply. When present, the email adapter
/// renders `In-Reply-To` / `References` / a fresh `Message-ID` so the reply
/// threads in the recipient's client. Inbound messages always have this as
/// `None`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ReplyHeaders {
    /// RFC 5322 destination address, typically the original sender.
    pub to: String,
    /// The original message's Message-ID (bare — no `<...>` wrapping needed).
    pub in_reply_to: String,
    /// Existing References chain + the original's Message-ID appended. Bare
    /// ids, one per Vec entry; the adapter wraps each in `<...>`.
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PriorityScore(u8);

impl PriorityScore {
    pub fn new(score: u8) -> Option<Self> {
        if (1..=5).contains(&score) {
            Some(Self(score))
        } else {
            None
        }
    }

    pub fn value(&self) -> u8 {
        self.0
    }
}
