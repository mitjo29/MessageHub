use rusqlite::params;
use uuid::Uuid;

use crate::error::Result;
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_contact(&self, contact: &Contact) -> Result<()> {
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
                format!("{:?}", identity.channel),
                identity.address,
            ],
        )?;
        Ok(())
    }

    pub fn insert_thread(&self, thread: &crate::types::Thread) -> Result<()> {
        self.conn().execute(
            "INSERT INTO threads (id, channel_type, subject, message_count, last_message_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                thread.id.to_string(),
                format!("{:?}", thread.channel),
                thread.subject,
                thread.message_count,
                thread.last_message_at.to_rfc3339(),
                thread.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }
}
