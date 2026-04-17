use rusqlite::params;

use crate::error::{CoreError, Result};
use crate::store::Store;

/// A row from `action_log`. Populated by `log_ai_decision`.
#[derive(Debug, Clone)]
pub struct AiDecision {
    pub id: i64,
    pub action_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub reasoning: Option<String>,
    pub confidence_score: Option<f64>,
    pub created_at: String,
}

impl Store {
    /// Append an AI decision row to `action_log`.
    ///
    /// Used by the classifier and pipeline to record every enrichment
    /// decision with a reasoning string and confidence score, which the
    /// UI surfaces as "Why is this prioritized?"
    pub fn log_ai_decision(
        &self,
        action_type: &str,
        entity_type: &str,
        entity_id: &str,
        reasoning: &str,
        confidence_score: f64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO action_log (action_type, entity_type, entity_id, reasoning, confidence_score)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![action_type, entity_type, entity_id, reasoning, confidence_score],
        )?;
        Ok(())
    }

    /// Return every decision about a given entity, oldest first.
    ///
    /// The ORDER BY `id` asc is deterministic even when multiple rows
    /// share the same `created_at` (sub-second inserts); the `id` column
    /// is autoincrement so insertion order is preserved.
    pub fn list_ai_decisions_for_entity(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Vec<AiDecision>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, action_type, entity_type, entity_id, reasoning, confidence_score, created_at
             FROM action_log
             WHERE entity_type = ?1 AND entity_id = ?2
             ORDER BY id ASC",
        )?;
        let rows: std::result::Result<Vec<AiDecision>, rusqlite::Error> = stmt
            .query_map(params![entity_type, entity_id], |row| {
                Ok(AiDecision {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    entity_type: row.get(2)?,
                    entity_id: row.get(3)?,
                    reasoning: row.get(4)?,
                    confidence_score: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect();
        rows.map_err(CoreError::Database)
    }
}
