use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use messagehub_core::store::Store;
use uuid::Uuid;

/// Shared state registered with Tauri via `Builder::manage`.
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub label_by_channel_id: HashMap<Uuid, String>,
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
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            label_by_channel_id,
            db_path: db_path.to_string(),
        })
    }
}
