//! runtime-demo — smoke-test harness for the Plan 6 Runtime.
//!
//! Reads `messagehub.toml`, opens a SQLCipher store, runs the Runtime
//! against real email + Telegram accounts, prints events to stdout,
//! exits cleanly on Ctrl-C. See
//! `docs/superpowers/specs/2026-04-19-plan7a-runtime-demo-design.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    init_tracing();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("runtime-demo: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_target(false).init();
}

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    database: String,
    password: String,
    #[serde(default)]
    ai: Option<AiConfig>,
    #[serde(default)]
    channels: Vec<ChannelEntry>,
}

#[derive(Debug, Deserialize)]
struct AiConfig {
    enabled: bool,
    ollama_url: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChannelEntry {
    kind: String,            // "email" | "telegram"
    label: String,
    poll_interval_secs: u32,
    enabled: bool,
    credentials: toml::Value, // kind-specific; deserialized lazily below
}

#[derive(Debug, Deserialize)]
struct EmailCredentials {
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
    username: String,
    password: String,
    #[serde(default = "default_mailbox")]
    mailbox: String,
}
fn default_mailbox() -> String { "INBOX".to_string() }

#[derive(Debug, Deserialize)]
struct TelegramCredentials {
    bot_token: String,
}

use std::path::PathBuf;

fn default_config_path() -> PathBuf { PathBuf::from("messagehub.toml") }

fn load_config(path: &std::path::Path) -> Result<Config, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config '{}': {}", path.display(), e))?;
    let config: Config = toml::from_str(&text)
        .map_err(|e| format!("failed to parse '{}': {}", path.display(), e))?;
    Ok(config)
}

fn parse_config_path(args: &[String]) -> PathBuf {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            if let Some(path) = iter.next() { return PathBuf::from(path); }
        }
    }
    default_config_path()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = parse_config_path(&args);
    let config = load_config(&config_path)?;
    eprintln!(
        "runtime-demo: loaded {} channel(s) from {}",
        config.channels.len(),
        config_path.display(),
    );
    Ok(())
}
