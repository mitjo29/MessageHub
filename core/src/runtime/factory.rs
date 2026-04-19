use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::ChannelAdapter;
use crate::error::Result;
use crate::types::ChannelConfig;

/// Builds adapter instances from persisted channel rows.
///
/// The factory is responsible for credential resolution (keychain lookup,
/// OAuth refresh). The `Runtime` calls `build` once per channel at startup,
/// then calls `connect` on the returned adapter before starting the poll loop.
#[async_trait]
pub trait AdapterFactory: Send + Sync {
    async fn build(&self, config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>>;
}

/// Keyed registry of factories. Key is the DB `channel_type` string
/// (see `Channel::to_db_str`: "Email", "Telegram", etc.).
#[derive(Default, Clone)]
pub struct FactoryRegistry {
    inner: HashMap<String, Arc<dyn AdapterFactory>>,
}

impl FactoryRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, channel_type: impl Into<String>, factory: Arc<dyn AdapterFactory>) {
        self.inner.insert(channel_type.into(), factory);
    }

    pub fn get(&self, channel_type: &str) -> Option<Arc<dyn AdapterFactory>> {
        self.inner.get(channel_type).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{ChannelAdapter, RawMessage};
    use crate::error::Result;
    use crate::types::{Channel, MessageContent};
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};

    struct DummyAdapter;
    #[async_trait]
    impl ChannelAdapter for DummyAdapter {
        async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> { Ok(()) }
        async fn fetch_messages(&self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
            Ok(vec![])
        }
        async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> { Ok(()) }
        async fn disconnect(&mut self) -> Result<()> { Ok(()) }
        fn channel_type(&self) -> Channel { Channel::Telegram }
    }

    struct DummyFactory;
    #[async_trait]
    impl AdapterFactory for DummyFactory {
        async fn build(&self, _config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
            Ok(Box::new(DummyAdapter))
        }
    }

    #[test]
    fn registry_returns_registered_factory() {
        let mut reg = FactoryRegistry::new();
        reg.register("Telegram", Arc::new(DummyFactory));
        assert!(reg.get("Telegram").is_some());
        assert!(reg.get("Email").is_none());
    }
}
