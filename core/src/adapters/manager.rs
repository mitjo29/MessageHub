use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn, error};
use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::types::ChannelConfig;
use super::{ChannelAdapter, RawMessage};

/// Coordinates multiple channel adapters, running sync loops on background tasks.
pub struct AdapterManager {
    adapters: HashMap<Uuid, Arc<Mutex<Box<dyn ChannelAdapter>>>>,
    configs: HashMap<Uuid, ChannelConfig>,
    sync_handles: HashMap<Uuid, JoinHandle<()>>,
    /// Callback invoked for each batch of fetched raw messages.
    on_messages: Arc<dyn Fn(Vec<RawMessage>) + Send + Sync>,
}

impl AdapterManager {
    /// Create a new manager with a callback that receives fetched messages.
    pub fn new<F>(on_messages: F) -> Self
    where
        F: Fn(Vec<RawMessage>) + Send + Sync + 'static,
    {
        Self {
            adapters: HashMap::new(),
            configs: HashMap::new(),
            sync_handles: HashMap::new(),
            on_messages: Arc::new(on_messages),
        }
    }

    /// Register an adapter for a specific channel config.
    /// Connects the adapter and returns its config ID.
    pub async fn register(
        &mut self,
        mut adapter: Box<dyn ChannelAdapter>,
        config: ChannelConfig,
    ) -> Result<Uuid> {
        let config_id = config.id;

        adapter.connect(&config).await?;
        info!(
            channel = %config.channel,
            label = %config.label,
            "adapter connected"
        );

        self.adapters.insert(config_id, Arc::new(Mutex::new(adapter)));
        self.configs.insert(config_id, config);

        Ok(config_id)
    }

    /// Start the background sync loop for a registered adapter.
    pub fn start_sync(&mut self, config_id: Uuid) -> Result<()> {
        let config = self.configs.get(&config_id).ok_or_else(|| {
            CoreError::NotFound {
                entity: "ChannelConfig".to_string(),
                id: config_id.to_string(),
            }
        })?;

        if !config.enabled {
            warn!(config_id = %config_id, "adapter disabled, skipping sync");
            return Ok(());
        }

        let adapter = Arc::clone(
            self.adapters.get(&config_id).ok_or_else(|| {
                CoreError::NotFound {
                    entity: "Adapter".to_string(),
                    id: config_id.to_string(),
                }
            })?,
        );

        let poll_interval = std::time::Duration::from_secs(config.poll_interval_secs as u64);
        let last_sync = config.last_sync_at;
        let on_messages = Arc::clone(&self.on_messages);
        let channel = config.channel;

        let handle = tokio::spawn(async move {
            let mut since = last_sync;
            loop {
                {
                    let adapter = adapter.lock().await;
                    match adapter.fetch_messages(since).await {
                        Ok(messages) if !messages.is_empty() => {
                            info!(
                                channel = %channel,
                                count = messages.len(),
                                "fetched messages"
                            );
                            // Update cursor to latest message timestamp
                            if let Some(latest) = messages.iter().map(|m| m.timestamp).max() {
                                since = Some(latest);
                            }
                            (on_messages)(messages);
                        }
                        Ok(_) => {
                            // No new messages, nothing to do
                        }
                        Err(e) => {
                            error!(
                                channel = %channel,
                                error = %e,
                                "fetch failed"
                            );
                        }
                    }
                }
                tokio::time::sleep(poll_interval).await;
            }
        });

        self.sync_handles.insert(config_id, handle);
        Ok(())
    }

    /// Stop the sync loop for a specific adapter.
    pub fn stop_sync(&mut self, config_id: &Uuid) {
        if let Some(handle) = self.sync_handles.remove(config_id) {
            handle.abort();
            info!(config_id = %config_id, "sync stopped");
        }
    }

    /// Disconnect and remove an adapter.
    pub async fn unregister(&mut self, config_id: &Uuid) -> Result<()> {
        self.stop_sync(config_id);

        if let Some(adapter) = self.adapters.remove(config_id) {
            let mut adapter = adapter.lock().await;
            adapter.disconnect().await?;
        }

        self.configs.remove(config_id);
        info!(config_id = %config_id, "adapter unregistered");
        Ok(())
    }

