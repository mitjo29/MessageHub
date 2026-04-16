pub mod channels;
pub mod contacts;
pub mod messages;
mod migrations;

use std::path::Path;
use rusqlite::Connection;
use tracing::info;

use crate::error::Result;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) an encrypted database at the given path.
    pub fn open(path: &Path, password: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", password)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", "5000")?;

        migrations::run_migrations(&conn)?;

        info!(path = %path.display(), "database opened");
        Ok(Self { conn })
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::run_migrations(&conn)?;

        Ok(Self { conn })
    }

    /// Access the raw connection (for advanced queries).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
