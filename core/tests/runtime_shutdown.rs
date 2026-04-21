//! Start the runtime, then shut it down. Assert disconnect was called.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::Result;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};
use uuid::Uuid;

struct Tracked {
    disconnected: Arc<AtomicBool>,
}

#[async_trait]
impl ChannelAdapter for Tracked {
    async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> {
        Ok(())
    }
    async fn fetch_messages(&mut self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Ok(vec![])
    }
    async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.disconnected.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn channel_type(&self) -> Channel {
        Channel::Telegram
    }
}

struct TrackedFactory {
    disconnected: Arc<AtomicBool>,
}

#[async_trait]
impl AdapterFactory for TrackedFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(Tracked {
            disconnected: Arc::clone(&self.disconnected),
        }))
    }
}

#[tokio::test]
async fn shutdown_disconnects_adapters() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let flag = Arc::new(AtomicBool::new(false));

    store
        .lock()
        .unwrap()
        .insert_channel_config(&ChannelConfig {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "t".into(),
            keychain_ref: "none".into(),
            enabled: true,
            poll_interval_secs: 1,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        })
        .unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory(
            "Telegram",
            Arc::new(TrackedFactory {
                disconnected: Arc::clone(&flag),
            }),
        )
        .build();
    rt.start().await.unwrap();

    // Let it run briefly.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    rt.shutdown().await;

    assert!(
        flag.load(Ordering::SeqCst),
        "disconnect() must be called on shutdown"
    );
}
