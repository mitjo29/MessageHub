use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use messagehub_core::ai::cloud::{AnthropicCloud, CloudActions, CloudProvider, Redactor};
use messagehub_core::ai::profile::UserProfile;
use messagehub_core::knowledge::Retriever;
use messagehub_core::store::Store;
use messagehub_core::types::Channel;
use uuid::Uuid;

/// Shared state registered with Tauri via `Builder::manage`.
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    /// Maps a Channel variant to all configured labels for that channel type.
    /// Used by `build_message_row` in commands.rs to pick a display label.
    /// TODO (7b.2): disambiguate per-message by receiving address when multiple
    /// accounts share a channel variant.
    pub channel_labels_by_variant: HashMap<Channel, Vec<String>>,
    pub db_path: String,
    /// Configured email-channel credentials keyed by the UUIDv5 id the
    /// runtime's `channels` table uses. Populated from `[[channels]]` at
    /// init time; the `send_email_reply` command (Task 11) looks up the
    /// entry for the relevant channel when building its SMTP transport.
    pub email_connections: HashMap<Uuid, EmailConnection>,
    /// Optional cloud actions handle. `None` if `[cloud]` is absent,
    /// disabled, or missing `api_key`/`model`.
    pub cloud: Option<Arc<CloudActions>>,
    /// Model name reported to the UI via `cloud_config_status`. Stored
    /// alongside `cloud` because `CloudActions` doesn't expose a getter.
    pub cloud_model: Option<String>,
}

/// Connection parameters for one configured email channel. Mirrors what
/// runtime-demo parses from `[[channels]]` with `kind = "email"`.
#[derive(Debug, Clone)]
pub struct EmailConnection {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
}

/// Matches runtime-demo's UUIDv5(OID, "{kind}:{label}") mapping so Reply
/// lines up with the runtime's `channels` rows.
pub fn stable_channel_id(kind: &str, label: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:{}", kind, label).as_bytes(),
    )
}

impl AppState {
    pub fn init(
        db_path: &str,
        password: &str,
        email_connections: HashMap<Uuid, EmailConnection>,
        cloud_cfg: Option<&crate::config::TauriCloudConfig>,
        profile: UserProfile,
    ) -> Result<Self, String> {
        let store = Store::open(std::path::Path::new(db_path), password)
            .map_err(|e| format!("failed to open store: {}", e))?;

        let channel_configs = store
            .list_channel_configs()
            .map_err(|e| format!("failed to list channels: {}", e))?;

        let mut channel_labels_by_variant: HashMap<Channel, Vec<String>> = HashMap::new();
        for c in &channel_configs {
            channel_labels_by_variant
                .entry(c.channel)
                .or_default()
                .push(c.label.clone());
        }

        // Build cloud while the store is in scope — Redactor::build(&store)
        // needs a live store to populate the vault-person regex cache.
        let (cloud, cloud_model) = match cloud_cfg.filter(|c| c.enabled) {
            Some(c) => match (c.api_key.as_ref(), c.model.as_ref()) {
                (Some(api_key), Some(model)) => {
                    let redactor = Redactor::build(&store)
                        .map_err(|e| format!("Redactor::build: {}", e))?;
                    let anthropic = AnthropicCloud::new(api_key.clone(), model.clone());
                    // Optional URL override — route drafts through a
                    // LiteLLM or Anthropic-compatible proxy instead of
                    // the default Anthropic endpoint.
                    let anthropic = match c.url.as_deref() {
                        Some(u) if !u.is_empty() => {
                            eprintln!("messagehub-desktop: cloud base URL overridden to {}", u);
                            anthropic.with_base_url(u.to_string())
                        }
                        _ => anthropic,
                    };
                    let provider: Arc<dyn CloudProvider> = Arc::new(anthropic);
                    let actions = CloudActions::new(
                        provider,
                        redactor,
                        None::<Arc<Retriever>>,
                        profile,
                        model.clone(),
                    );
                    (Some(Arc::new(actions)), Some(model.clone()))
                }
                _ => {
                    eprintln!(
                        "messagehub-desktop: [cloud] enabled but api_key / model missing — running without cloud",
                    );
                    (None, None)
                }
            },
            None => (None, None),
        };

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            channel_labels_by_variant,
            db_path: db_path.to_string(),
            email_connections,
            cloud,
            cloud_model,
        })
    }
}
