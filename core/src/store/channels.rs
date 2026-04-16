use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_channel_config(&self, config: &ChannelConfig) -> Result<()> {
        self.conn().execute(
            "INSERT INTO channels (id, channel_type, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                config.id.to_string(),
                format!("{:?}", config.channel),
                config.label,
                config.keychain_ref,
                config.enabled as i32,
                config.poll_interval_secs,
                config.last_sync_cursor,
                config.last_sync_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn list_channel_configs(&self) -> Result<Vec<ChannelConfig>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, channel_type, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at FROM channels"
        )?;
        let configs = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let channel_str: String = row.get(1)?;
                let label: String = row.get(2)?;
                let keychain_ref: String = row.get(3)?;
                let enabled: i32 = row.get(4)?;
                let poll_interval_secs: u32 = row.get(5)?;
                let last_sync_cursor: Option<String> = row.get(6)?;
                let last_sync_at_str: Option<String> = row.get(7)?;

                Ok((id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at_str))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at_str)| {
                let channel = match channel_str.as_str() {
                    "Email" => Channel::Email,
                    "Sms" => Channel::Sms,
                    "WhatsApp" => Channel::WhatsApp,
                    "Teams" => Channel::Teams,
                    "Telegram" => Channel::Telegram,
                    _ => return None,
                };
                let last_sync_at = last_sync_at_str
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc));

                Some(ChannelConfig {
                    id: Uuid::parse_str(&id_str).ok()?,
                    channel,
                    label,
                    keychain_ref,
                    enabled: enabled != 0,
                    poll_interval_secs,
                    last_sync_cursor,
                    last_sync_at,
                })
            })
            .collect();
        Ok(configs)
    }

    pub fn update_sync_state(&self, channel_id: &Uuid, cursor: Option<&str>, synced_at: chrono::DateTime<chrono::Utc>) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE channels SET last_sync_cursor = ?1, last_sync_at = ?2, updated_at = ?3 WHERE id = ?4",
            params![cursor, synced_at.to_rfc3339(), synced_at.to_rfc3339(), channel_id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound { entity: "channel".into(), id: channel_id.to_string() });
        }
        Ok(())
    }
}
