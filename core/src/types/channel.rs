use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Channel {
    Email,
    Sms,
    WhatsApp,
    Teams,
    Telegram,
}

impl Channel {
    pub fn display_name(&self) -> &'static str {
        match self {
            Channel::Email => "Email",
            Channel::Sms => "SMS",
            Channel::WhatsApp => "WhatsApp",
            Channel::Teams => "Teams",
            Channel::Telegram => "Telegram",
        }
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for a connected channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub id: uuid::Uuid,
    pub channel: Channel,
    pub label: String,
    /// Reference to OS keychain entry (not the secret itself).
    pub keychain_ref: String,
    pub enabled: bool,
    pub poll_interval_secs: u32,
    pub last_sync_cursor: Option<String>,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
}
