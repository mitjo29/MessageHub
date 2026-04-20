//! TOML config loader. Reuses the schema from runtime-demo — the
//! `[ai]` and `[[channels]]` sections are ignored here.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct DesktopConfig {
    pub database: String,
    pub password: String,
}

/// Locate the config file. Checks `./messagehub.toml` first (if launched
/// from desktop/), then `../core/messagehub.toml` (if reusing
/// runtime-demo's config), then returns the first one that exists.
pub fn resolve_config_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("messagehub.toml"),
        PathBuf::from("../core/messagehub.toml"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn load_config(path: &Path) -> Result<DesktopConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
    toml::from_str::<DesktopConfig>(&text)
        .map_err(|e| format!("failed to parse '{}': {}", path.display(), e))
}
