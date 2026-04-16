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
                config.channel.to_db_str(),
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
        let rows: Vec<(String, String, String, String, i32, u32, Option<String>, Option<String>)> = stmt
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
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        let mut configs = Vec::with_capacity(rows.len());
        for (id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs, last_sync_cursor, last_sync_at_str) in rows {
            let channel = Channel::from_db_str(&channel_str).ok_or_else(|| {
                CoreError::InvalidInput(format!("unknown channel: {}", channel_str))
            })?;
            let last_sync_at = last_sync_at_str
                .map(|s| chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .map_err(|e| CoreError::InvalidInput(e.to_string())))
                .transpose()?;

            configs.push(ChannelConfig {
                id: Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                channel,
                label,
                keychain_ref,
                enabled: enabled != 0,
                poll_interval_secs,
                last_sync_cursor,
                last_sync_at,
            });
        }
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
