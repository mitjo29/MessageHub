use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct ReplyDraft {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: String,
    pub subject: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Borrowed payload for `upsert_reply_draft` — avoids string allocations at
/// the call site on every autosave tick.
#[derive(Debug, Clone)]
pub struct NewReplyDraft<'a> {
    pub thread_id: Uuid,
    pub in_reply_to_message_id: Uuid,
    pub body: &'a str,
    pub subject: Option<&'a str>,
}

impl Store {
    pub fn upsert_reply_draft(&self, draft: &NewReplyDraft<'_>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO reply_drafts
                (thread_id, in_reply_to_message_id, body, subject, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT(thread_id) DO UPDATE SET
                 in_reply_to_message_id = excluded.in_reply_to_message_id,
                 body = excluded.body,
                 subject = excluded.subject,
                 updated_at = excluded.updated_at",
            params![
                draft.thread_id.to_string(),
                draft.in_reply_to_message_id.to_string(),
                draft.body,
                draft.subject,
            ],
        )?;
        Ok(())
    }

    pub fn get_reply_draft(&self, thread_id: &Uuid) -> Result<Option<ReplyDraft>> {
        let row = self
            .conn()
            .query_row(
                "SELECT thread_id, in_reply_to_message_id, body, subject, updated_at
                 FROM reply_drafts
                 WHERE thread_id = ?1",
                params![thread_id.to_string()],
                |row| {
                    let thread: String = row.get(0)?;
                    let irt: String = row.get(1)?;
                    let body: String = row.get(2)?;
                    let subject: Option<String> = row.get(3)?;
                    let updated_at: String = row.get(4)?;
                    Ok((thread, irt, body, subject, updated_at))
                },
            )
            .optional()?;

        match row {
            None => Ok(None),
            Some((thread, irt, body, subject, updated_at)) => Ok(Some(ReplyDraft {
                thread_id: Uuid::parse_str(&thread)
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                in_reply_to_message_id: Uuid::parse_str(&irt)
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))?,
                body,
                subject,
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map_err(|e| CoreError::InvalidInput(format!("bad updated_at '{}': {}", updated_at, e)))?
                    .with_timezone(&Utc),
            })),
        }
    }

    /// Idempotent — deleting a missing row is Ok.
    pub fn delete_reply_draft(&self, thread_id: &Uuid) -> Result<()> {
        self.conn().execute(
            "DELETE FROM reply_drafts WHERE thread_id = ?1",
            params![thread_id.to_string()],
        )?;
        Ok(())
    }
}
