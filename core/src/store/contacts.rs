use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_contact(&self, contact: &Contact) -> Result<()> {
        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.insert_contact_inner(contact);
        match &result {
            Ok(_) => self.conn().execute_batch("COMMIT;")?,
            Err(_) => { let _ = self.conn().execute_batch("ROLLBACK;"); }
        }
        result
    }

    fn insert_contact_inner(&self, contact: &Contact) -> Result<()> {
        self.conn().execute(
            "INSERT INTO contacts (id, display_name, vault_ref, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                contact.id.to_string(),
                contact.display_name,
                contact.vault_ref,
                contact.created_at.to_rfc3339(),
                contact.updated_at.to_rfc3339(),
            ],
        )?;
        for identity in &contact.identities {
            self.add_identity(&contact.id, identity)?;
        }
        Ok(())
    }

    pub fn add_identity(&self, contact_id: &Uuid, identity: &ContactIdentity) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO contact_identities (contact_id, channel_type, address) VALUES (?1, ?2, ?3)",
            params![
                contact_id.to_string(),
                identity.channel.to_db_str(),
                identity.address,
            ],
        )?;
        Ok(())
    }

    pub fn get_contact(&self, id: &Uuid) -> Result<Contact> {
        let id_str = id.to_string();
        let (display_name, vault_ref, created_at_str, updated_at_str): (String, Option<String>, String, String) =
            self.conn().query_row(
                "SELECT display_name, vault_ref, created_at, updated_at FROM contacts WHERE id = ?1",
                [&id_str],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|_| CoreError::NotFound { entity: "contact".into(), id: id_str.clone() })?;

        let identities = self.get_identities(id)?;

        Ok(Contact {
            id: *id,
            display_name,
            identities,
            vault_ref,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                .with_timezone(&chrono::Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                .with_timezone(&chrono::Utc),
        })
    }

    pub fn find_contact_by_address(&self, channel: Channel, address: &str) -> Result<Option<Contact>> {
        let channel_str = channel.to_db_str();
        let result = self.conn().query_row(
            "SELECT contact_id FROM contact_identities WHERE channel_type = ?1 AND address = ?2",
            params![channel_str, address],
            |row| {
                let id_str: String = row.get(0)?;
                Ok(id_str)
            },
        );

        match result {
            Ok(id_str) => {
                let id = Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?;
                Ok(Some(self.get_contact(&id)?))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    fn get_identities(&self, contact_id: &Uuid) -> Result<Vec<ContactIdentity>> {
        let mut stmt = self.conn().prepare(
            "SELECT channel_type, address FROM contact_identities WHERE contact_id = ?1"
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map([contact_id.to_string()], |row| {
                let channel_str: String = row.get(0)?;
                let address: String = row.get(1)?;
                Ok((channel_str, address))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        let mut identities = Vec::with_capacity(rows.len());
        for (ch, addr) in rows {
            let channel = Channel::from_db_str(&ch).ok_or_else(|| {
                CoreError::InvalidInput(format!("unknown channel: {}", ch))
            })?;
            identities.push(ContactIdentity { channel, address: addr });
        }
        Ok(identities)
    }

    pub fn find_thread_by_external_id(
        &self,
        channel: crate::types::Channel,
        external_id: &str,
    ) -> Result<Option<crate::types::Thread>> {
        use crate::types::Thread;
        use rusqlite::OptionalExtension;

        let row = self.conn().query_row(
            "SELECT id, channel_type, subject, message_count, last_message_at, created_at, external_thread_id \
             FROM threads \
             WHERE channel_type = ?1 AND external_thread_id = ?2 \
             LIMIT 1",
            params![channel.to_db_str(), external_id],
            |row| {
                let id_str: String = row.get(0)?;
                let channel_str: String = row.get(1)?;
                let subject: Option<String> = row.get(2)?;
                let message_count: u32 = row.get(3)?;
                let last_at: String = row.get(4)?;
                let created_at: String = row.get(5)?;
                let ext: Option<String> = row.get(6)?;
                Ok((id_str, channel_str, subject, message_count, last_at, created_at, ext))
            },
        ).optional()?;

        let Some((id_str, channel_str, subject, message_count, last_at, created_at, ext)) = row else {
            return Ok(None);
        };

        let id = uuid::Uuid::parse_str(&id_str)
            .map_err(|e| CoreError::InvalidInput(format!("bad thread id: {}", e)))?;
        let channel = crate::types::Channel::from_db_str(&channel_str)
            .ok_or_else(|| CoreError::InvalidInput(format!("unknown channel: {}", channel_str)))?;
        let last_message_at = chrono::DateTime::parse_from_rfc3339(&last_at)
            .map_err(|e| CoreError::InvalidInput(format!("bad last_message_at: {}", e)))?
            .with_timezone(&chrono::Utc);
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| CoreError::InvalidInput(format!("bad created_at: {}", e)))?
            .with_timezone(&chrono::Utc);

        Ok(Some(Thread {
            id,
            channel,
            subject,
            participant_ids: vec![],
            message_count,
            last_message_at,
            created_at,
            external_thread_id: ext,
        }))
    }

    pub fn find_or_create_contact_by_address(
        &self,
        channel: crate::types::Channel,
        address: &str,
        display_name: &str,
    ) -> Result<crate::types::Contact> {
        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.find_or_create_contact_by_address_inner(channel, address, display_name);
        match &result {
            Ok(_) => self.conn().execute_batch("COMMIT;")?,
            Err(_) => { let _ = self.conn().execute_batch("ROLLBACK;"); }
        }
        result
    }

    fn find_or_create_contact_by_address_inner(
        &self,
        channel: crate::types::Channel,
        address: &str,
        display_name: &str,
    ) -> Result<crate::types::Contact> {
        use crate::types::{Contact, ContactIdentity};

        if let Some(existing) = self.find_contact_by_address(channel, address)? {
            return Ok(existing);
        }

        let contact = Contact {
            id: uuid::Uuid::new_v4(),
            display_name: display_name.to_string(),
            vault_ref: None,
            identities: vec![ContactIdentity {
                channel,
                address: address.to_string(),
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        self.insert_contact_inner(&contact)?;
        Ok(contact)
    }

    pub fn insert_thread(&self, thread: &crate::types::Thread) -> Result<()> {
        self.conn().execute(
            "INSERT INTO threads (id, channel_type, subject, message_count, last_message_at, created_at, external_thread_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                thread.id.to_string(),
                thread.channel.to_db_str(),
                thread.subject,
                thread.message_count,
                thread.last_message_at.to_rfc3339(),
                thread.created_at.to_rfc3339(),
                thread.external_thread_id,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_find_thread_by_external_id_roundtrip() {
        use crate::types::{Channel, Thread};
        use chrono::Utc;
        use uuid::Uuid;

        let store = crate::store::Store::open_in_memory().unwrap();
        let thread = Thread {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            subject: None,
            participant_ids: vec![],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: Some("chat-42".to_string()),
        };
        store.insert_thread(&thread).unwrap();

        let hit = store
            .find_thread_by_external_id(Channel::Telegram, "chat-42")
            .unwrap();
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().id, thread.id);

        let miss = store
            .find_thread_by_external_id(Channel::Telegram, "chat-99")
            .unwrap();
        assert!(miss.is_none());

        let wrong_channel = store
            .find_thread_by_external_id(Channel::Email, "chat-42")
            .unwrap();
        assert!(wrong_channel.is_none());
    }

    #[test]
    fn test_find_or_create_contact_by_address_creates_then_finds() {
        use crate::types::Channel;

        let store = crate::store::Store::open_in_memory().unwrap();

        let a = store
            .find_or_create_contact_by_address(Channel::Telegram, "alice_bot", "Alice")
            .unwrap();
        let b = store
            .find_or_create_contact_by_address(Channel::Telegram, "alice_bot", "Alice-renamed")
            .unwrap();

        assert_eq!(a.id, b.id, "second call should return the same contact");
        // Display name is set only on creation; second call does not overwrite.
        assert_eq!(a.display_name, "Alice");
    }
}
