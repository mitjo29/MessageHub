use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use messagehub_core::store::Store;
use messagehub_core::types::Channel;
use uuid::Uuid;

/// Shared state registered with Tauri via `Builder::manage`.
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub label_by_channel_id: HashMap<Uuid, String>,
    /// Maps a Channel variant to all configured labels for that channel type.
    /// Used by `build_message_row` in commands.rs to pick a display label.
    pub channel_labels_by_variant: HashMap<Channel, Vec<String>>,
    pub db_path: String,
}

impl AppState {
    pub fn init(db_path: &str, password: &str) -> Result<Self, String> {
        let store = Store::open(std::path::Path::new(db_path), password)
            .map_err(|e| format!("failed to open store: {}", e))?;
        let channel_configs = store
            .list_channel_configs()
            .map_err(|e| format!("failed to list channels: {}", e))?;
        let label_by_channel_id = channel_configs
            .iter()
            .map(|c| (c.id, c.label.clone()))
            .collect::<HashMap<_, _>>();

        // Build variant → labels map for display in message rows.
        let mut channel_labels_by_variant: HashMap<Channel, Vec<String>> = HashMap::new();
        for c in &channel_configs {
            channel_labels_by_variant
                .entry(c.channel)
                .or_default()
                .push(c.label.clone());
        }

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            label_by_channel_id,
            channel_labels_by_variant,
            db_path: db_path.to_string(),
        })
    }
}
