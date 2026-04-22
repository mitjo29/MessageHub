use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    pub channel: Option<Channel>,
    pub unread_only: bool,
    /// Inclusive floor on `priority_score`. `None` = any priority (including unset).
    pub min_priority: Option<u8>,
    pub archived: bool,
}

impl Store {
    pub fn insert_message(&self, msg: &Message) -> Result<()> {
        let attachments_json = serde_json::to_string(&msg.content.attachments)?;
        let metadata_json = serde_json::to_string(&msg.metadata)?;

        self.conn().execute(
            "INSERT INTO messages (id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived, external_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(channel_type, external_id) WHERE external_id IS NOT NULL DO NOTHING",
            params![
                msg.id.to_string(),
                msg.channel.to_db_str(),
                msg.thread_id.to_string(),
                msg.sender_id.to_string(),
                msg.content.text,
                msg.content.html,
                msg.content.subject,
                attachments_json,
                msg.timestamp.to_rfc3339(),
                metadata_json,
                msg.priority.map(|p| p.value() as i32),
                msg.category,
                msg.is_read as i32,
                msg.is_archived as i32,
                msg.external_id,
            ],
        )?;
        Ok(())
    }

    pub fn get_message(&self, id: &Uuid) -> Result<Message> {
        let id_str = id.to_string();
        let result = self.conn().query_row(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived, external_id FROM messages WHERE id = ?1",
            [&id_str],
            |row| {
                Ok(row_to_message(row))
            },
        );

        match result {
            Ok(inner) => inner,
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(CoreError::NotFound {
                entity: "message".into(),
                id: id_str,
            }),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    pub fn list_messages(
        &self,
        filter: &MessageFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        let (where_sql, mut params_vec) = build_where_clause(filter);
        let mut sql = String::from(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, \
             content_subject, attachments_json, timestamp, metadata_json, priority_score, \
             category, is_read, is_archived, external_id FROM messages"
        );
        sql.push_str(&where_sql);

        let limit_idx = params_vec.len() + 1;
        sql.push_str(&format!(
            " ORDER BY timestamp DESC LIMIT ?{} OFFSET ?{}",
            limit_idx,
            limit_idx + 1
        ));
        params_vec.push(Box::new(limit));
        params_vec.push(Box::new(offset));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn().prepare(&sql)?;
        let messages: Vec<Message> = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_message(row)))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
            .into_iter()
            .collect::<std::result::Result<Vec<_>, CoreError>>()?;
        Ok(messages)
    }

    pub fn count_messages(&self, filter: &MessageFilter) -> Result<u64> {
        let (where_sql, params_vec) = build_where_clause(filter);
        let sql = format!("SELECT COUNT(*) FROM messages{}", where_sql);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let n: i64 = self
            .conn()
            .query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(n as u64)
    }

