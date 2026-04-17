use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use messagehub_core::adapters::mock::MockAdapter;
use messagehub_core::adapters::manager::AdapterManager;
use messagehub_core::adapters::{normalize, RawMessage, ChannelAdapter};
use messagehub_core::types::{Channel, ChannelConfig, MessageContent};

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
async fn test_full_lifecycle_with_mock() {
    let mock = MockAdapter::new().with_channel(Channel::Email);
    mock.add_message(make_raw_message(Channel::Email, "msg-1", "Hello from email"));
    mock.add_message(make_raw_message(Channel::Email, "msg-2", "Second email"));

    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = Arc::clone(&received);

    let mut manager = AdapterManager::new(move |msgs| {
        received_clone.fetch_add(msgs.len(), Ordering::Relaxed);
    });

    let config = make_config(Channel::Email, "test@example.com");
    let config_id = manager.register(Box::new(mock), config).await.unwrap();

    manager.start_sync(config_id).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(received.load(Ordering::Relaxed) >= 2);

    let adapter = manager.get_adapter(&config_id).unwrap();
    {
        let adapter = adapter.lock().await;
        let content = MessageContent {
            text: Some("Reply content".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
        };
        adapter.send_reply("thread-1", &content).await.unwrap();
    }

    manager.shutdown().await.unwrap();
    assert_eq!(manager.registered_configs().len(), 0);
}

#[tokio::test]
async fn test_multiple_adapters_independent() {
    let email_count = Arc::new(AtomicUsize::new(0));
    let telegram_count = Arc::new(AtomicUsize::new(0));
    let email_clone = Arc::clone(&email_count);
    let telegram_clone = Arc::clone(&telegram_count);

    let mut manager = AdapterManager::new(move |msgs| {
        for msg in &msgs {
            match msg.channel {
                Channel::Email => {
                    email_clone.fetch_add(1, Ordering::Relaxed);
                }
                Channel::Telegram => {
                    telegram_clone.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    });

    let email_mock = MockAdapter::new().with_channel(Channel::Email);
    email_mock.add_message(make_raw_message(Channel::Email, "e1", "Email 1"));
    let email_config = make_config(Channel::Email, "email");
    let email_id = manager
        .register(Box::new(email_mock), email_config)
        .await
        .unwrap();

    let tg_mock = MockAdapter::new().with_channel(Channel::Telegram);
    tg_mock.add_message(make_raw_message(Channel::Telegram, "t1", "Telegram 1"));
    tg_mock.add_message(make_raw_message(Channel::Telegram, "t2", "Telegram 2"));
    let tg_config = make_config(Channel::Telegram, "telegram");
    let tg_id = manager
        .register(Box::new(tg_mock), tg_config)
        .await
        .unwrap();

    manager.start_sync(email_id).unwrap();
    manager.start_sync(tg_id).unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(email_count.load(Ordering::Relaxed) >= 1);
    assert!(telegram_count.load(Ordering::Relaxed) >= 2);

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_adapter_failure_does_not_crash_manager() {
    let mut manager = AdapterManager::new(|_| {});

    let failing_mock = MockAdapter::new();
    failing_mock.set_fail_fetch(true);

    let config = make_config(Channel::Telegram, "failing");
    let config_id = manager
        .register(Box::new(failing_mock), config)
        .await
        .unwrap();

    manager.start_sync(config_id).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(manager.is_syncing(&config_id));

    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_connect_failure_prevents_registration() {
    let mut manager = AdapterManager::new(|_| {});

    let failing_mock = MockAdapter::new();
    failing_mock.set_fail_connect(true);

    let config = make_config(Channel::Telegram, "fail-connect");
    let result = manager.register(Box::new(failing_mock), config).await;

    assert!(result.is_err());
    assert_eq!(manager.registered_configs().len(), 0);
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
    let mock = MockAdapter::new().with_channel(Channel::Sms);
    let mut adapter: Box<dyn ChannelAdapter> = Box::new(mock);

    let config = make_config(Channel::Sms, "sms-test");
    adapter.connect(&config).await.unwrap();

    assert_eq!(adapter.channel_type(), Channel::Sms);

    let messages = adapter.fetch_messages(None).await.unwrap();
    assert!(messages.is_empty());

    adapter.disconnect().await.unwrap();
}
