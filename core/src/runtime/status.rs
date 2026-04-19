use serde::{Deserialize, Serialize};

/// Per-channel health state. Persisted to `channels.status` as a lowercase string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChannelStatus {
    #[default]
    Healthy,
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