    /// Return every message in a thread, oldest first.
    ///
    /// Ordering is `timestamp ASC` so the conversation reads naturally
    /// top-to-bottom when rendered into a prompt. The `limit` caps the
    /// oldest-N returned (not the newest-N) — use a high value if you
    /// want the whole thread, or truncate at the call site if you need
    /// "last N".
    pub fn list_messages_in_thread(&self, thread_id: &Uuid, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html,
                    content_subject, attachments_json, timestamp, metadata_json,
                    priority_score, category, is_read, is_archived, external_id
             FROM messages
             WHERE thread_id = ?1
             ORDER BY timestamp ASC
             LIMIT ?2",
        )?;
        let messages: Vec<Message> = stmt
            .query_map(params![thread_id.to_string(), limit], |row| {
                Ok(row_to_message(row))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
            .into_iter()
            .collect::<std::result::Result<Vec<_>, CoreError>>()?;
        Ok(messages)
    }

    pub fn mark_read(&self, id: &Uuid, read: bool) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE messages SET is_read = ?1 WHERE id = ?2",
            params![read as i32, id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "message".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    pub fn set_message_classification(
        &self,
        id: &uuid::Uuid,
        category: Option<&str>,
        priority: Option<crate::types::PriorityScore>,
    ) -> Result<()> {
        let priority_score: Option<i32> = priority.map(|p| p.value() as i32);
        self.conn().execute(
            "UPDATE messages SET priority_score = ?1, category = ?2 WHERE id = ?3",
            params![priority_score, category, id.to_string()],
        )?;
        Ok(())
    }

    pub fn search_messages(&self, query: &str, limit: u32) -> Result<Vec<Message>> {
        // Escape double quotes and wrap in quotes for FTS5 phrase search to prevent syntax injection
        let escaped = query.replace('"', "\"\"");
        let fts_query = format!("\"{}\"", escaped);
        let mut stmt = self.conn().prepare(
            "SELECT m.id, m.channel_type, m.thread_id, m.sender_id, m.content_text, m.content_html, m.content_subject, m.attachments_json, m.timestamp, m.metadata_json, m.priority_score, m.category, m.is_read, m.is_archived, m.external_id
             FROM messages_fts fts
             JOIN messages m ON m.rowid = fts.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let messages: Vec<Message> = stmt
            .query_map(params![fts_query, limit], |row| Ok(row_to_message(row)))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
            .into_iter()
            .collect::<std::result::Result<Vec<_>, CoreError>>()?;
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_set_message_classification_updates_row() {
        use crate::types::{Channel, Message, MessageContent, PriorityScore, Thread};
        use std::collections::HashMap;

        let store = crate::store::Store::open_in_memory().unwrap();
        let contact = store
            .find_or_create_contact_by_address(Channel::Telegram, "u1", "User")
            .unwrap();
        let thread_id = uuid::Uuid::new_v4();
        store.insert_thread(&Thread {
            id: thread_id,
            channel: Channel::Telegram,
            subject: None,
            participant_ids: vec![],
            message_count: 0,
            last_message_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            external_thread_id: None,
        }).unwrap();

        let msg = Message {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Telegram,
            thread_id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some("Hi".to_string()),
                html: None,
                subject: None,
                attachments: vec![],
                reply_headers: None,
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
            external_id: None,
        };
        store.insert_message(&msg).unwrap();

        store.set_message_classification(&msg.id, Some("work"), Some(PriorityScore::new(4).unwrap()))
            .unwrap();

        let reloaded = store.get_message(&msg.id).unwrap();
        assert_eq!(reloaded.category.as_deref(), Some("work"));
        assert_eq!(reloaded.priority, Some(PriorityScore::new(4).unwrap()));
    }
}

/// Build a WHERE clause and its bound params from a MessageFilter.
///
/// The returned SQL starts with " WHERE is_archived = ?1" and appends
/// additional AND clauses as needed; the caller concatenates SELECT/ORDER
/// BY/LIMIT around it. Positional params use `?N` indexed from 1 based on
/// insertion order.
fn build_where_clause(
    filter: &MessageFilter,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut sql = String::from(" WHERE is_archived = ?1");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(filter.archived as i32)];

    if let Some(ch) = filter.channel {
        params.push(Box::new(ch.to_db_str().to_owned()));
        sql.push_str(&format!(" AND channel_type = ?{}", params.len()));
    }
    if filter.unread_only {
        sql.push_str(" AND is_read = 0");
    }
    if let Some(min_p) = filter.min_priority {
        params.push(Box::new(min_p as i32));
        sql.push_str(&format!(
            " AND priority_score IS NOT NULL AND priority_score >= ?{}",
            params.len()
        ));
    }

    (sql, params)
}

fn row_to_message(row: &rusqlite::Row) -> std::result::Result<Message, CoreError> {
    let id_str: String = row.get(0).map_err(CoreError::Database)?;
    let channel_str: String = row.get(1).map_err(CoreError::Database)?;
    let thread_str: String = row.get(2).map_err(CoreError::Database)?;
    let sender_str: String = row.get(3).map_err(CoreError::Database)?;
    let content_text: Option<String> = row.get(4).map_err(CoreError::Database)?;
    let content_html: Option<String> = row.get(5).map_err(CoreError::Database)?;
    let content_subject: Option<String> = row.get(6).map_err(CoreError::Database)?;
    let attachments_json: Option<String> = row.get(7).map_err(CoreError::Database)?;
    let timestamp_str: String = row.get(8).map_err(CoreError::Database)?;
    let metadata_json: Option<String> = row.get(9).map_err(CoreError::Database)?;
    let priority_val: Option<i32> = row.get(10).map_err(CoreError::Database)?;
    let category: Option<String> = row.get(11).map_err(CoreError::Database)?;
    let is_read: i32 = row.get(12).map_err(CoreError::Database)?;
    let is_archived: i32 = row.get(13).map_err(CoreError::Database)?;
    let external_id: Option<String> = row.get(14).map_err(CoreError::Database)?;

    let channel = Channel::from_db_str(&channel_str).ok_or_else(|| {
        CoreError::InvalidInput(format!("unknown channel: {}", channel_str))
    })?;

    let attachments: Vec<Attachment> = attachments_json
        .map(|j| serde_json::from_str(&j).unwrap_or_default())
        .unwrap_or_default();

    let metadata: std::collections::HashMap<String, String> = metadata_json
        .map(|j| serde_json::from_str(&j).unwrap_or_default())
        .unwrap_or_default();

    Ok(Message {
        id: Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        channel,
        thread_id: Uuid::parse_str(&thread_str)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        sender_id: Uuid::parse_str(&sender_str)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        content: MessageContent {
            text: content_text,
            html: content_html,
            subject: content_subject,
            attachments,
            reply_headers: None,
        },
        timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
            .with_timezone(&chrono::Utc),
        metadata,
        priority: priority_val.and_then(|v| PriorityScore::new(v as u8)),
        category,
        is_read: is_read != 0,
        is_archived: is_archived != 0,
        external_id,
    })
}