    /// Stop all sync loops and disconnect all adapters.
    pub async fn shutdown(&mut self) -> Result<()> {
        let config_ids: Vec<Uuid> = self.adapters.keys().cloned().collect();
        for config_id in config_ids {
            self.unregister(&config_id).await?;
        }
        info!("all adapters shut down");
        Ok(())
    }

    /// Get a list of all registered config IDs.
    pub fn registered_configs(&self) -> Vec<Uuid> {
        self.configs.keys().cloned().collect()
    }

    /// Check if a specific adapter is registered and has an active sync loop.
    pub fn is_syncing(&self, config_id: &Uuid) -> bool {
        self.sync_handles
            .get(config_id)
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// Get a reference to an adapter for sending replies.
    pub fn get_adapter(&self, config_id: &Uuid) -> Option<Arc<Mutex<Box<dyn ChannelAdapter>>>> {
        self.adapters.get(config_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Channel;

    fn test_config() -> ChannelConfig {
        ChannelConfig {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "Test Telegram".to_string(),
            keychain_ref: "test-key".to_string(),
            enabled: true,
            poll_interval_secs: 1,
            last_sync_cursor: None,
            last_sync_at: None,
        }
    }

    #[tokio::test]
    async fn test_manager_register_and_unregister() {
        use crate::adapters::mock::MockAdapter;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let mut manager = AdapterManager::new(move |msgs| {
            counter_clone.fetch_add(msgs.len(), Ordering::Relaxed);
        });

        let adapter = Box::new(MockAdapter::new());
        let config = test_config();
        let config_id = config.id;

        let result = manager.register(adapter, config).await;
        assert!(result.is_ok());
        assert_eq!(manager.registered_configs().len(), 1);

        let result = manager.unregister(&config_id).await;
        assert!(result.is_ok());
        assert_eq!(manager.registered_configs().len(), 0);
    }

    #[tokio::test]
    async fn test_manager_start_and_stop_sync() {
        use crate::adapters::mock::MockAdapter;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let mut manager = AdapterManager::new(move |msgs| {
            counter_clone.fetch_add(msgs.len(), Ordering::Relaxed);
        });

        let adapter = MockAdapter::new();
        adapter.add_message(RawMessage {
            external_id: "msg-1".to_string(),
            channel: Channel::Telegram,
            external_thread_id: None,
            sender_name: "Bot".to_string(),
            sender_address: "bot123".to_string(),
            text: Some("Hello".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });

        let config = test_config();
        let config_id = config.id;

        manager.register(Box::new(adapter), config).await.unwrap();
        manager.start_sync(config_id).unwrap();

        assert!(manager.is_syncing(&config_id));

        // Let one poll cycle complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(counter.load(Ordering::Relaxed) >= 1);

        manager.stop_sync(&config_id);
        // The handle is aborted, is_syncing may still return true briefly
    }

    #[tokio::test]
    async fn test_manager_disabled_adapter_skips_sync() {
        use crate::adapters::mock::MockAdapter;

        let mut manager = AdapterManager::new(|_| {});

        let adapter = Box::new(MockAdapter::new());
        let mut config = test_config();
        config.enabled = false;

        let config_id = manager.register(adapter, config).await.unwrap();
        let result = manager.start_sync(config_id);
        assert!(result.is_ok());
        assert!(!manager.is_syncing(&config_id));
    }

    #[tokio::test]
    async fn test_manager_shutdown() {
        use crate::adapters::mock::MockAdapter;

        let mut manager = AdapterManager::new(|_| {});

        let config1 = test_config();
        let config2 = test_config();

        manager
            .register(Box::new(MockAdapter::new()), config1)
            .await
            .unwrap();
        manager
            .register(Box::new(MockAdapter::new()), config2)
            .await
            .unwrap();

        assert_eq!(manager.registered_configs().len(), 2);

        manager.shutdown().await.unwrap();
        assert_eq!(manager.registered_configs().len(), 0);
    }
}
