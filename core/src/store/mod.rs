pub mod ai_log;
pub mod channels;
pub mod contacts;
pub mod knowledge;
pub mod messages;
mod migrations;

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;
use tracing::info;

use crate::error::Result;

/// Register the sqlite-vec extension with SQLite's auto-extension list exactly once.
///
/// The `sqlite-vec` crate statically links the extension's C code and exposes
/// `sqlite3_vec_init` as an `extern "C"` symbol. Rather than calling
/// `load_extension()` on every `Connection` (which is meant for dynamic `.so`
/// libraries and requires the `load_extension` rusqlite feature), we register
/// the init function with `sqlite3_auto_extension`. Every `Connection::open*`
/// call afterwards automatically invokes it, which is what we want.
///
/// Safe to call multiple times — `Once` ensures the FFI call fires exactly once.
fn ensure_sqlite_vec_loaded() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is a C function pointer with the correct
        // signature for `sqlite3_auto_extension`. The transmute strips the
        // explicit argument list from the public `extern "C"` declaration —
        // SQLite calls it with `(db, pzErrMsg, pApi)` under the hood.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

pub use ai_log::AiDecision;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) an encrypted database at the given path.
    pub fn open(path: &Path, password: &str) -> Result<Self> {
        // Must happen before any Connection::open* call so sqlite-vec is
        // auto-loaded into the new connection.
        ensure_sqlite_vec_loaded();

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
        ensure_sqlite_vec_loaded();

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
