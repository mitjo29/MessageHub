//! TOML config loader. Reuses the schema from runtime-demo — the
//! `[ai]` and `[[channels]]` sections are ignored here.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct DesktopConfig {
    pub database: String,
    pub password: String,
}

/// Locate the config file. Tauri `dev` spawns the Rust binary with CWD =
/// `desktop/src-tauri/`; `cargo run --bin` from workspace root uses the
/// workspace root; `tauri build` may differ again. Check all plausible
/// relative paths and pick the first that exists.
pub fn resolve_config_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("messagehub.toml"),          // CWD = desktop/ or workspace root
        PathBuf::from("../core/messagehub.toml"),  // CWD = desktop/
        PathBuf::from("../../core/messagehub.toml"), // CWD = desktop/src-tauri/ (tauri dev)
        PathBuf::from("core/messagehub.toml"),     // CWD = workspace root
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn load_config(path: &Path) -> Result<DesktopConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
    toml::from_str::<DesktopConfig>(&text)
        .map_err(|e| format!("failed to parse '{}': {}", path.display(), e))
}
