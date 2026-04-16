use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;
use crate::types::*;

impl Store {
    pub fn insert_message(&self, msg: &Message) -> Result<()> {
        let attachments_json = serde_json::to_string(&msg.content.attachments)?;
        let metadata_json = serde_json::to_string(&msg.metadata)?;

        self.conn().execute(
            "INSERT INTO messages (id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                msg.id.to_string(),
                format!("{:?}", msg.channel),
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
            ],
        )?;
        Ok(())
    }

    pub fn get_message(&self, id: &Uuid) -> Result<Message> {
        let id_str = id.to_string();
        let result = self.conn().query_row(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived FROM messages WHERE id = ?1",
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
        channel: Option<Channel>,
        archived: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        let mut sql = String::from(
            "SELECT id, channel_type, thread_id, sender_id, content_text, content_html, content_subject, attachments_json, timestamp, metadata_json, priority_score, category, is_read, is_archived FROM messages WHERE is_archived = ?1"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(archived as i32)];

        if let Some(ch) = channel {
            sql.push_str(" AND channel_type = ?2");
            params_vec.push(Box::new(format!("{:?}", ch)));
        }

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
        let messages = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_message(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
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

    pub fn search_messages(&self, query: &str, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self.conn().prepare(
            "SELECT m.id, m.channel_type, m.thread_id, m.sender_id, m.content_text, m.content_html, m.content_subject, m.attachments_json, m.timestamp, m.metadata_json, m.priority_score, m.category, m.is_read, m.is_archived
             FROM messages_fts fts
             JOIN messages m ON m.rowid = fts.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let messages = stmt
            .query_map(params![query, limit], |row| Ok(row_to_message(row)))?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(messages)
    }
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

    let channel = match channel_str.as_str() {
        "Email" => Channel::Email,
        "Sms" => Channel::Sms,
        "WhatsApp" => Channel::WhatsApp,
        "Teams" => Channel::Teams,
        "Telegram" => Channel::Telegram,
        _ => {
            return Err(CoreError::InvalidInput(format!(
                "unknown channel: {}",
                channel_str
            )))
        }
    };

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
        },
        timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
            .with_timezone(&chrono::Utc),
        metadata,
        priority: priority_val.and_then(|v| PriorityScore::new(v as u8)),
        category,
        is_read: is_read != 0,
        is_archived: is_archived != 0,
    })
}
