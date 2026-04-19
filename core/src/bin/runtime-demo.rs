//! runtime-demo — smoke-test harness for the Plan 6 Runtime.
//!
//! Reads `messagehub.toml`, opens a SQLCipher store, runs the Runtime
//! against real email + Telegram accounts, prints events to stdout,
//! exits cleanly on Ctrl-C. See
//! `docs/superpowers/specs/2026-04-19-plan7a-runtime-demo-design.md`.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use async_trait::async_trait;
use messagehub_core::adapters::{
    email::{EmailAdapter, ImapSettings},
    telegram::TelegramAdapter,
    ChannelAdapter,
};
use messagehub_core::error::Result as CoreResult;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};

fn main() -> ExitCode {
    init_tracing();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime-demo: failed to start tokio runtime: {}", e);
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("runtime-demo: {}", e); ExitCode::FAILURE }
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

fn stable_channel_id(kind: &str, label: &str) -> Uuid {
    // UUID v5 with NAMESPACE_OID gives a deterministic id from (kind, label).
    // Re-runs of runtime-demo always map the same TOML entry to the same DB row.
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:{}", kind, label).as_bytes())
}

fn channel_kind_from_toml(kind: &str) -> Result<Channel, Box<dyn std::error::Error>> {
    match kind {
        "email"    => Ok(Channel::Email),
        "telegram" => Ok(Channel::Telegram),
        other => Err(format!("unsupported channel kind '{}'", other).into()),
    }
}

fn credential_keychain_ref(entry: &ChannelEntry) -> Result<String, Box<dyn std::error::Error>> {
    match entry.kind.as_str() {
        "email" => {
            let c: EmailCredentials = entry.credentials.clone().try_into()
                .map_err(|e| format!("email channel '{}': bad credentials — {}", entry.label, e))?;
            // EmailAdapter::connect() reads config.keychain_ref as "user:password".
            Ok(format!("{}:{}", c.username, c.password))
        }
        "telegram" => {
            let c: TelegramCredentials = entry.credentials.clone().try_into()
                .map_err(|e| format!("telegram channel '{}': bad credentials — {}", entry.label, e))?;
            // TelegramAdapter::connect() reads config.keychain_ref as the raw bot token.
            Ok(c.bot_token)
        }
        other => Err(format!("unsupported channel kind '{}'", other).into()),
    }
}

fn reconcile_channels(
    store: &Mutex<Store>,
    entries: &[ChannelEntry],
) -> Result<HashMap<Uuid, String>, Box<dyn std::error::Error>> {
    let guard = store.lock().expect("runtime-demo: store mutex poisoned");
    let existing: Vec<ChannelConfig> = guard.list_channel_configs()?;
    let existing_ids: std::collections::HashSet<Uuid> =
        existing.iter().map(|c| c.id).collect();

    let mut labels = HashMap::new();
    for entry in entries {
        let id = stable_channel_id(&entry.kind, &entry.label);
        labels.insert(id, entry.label.clone());
        if existing_ids.contains(&id) { continue; }

        let channel = channel_kind_from_toml(&entry.kind)?;
        let keychain_ref = credential_keychain_ref(entry)?;
        let cfg = ChannelConfig {
            id,
            channel,
            label: entry.label.clone(),
            keychain_ref,
            enabled: entry.enabled,
            poll_interval_secs: entry.poll_interval_secs,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        };
        guard.insert_channel_config(&cfg)?;
    }
    Ok(labels)
}

// ---------------------------------------------------------------------------
// Factory implementations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EmailConnection {
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
}

struct EmailFactoryImpl { creds: HashMap<Uuid, EmailConnection> }

#[async_trait]
impl AdapterFactory for EmailFactoryImpl {
    async fn build(&self, row: &ChannelConfig) -> CoreResult<Box<dyn ChannelAdapter>> {
        let c = self.creds.get(&row.id).ok_or_else(|| {
            messagehub_core::error::CoreError::InvalidInput(format!(
                "runtime-demo: no email credentials for channel '{}'", row.label,
            ))
        })?;
        let adapter = EmailAdapter::with_settings(ImapSettings {
            host: c.imap_host.clone(),
            port: c.imap_port,
            smtp_host: c.smtp_host.clone(),
            smtp_port: c.smtp_port,
        });
        Ok(Box::new(adapter))
    }
}

struct TelegramFactoryImpl;

#[async_trait]
impl AdapterFactory for TelegramFactoryImpl {
    async fn build(&self, _row: &ChannelConfig) -> CoreResult<Box<dyn ChannelAdapter>> {
        Ok(Box::new(TelegramAdapter::new()))
    }
}

