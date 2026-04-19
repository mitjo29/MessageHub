//! LLM always errors → message is still inserted, `MessageClassified` fires
//! with the fallback (Low priority = PriorityScore(1), non-None category).

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
use messagehub_core::error::{CoreError, Result};
use messagehub_core::runtime::events::RuntimeEvent;
use messagehub_core::runtime::factory::AdapterFactory;
use messagehub_core::runtime::Runtime;
use messagehub_core::runtime::status::ChannelStatus;
use messagehub_core::store::Store;
use messagehub_core::types::{Channel, ChannelConfig, PriorityScore};
use uuid::Uuid;

struct BrokenLlm;

#[async_trait]
impl LlmBackend for BrokenLlm {
    async fn complete(&self, _system: &str, _user: &str, _max_tokens: u32) -> Result<String> {
        Err(CoreError::InvalidInput("down".into()))
    }
}

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

#[tokio::test]
async fn llm_failure_still_classifies_with_low_priority_fallback() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
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
        .with_ai_pipeline(Arc::new(AiPipeline::new(
            Arc::new(BrokenLlm),
            None,
            UserProfile { content: String::new() },
        )))
        .with_factory("Telegram", Arc::new(MockFactory))
        .build();
    let mut events = rt.subscribe();
    rt.start().await.unwrap();

    let low_fallback = PriorityScore::new(1).unwrap();
    let mut fallback_seen = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline && !fallback_seen {
        if let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            if let RuntimeEvent::MessageClassified { priority, .. } = ev {
                assert_eq!(
                    priority,
                    Some(low_fallback),
                    "expected Low fallback priority (PriorityScore(1))"
                );
                fallback_seen = true;
            }
        }
    }
    assert!(fallback_seen, "MessageClassified with Low fallback must fire");

    rt.shutdown().await;
}
