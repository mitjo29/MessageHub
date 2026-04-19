//! An adapter fails N times then recovers. Assert status progression.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::{CoreError, Result};
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};
use uuid::Uuid;

/// Fails on fetch_messages until `fail_remaining` hits zero, then succeeds.
struct FlakyAdapter {
    fail_remaining: Arc<AtomicUsize>,
}

#[async_trait]
impl ChannelAdapter for FlakyAdapter {
    async fn connect(&mut self, _c: &ChannelConfig) -> Result<()> {
        Ok(())
    }
    async fn fetch_messages(&self, _s: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if self.fail_remaining.load(Ordering::SeqCst) > 0 {
            self.fail_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(CoreError::InvalidInput("boom".to_string()));
        }
        Ok(vec![])
    }
    async fn send_reply(&self, _t: &str, _c: &MessageContent) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    fn channel_type(&self) -> Channel {
        Channel::Telegram
    }
}

struct FlakyFactory {
    fail_remaining: Arc<AtomicUsize>,
}

#[async_trait]
impl AdapterFactory for FlakyFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(FlakyAdapter {
            fail_remaining: Arc::clone(&self.fail_remaining),
        }))
    }
}

fn cfg(id: Uuid) -> ChannelConfig {
    ChannelConfig {
        id,
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
    }
}

#[tokio::test]
async fn status_progresses_degraded_failed_then_healthy() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let id = Uuid::new_v4();
    store
        .lock()
        .unwrap()
        .insert_channel_config(&cfg(id))
        .unwrap();

    let fails = Arc::new(AtomicUsize::new(5)); // 5 consecutive failures (crosses threshold)
    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory(
            "Telegram",
            Arc::new(FlakyFactory {
                fail_remaining: Arc::clone(&fails),
            }),
        )
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let mut saw_degraded = false;
    let mut saw_failed = false;
    let mut saw_healthy_after = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline
        && !(saw_degraded && saw_failed && saw_healthy_after)
    {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(500), events.recv()).await
        {
            if let RuntimeEvent::ChannelStatusChanged { status, .. } = ev {
                match status {
                    ChannelStatus::Degraded { .. } => saw_degraded = true,
                    ChannelStatus::Failed { .. } => saw_failed = true,
                    ChannelStatus::Healthy => {
                        if saw_failed {
                            saw_healthy_after = true;
                        }
                    }
                }
            }
        }
    }
    assert!(saw_degraded, "expected Degraded");
    assert!(saw_failed, "expected Failed");
    assert!(saw_healthy_after, "expected return to Healthy after recovery");

    rt.shutdown().await;
}
