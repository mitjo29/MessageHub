use rusqlite::params;
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::store::Store;

/// Input payload for `Store::insert_draft`.
///
/// We use a borrowed struct rather than positional arguments because
/// the field count is high and positional calls at the call site are
/// error-prone ("which string was `input_redacted` again?").
#[derive(Debug, Clone)]
pub struct NewDraft<'a> {
    pub id: Uuid,
    /// `None` for `smart_search` (no anchor message); `Some(...)` for
    /// `summarize_thread` and `draft_reply`.
    pub message_id: Option<Uuid>,
    pub action_type: &'a str,
    pub input_redacted: &'a str,
    pub output: &'a str,
    pub confidence: f32,
    pub provider: &'a str,
    pub model: &'a str,
}

/// A row from `ai_drafts`. Returned by `list_drafts_for_message`.
#[derive(Debug, Clone)]
pub struct DraftRecord {
    pub id: Uuid,
    pub message_id: Option<Uuid>,
    pub action_type: String,
    pub input_redacted: String,
    pub output: String,
    pub user_edited_output: Option<String>,
    pub confidence: f32,
    pub provider: String,
    pub model: String,
    pub created_at: String,
}

impl Store {
    /// Persist a newly generated cloud draft.
    pub fn insert_draft(&self, draft: &NewDraft<'_>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO ai_drafts
                (id, message_id, action_type, input_redacted, output,
                 confidence, provider, model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                draft.id.to_string(),
                draft.message_id.map(|u| u.to_string()),
                draft.action_type,
                draft.input_redacted,
                draft.output,
                draft.confidence as f64,
                draft.provider,
                draft.model,
            ],
        )?;
        Ok(())
    }

    /// Return every draft anchored to `message_id`, newest first.
    ///
    /// `smart_search` drafts are persisted with `message_id = NULL` and
    /// therefore never appear here — call sites that need them should
    /// query by action type instead (future helper).
    pub fn list_drafts_for_message(&self, message_id: &Uuid) -> Result<Vec<DraftRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, message_id, action_type, input_redacted, output,
                    user_edited_output, confidence, provider, model, created_at
             FROM ai_drafts
             WHERE message_id = ?1
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows: Vec<DraftRecord> = stmt
            .query_map(params![message_id.to_string()], |row| {
                Ok(row_to_draft(row))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
            .into_iter()
            .collect::<std::result::Result<Vec<_>, CoreError>>()?;
        Ok(rows)
    }

    /// Replace `user_edited_output` on an existing draft. Does not touch
    /// the original `output` column — that stays as the cloud's
    /// verbatim response for audit.
    pub fn update_draft_output(&self, draft_id: &Uuid, edited: &str) -> Result<()> {
        let rows = self.conn().execute(
            "UPDATE ai_drafts SET user_edited_output = ?1 WHERE id = ?2",
            params![edited, draft_id.to_string()],
        )?;
        if rows == 0 {
            return Err(CoreError::NotFound {
                entity: "ai_draft".into(),
                id: draft_id.to_string(),
            });
        }
        Ok(())
    }
}

fn row_to_draft(row: &rusqlite::Row) -> std::result::Result<DraftRecord, CoreError> {
    let id_str: String = row.get(0)?;
    let msg_id_str: Option<String> = row.get(1)?;
    Ok(DraftRecord {
        id: Uuid::parse_str(&id_str).map_err(|e| CoreError::InvalidInput(e.to_string()))?,
        message_id: msg_id_str
            .as_deref()
            .map(|s| Uuid::parse_str(s).map_err(|e| CoreError::InvalidInput(e.to_string())))
            .transpose()?,
        action_type: row.get(2)?,
        input_redacted: row.get(3)?,
        output: row.get(4)?,
        user_edited_output: row.get(5)?,
        confidence: row.get::<_, f64>(6)? as f32,
        provider: row.get(7)?,
        model: row.get(8)?,
        created_at: row.get(9)?,
    })
}
