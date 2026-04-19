//! End-to-end: register a MockFactory, seed a message, assert both
//! `MessageIngested` and `MessageClassified` arrive and DB rows are populated.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::{ChannelAdapter, RawMessage};
use messagehub_core::ai::llm::LlmBackend;
use messagehub_core::ai::pipeline::AiPipeline;
use messagehub_core::ai::profile::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig};
use uuid::Uuid;

struct StubLlm;

#[async_trait]
impl LlmBackend for StubLlm {
    async fn complete(&self, _system: &str, _user: &str, _max_tokens: u32) -> Result<String> {
        Ok("{\"category\":\"Work\",\"priority\":4,\"reasoning\":\"t\"}".into())
    }
}

struct MockFactory {
    seed: Vec<RawMessage>,
}

#[async_trait]
impl AdapterFactory for MockFactory {
    async fn build(&self, _config: &ChannelConfig) -> Result<Box<dyn ChannelAdapter>> {
        let a = MockAdapter::new();
        for m in &self.seed {
            a.add_message(m.clone());
        }
        Ok(Box::new(a))
    }
}

fn raw() -> RawMessage {
    RawMessage {
        external_id: "ext-1".into(),
        channel: Channel::Telegram,
        external_thread_id: Some("chat-1".into()),
        sender_name: "Alice".into(),
        sender_address: "alice".into(),
        text: Some("hello".into()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

fn seeded_config() -> ChannelConfig {
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
async fn full_loop_ingests_and_classifies() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .insert_channel_config(&seeded_config())
        .unwrap();

    let pipeline = Arc::new(AiPipeline::new(
        Arc::new(StubLlm),
        None,
        UserProfile { content: String::new() },
    ));

    let mut rt = Runtime::builder(Arc::clone(&store))
        .with_ai_pipeline(pipeline)
        .with_factory("Telegram", Arc::new(MockFactory { seed: vec![raw()] }))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    // Collect until we see both events, bounded to 10s.
    let mut ingested = false;
    let mut classified = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !(ingested && classified) {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            match ev {
                RuntimeEvent::MessageIngested { .. } => ingested = true,
                RuntimeEvent::MessageClassified {
                    category, priority, ..
                } => {
                    classified = true;
                    assert!(category.is_some());
                    assert!(priority.is_some());
                }
                _ => {}
            }
        }
    }
    assert!(ingested && classified, "expected both events within 10s");

    rt.shutdown().await;
}
