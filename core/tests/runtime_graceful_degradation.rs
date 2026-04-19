//! No AiPipeline configured → MessageIngested fires; MessageClassified never does.

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

struct MockFactory {
    seed: Vec<RawMessage>,
}

#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _c: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        for m in &self.seed {
            a.add_message(m.clone());
        }
        Ok(Box::new(a))
    }
}

fn raw() -> RawMessage {
    RawMessage {
        external_id: "x".into(),
        channel: Channel::Telegram,
        external_thread_id: None,
        sender_name: "A".into(),
        sender_address: "a".into(),
        text: Some("y".into()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

fn cfg() -> ChannelConfig {
    ChannelConfig {
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
    }
}

#[tokio::test]
async fn no_pipeline_means_no_classified_events() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .insert_channel_config(&cfg())
        .unwrap();

    let mut rt = Runtime::builder(Arc::clone(&store))
        // NOTE: no with_ai_pipeline
        .with_factory("Telegram", Arc::new(MockFactory { seed: vec![raw()] }))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    // Collect events for 3s. Assert at least one MessageIngested, zero MessageClassified.
    let mut saw_ingested = false;
    let mut saw_classified = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            match ev {
                RuntimeEvent::MessageIngested { .. } => saw_ingested = true,
                RuntimeEvent::MessageClassified { .. } => saw_classified = true,
                _ => {}
            }
        }
    }

    assert!(saw_ingested, "expected at least one MessageIngested");
    assert!(!saw_classified, "MessageClassified must not fire without AiPipeline");

    rt.shutdown().await;
}
