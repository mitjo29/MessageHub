//! Two channels. One fails every fetch; the other succeeds. The healthy one
//! must continue emitting SyncSucceeded events regardless.

use std::collections::HashMap;
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

fn make_raw_msg(channel: Channel) -> RawMessage {
    RawMessage {
        external_id: Uuid::new_v4().to_string(),
        channel,
        external_thread_id: None,
        sender_name: "Alice".into(),
        sender_address: "alice@example.com".into(),
        text: Some("hello".into()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

struct Ok0;

#[async_trait]
impl ChannelAdapter for Ok0 {
    async fn connect(&mut self, _: &ChannelConfig) -> Result<()> {
        Ok(())
    }
    async fn fetch_messages(&self, _: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Ok(vec![make_raw_msg(Channel::Telegram)])
    }
    async fn send_reply(&self, _: &str, _: &MessageContent) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    fn channel_type(&self) -> Channel {
        Channel::Telegram
    }
}

struct Err0;

#[async_trait]
impl ChannelAdapter for Err0 {
    async fn connect(&mut self, _: &ChannelConfig) -> Result<()> {
        Ok(())
    }
    async fn fetch_messages(&self, _: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        Err(CoreError::InvalidInput("always".into()))
    }
    async fn send_reply(&self, _: &str, _: &MessageContent) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    fn channel_type(&self) -> Channel {
        Channel::Email
    }
}

struct OkFactory;

#[async_trait]
impl AdapterFactory for OkFactory {
    async fn build(&self, _: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(Ok0))
    }
}

struct ErrFactory;

#[async_trait]
impl AdapterFactory for ErrFactory {
    async fn build(&self, _: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        Ok(Box::new(Err0))
    }
}

fn row(channel: Channel) -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel,
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
async fn healthy_channel_keeps_polling_while_other_fails() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let ok_row = row(Channel::Telegram);
    let err_row = row(Channel::Email);
    store
        .lock()
        .unwrap()
        .insert_channel_config(&ok_row)
        .unwrap();
    store
        .lock()
        .unwrap()
        .insert_channel_config(&err_row)
        .unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(OkFactory))
        .with_factory("Email", Arc::new(ErrFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let mut ok_successes = 0u32;
    let mut err_failures = 0u32;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            match ev {
                RuntimeEvent::SyncSucceeded { channel_id, .. } if channel_id == ok_row.id => {
                    ok_successes += 1;
                }
                RuntimeEvent::SyncFailed { channel_id, .. } if channel_id == err_row.id => {
                    err_failures += 1;
                }
                _ => {}
            }
        }
    }

    assert!(ok_successes >= 2, "healthy channel produced {} successes", ok_successes);
    assert!(err_failures >= 1, "failing channel produced no failures");

    rt.shutdown().await;
}
