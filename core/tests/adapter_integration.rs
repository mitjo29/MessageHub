use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use messagehub_core::adapters::{normalize, RawMessage, ChannelAdapter};
use messagehub_core::types::{Channel, ChannelConfig};

fn make_config(channel: Channel, label: &str) -> ChannelConfig {
    ChannelConfig {
        id: Uuid::new_v4(),
        channel,
        label: label.to_string(),
        keychain_ref: "test-key".to_string(),
        enabled: true,
        poll_interval_secs: 1,
        last_sync_cursor: None,
        last_sync_at: None,
        status: messagehub_core::runtime::status::ChannelStatus::Healthy,
        last_error: None,
        consecutive_failures: 0,
    }
}

fn make_raw_message(channel: Channel, id: &str, text: &str) -> RawMessage {
    RawMessage {
        external_id: id.to_string(),
        channel,
        external_thread_id: Some("thread-1".to_string()),
        sender_name: "Alice".to_string(),
        sender_address: "alice@example.com".to_string(),
        text: Some(text.to_string()),
        html: None,
        subject: None,
        attachments: vec![],
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_normalize_roundtrip() {
    let raw = make_raw_message(Channel::Email, "ext-1", "Test content");
    let sender_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();

    let message = normalize(raw, sender_id, thread_id);

    assert_eq!(message.channel, Channel::Email);
    assert_eq!(message.sender_id, sender_id);
    assert_eq!(message.thread_id, thread_id);
    assert_eq!(message.content.text.as_deref(), Some("Test content"));
    assert!(!message.is_read);
    assert!(!message.is_archived);
    assert!(message.priority.is_none());
}

#[tokio::test]
async fn test_mock_adapter_trait_object() {
    use messagehub_core::adapters::mock::MockAdapter;

    let mock = MockAdapter::new().with_channel(Channel::Sms);
    let mut adapter: Box<dyn ChannelAdapter> = Box::new(mock);

    let config = make_config(Channel::Sms, "sms-test");
    adapter.connect(&config).await.unwrap();

    assert_eq!(adapter.channel_type(), Channel::Sms);

    let messages = adapter.fetch_messages(None).await.unwrap();
    assert!(messages.is_empty());

    adapter.disconnect().await.unwrap();
}
