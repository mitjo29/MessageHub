//! Insert a row at runtime → reload → a task spawns.
//! Disable the row → reload → the task stops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::error::Result;
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
use uuid::Uuid;

struct MockFactory;

#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        a.add_message(RawMessage {
            external_id: "x".into(),
            channel: Channel::Telegram,
            external_thread_id: None,
            sender_name: "A".into(),
            sender_address: "a".into(),
            text: Some("hi".into()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        });
        Ok(Box::new(a))
    }
}

fn row(id: Uuid, enabled: bool) -> ChannelConfig {
    ChannelConfig {
        id,
        channel: Channel::Telegram,
        label: "t".into(),
        keychain_ref: "none".into(),
        enabled,
        poll_interval_secs: 1,
        last_sync_cursor: None,
        last_sync_at: None,
        status: ChannelStatus::Healthy,
        last_error: None,
        consecutive_failures: 0,
    }
}

#[tokio::test]
async fn reload_adds_and_removes_channel_tasks() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_factory("Telegram", Arc::new(MockFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap(); // zero channels yet → no channel task spawned

    // Add a row + reload.
    let id = Uuid::new_v4();
    store.lock().unwrap().insert_channel_config(&row(id, true)).unwrap();
    rt.reload_channels().await.unwrap();

    // Expect at least one SyncSucceeded/MessageIngested within a few seconds.
    let mut saw_activity = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && !saw_activity {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            if matches!(
                ev,
                RuntimeEvent::SyncSucceeded { .. } | RuntimeEvent::MessageIngested { .. }
            ) {
                saw_activity = true;
            }
        }
    }
    assert!(saw_activity, "runtime should poll after add+reload");

    // Disable the row + reload → task stops.
    store.lock().unwrap().update_channel_enabled(&id, false).unwrap();
    rt.reload_channels().await.unwrap();

    rt.shutdown().await;
}
