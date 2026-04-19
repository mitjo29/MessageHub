use serde::{Deserialize, Serialize};

/// Per-channel health state. Persisted to `channels.status` as a lowercase string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelStatus {
    #[default]
    Healthy,
    /// Channel has failed 1..=3 times in a row.
    ///
    /// `attempt` mirrors `channels.consecutive_failures` in the DB and is
    /// populated from that column on load — it is not a separately-persisted
    /// field. Callers persisting `Degraded` must pass the matching
    /// `consecutive_failures` value to `Store::update_channel_status`.
    Degraded { attempt: u32 },
    Failed { last_error: String },
}

impl ChannelStatus {
    pub fn db_str(&self) -> &'static str {
        match self {
            ChannelStatus::Healthy => "healthy",
            ChannelStatus::Degraded { .. } => "degraded",
            ChannelStatus::Failed { .. } => "failed",
        }
    }
}
