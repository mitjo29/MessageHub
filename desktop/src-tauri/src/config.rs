//! TOML config loader. Reuses the schema from runtime-demo — the
//! `[cloud]` and `[[channels]]` sections are now parsed here.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct DesktopConfig {
    pub database: String,
    pub password: String,
    /// Optional path to a markdown file containing the user's self-authored
    /// profile (languages, role, relationships, tone). Injected into cloud
    /// draft prompts via `UserProfile`. Resolved against the TOML's parent
    /// directory when relative. Missing file → empty profile (graceful).
    #[serde(default)]
    pub profile_path: Option<String>,
    #[serde(default)]
    pub cloud: Option<TauriCloudConfig>,
    #[serde(default)]
    pub channels: Vec<ChannelEntry>,
}

/// Cloud / AI config block (`[cloud]` in messagehub.toml).
/// Named `TauriCloudConfig` to avoid collision with
/// `messagehub_core::ai::cloud::CloudConfig`.
#[derive(Debug, Deserialize)]
pub struct TauriCloudConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub model: Option<String>,
    /// Optional base URL override. Defaults to `https://api.anthropic.com`
    /// when absent. Point this at a LiteLLM/Anthropic-compatible proxy to
    /// route drafts through an alternate backend (e.g. Ollama via proxy).
    /// The wire format stays Anthropic's `/v1/messages` — the proxy is
    /// responsible for translation if the underlying model speaks OpenAI.
    pub url: Option<String>,
}

/// One entry in the `[[channels]]` array.
#[derive(Debug, Deserialize)]
pub struct ChannelEntry {
    pub kind: String,
    pub label: String,
    pub enabled: bool,
    pub credentials: toml::Value,
}

/// Credential shape for `kind = "email"` channels.
/// `Clone` is required so it can be stored in a HashMap (Task 9).
#[derive(Debug, Deserialize, Clone)]
pub struct EmailCredentials {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
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
