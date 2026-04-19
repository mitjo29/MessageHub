use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_channel_config(&self, config: &ChannelConfig) -> Result<()> {
        self.conn().execute(
            "INSERT INTO channels (id, channel_type, label, keychain_ref, enabled, \
                                   poll_interval_secs, last_sync_cursor, last_sync_at, \
                                   status, last_error, consecutive_failures) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                config.id.to_string(),
                config.channel.to_db_str(),
                config.label,
                config.keychain_ref,
                config.enabled as i32,
                config.poll_interval_secs,
                config.last_sync_cursor,
                config.last_sync_at.map(|t| t.to_rfc3339()),
                config.status.db_str(),
                config.last_error,
                config.consecutive_failures,
            ],
        )?;
        Ok(())
    }

    pub fn list_channel_configs(&self) -> Result<Vec<ChannelConfig>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, channel_type, label, keychain_ref, enabled, poll_interval_secs, \
                    last_sync_cursor, last_sync_at, status, last_error, consecutive_failures \
             FROM channels"
        )?;
        let rows = stmt
            .query_map([], |row| {
                let id_str: String = row.get(0)?;
                let channel_str: String = row.get(1)?;
                let label: String = row.get(2)?;
                let keychain_ref: String = row.get(3)?;
                let enabled: i32 = row.get(4)?;
                let poll_interval_secs: u32 = row.get(5)?;
                let last_sync_cursor: Option<String> = row.get(6)?;
                let last_sync_at_str: Option<String> = row.get(7)?;
                let status_str: String = row.get(8)?;
                let last_error: Option<String> = row.get(9)?;
                let consecutive_failures: u32 = row.get(10)?;
                Ok((id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs,
                    last_sync_cursor, last_sync_at_str, status_str, last_error, consecutive_failures))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        let mut configs = Vec::with_capacity(rows.len());
        for (id_str, channel_str, label, keychain_ref, enabled, poll_interval_secs,
             last_sync_cursor, last_sync_at_str, status_str, last_error, consecutive_failures) in rows
        {
            let channel = Channel::from_db_str(&channel_str).ok_or_else(|| {
                CoreError::InvalidInput(format!("unknown channel: {}", channel_str))
            })?;
            let last_sync_at = last_sync_at_str
                .map(|s| chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .map_err(|e| CoreError::InvalidInput(e.to_string())))
                .transpose()?;
            let status = match status_str.as_str() {
                "healthy"  => crate::runtime::status::ChannelStatus::Healthy,
                "degraded" => {
                    if consecutive_failures == 0 {
                        return Err(CoreError::Database(rusqlite::Error::InvalidParameterName(
                            "channel row is 'degraded' but consecutive_failures = 0".to_string(),
                        )));
                    }
                    crate::runtime::status::ChannelStatus::Degraded { attempt: consecutive_failures }
                }
                "failed"   => crate::runtime::status::ChannelStatus::Failed {
                    last_error: last_error.clone().unwrap_or_default(),
                },
                other => return Err(CoreError::InvalidInput(
                    format!("unknown channel status: {}", other),
                )),
            };

            configs.push(ChannelConfig {
                id: Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                channel,
                label,
                keychain_ref,
                enabled: enabled != 0,
                poll_interval_secs,
                last_sync_cursor,
                last_sync_at,
                status,
                last_error,
                consecutive_failures,
            });
        }
        Ok(configs)
    }

    pub fn update_channel_status(
        &self,
        id: &uuid::Uuid,
        status: &crate::runtime::status::ChannelStatus,
        consecutive_failures: u32,
    ) -> Result<()> {
        let last_error = match status {
            crate::runtime::status::ChannelStatus::Failed { last_error } => Some(last_error.as_str()),
            _ => None,
        };
        let rows = self.conn().execute(
            "UPDATE channels SET status = ?1, last_error = ?2, consecutive_failures = ?3 WHERE id = ?4",
            params![status.db_str(), last_error, consecutive_failures, id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound { entity: "channel".to_string(), id: id.to_string() });
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_update_channel_status_persists_and_reloads() {
        use crate::runtime::status::ChannelStatus;
        use crate::types::{Channel, ChannelConfig};

        let store = crate::store::Store::open_in_memory().unwrap();
        let id = uuid::Uuid::new_v4();
        store.insert_channel_config(&ChannelConfig {
            id,
            channel: Channel::Telegram,
            label: "t".to_string(),
            keychain_ref: "ref".to_string(),
            enabled: true,
            poll_interval_secs: 30,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        }).unwrap();

        store.update_channel_status(
            &id,
            &ChannelStatus::Failed { last_error: "boom".to_string() },
            3,
        ).unwrap();

        let cfgs = store.list_channel_configs().unwrap();
        let cfg = cfgs.iter().find(|c| c.id == id).unwrap();
        assert_eq!(cfg.status, ChannelStatus::Failed { last_error: "boom".to_string() });
        assert_eq!(cfg.last_error.as_deref(), Some("boom"));
        assert_eq!(cfg.consecutive_failures, 3);
    }

    #[test]
    fn test_update_channel_status_unknown_id_errors() {
        use crate::runtime::status::ChannelStatus;
        let store = crate::store::Store::open_in_memory().unwrap();
        let result = store.update_channel_status(
            &uuid::Uuid::new_v4(),
            &ChannelStatus::Healthy,
            0,
        );
        assert!(result.is_err(), "update on unknown id must return Err");
    }
}