fn build_factories(
    entries: &[ChannelEntry],
) -> Result<(Arc<EmailFactoryImpl>, Arc<TelegramFactoryImpl>), Box<dyn std::error::Error>> {
    let mut email_creds = HashMap::new();
    for entry in entries.iter().filter(|e| e.kind == "email") {
        let id = stable_channel_id(&entry.kind, &entry.label);
        let c: EmailCredentials = entry.credentials.clone().try_into()
            .map_err(|e| format!("email channel '{}': {}", entry.label, e))?;
        email_creds.insert(id, EmailConnection {
            imap_host: c.imap_host,
            imap_port: c.imap_port,
            smtp_host: c.smtp_host,
            smtp_port: c.smtp_port,
        });
    }
    Ok((
        Arc::new(EmailFactoryImpl { creds: email_creds }),
        Arc::new(TelegramFactoryImpl),
    ))
}

// ---------------------------------------------------------------------------
// Event printer
// ---------------------------------------------------------------------------

use chrono::Local;
use messagehub_core::runtime::events::RuntimeEvent;
use tokio::sync::broadcast::Receiver;

async fn print_events(mut rx: Receiver<RuntimeEvent>, labels: HashMap<Uuid, String>) {
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let now = Local::now().format("%H:%M:%S%.3f");
                let label_of = |id: &Uuid| labels.get(id).cloned()
                    .unwrap_or_else(|| id.to_string());
                match ev {
                    RuntimeEvent::MessageIngested { id, channel_id } => {
                        println!("[{} ingested]  msg={} channel=\"{}\"",
                                 now, short(&id), label_of(&channel_id));
                    }
                    RuntimeEvent::MessageClassified { id, category, priority } => {
                        println!("[{} classified] msg={} category={} priority={}",
                                 now,
                                 short(&id),
                                 category.unwrap_or_else(|| "?".into()),
                                 priority.map(|p| p.value().to_string())
                                         .unwrap_or_else(|| "?".into()));
                    }
                    RuntimeEvent::SyncSucceeded { channel_id, count } => {
                        println!("[{} sync ok]   channel=\"{}\" count={}",
                                 now, label_of(&channel_id), count);
                    }
                    RuntimeEvent::SyncFailed { channel_id, error, attempt } => {
                        println!("[{} sync fail] channel=\"{}\" attempt={} error=\"{}\"",
                                 now, label_of(&channel_id), attempt, error);
                    }
                    RuntimeEvent::ChannelStatusChanged { channel_id, status } => {
                        println!("[{} status]    channel=\"{}\" {:?}",
                                 now, label_of(&channel_id), status);
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("runtime-demo: event stream lagged, dropped {} events", n);
            }
        }
    }
}

fn short(id: &Uuid) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Main async entry point
// ---------------------------------------------------------------------------

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    use messagehub_core::ai::{llm::OllamaLlm, pipeline::AiPipeline, profile::UserProfile};
    use messagehub_core::runtime::Runtime;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = parse_config_path(&args);
    let config = load_config(&config_path)?;

    let store = Arc::new(Mutex::new(Store::open(
        std::path::Path::new(&config.database),
        &config.password,
    )?));

    // reconcile_channels is sync — lock is held only within the call, released before any await.
    let labels = reconcile_channels(&store, &config.channels)?;
    let (email_factory, telegram_factory) = build_factories(&config.channels)?;

    let mut builder = Runtime::builder(Arc::clone(&store))
        .with_factory("Email", email_factory)
        .with_factory("Telegram", telegram_factory);

    if let Some(ai) = &config.ai {
        if ai.enabled {
            let url   = ai.ollama_url.clone().unwrap_or_else(|| "http://localhost:11434".into());
            let model = ai.model     .clone().unwrap_or_else(|| "llama3.2".into());
            let llm: Arc<dyn messagehub_core::ai::llm::LlmBackend> =
                Arc::new(OllamaLlm::new(url.clone(), model.clone()));
            let pipeline = AiPipeline::new(
                llm,
                None, // no retriever in Plan 7a — see spec §Non-Goals
                UserProfile { content: String::new() },
            );
            builder = builder.with_ai_pipeline(Arc::new(pipeline));
            eprintln!("runtime-demo: AI tier enabled (ollama at {}, model {})", url, model);
        } else {
            eprintln!("runtime-demo: AI tier disabled (ingest-only)");
        }
    } else {
        eprintln!("runtime-demo: no [ai] section in config — ingest-only");
    }

    let mut rt = builder.build();
    let events = rt.subscribe();
    rt.start().await?;
    eprintln!("runtime-demo: runtime started. Ctrl-C to stop.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nruntime-demo: Ctrl-C received, shutting down...");
        }
        _ = print_events(events, labels) => {
            // print_events only returns when the broadcast is closed, which
            // happens during shutdown. Nothing to do here.
        }
    }

    rt.shutdown().await;
    eprintln!("runtime-demo: shutdown complete");
    Ok(())
}
