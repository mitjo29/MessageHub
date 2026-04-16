use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::channel::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: Uuid,
    pub channel: Channel,
    pub subject: Option<String>,
    pub participant_ids: Vec<Uuid>,
    pub message_count: u32,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
