use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::{CloudActions, CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct SequencedProvider {
    responses: Mutex<Vec<Result<String>>>,
}

impl SequencedProvider {
    fn new(rs: Vec<Result<String>>) -> Self {
        Self {
            responses: Mutex::new(rs),
        }
    }
}

#[async_trait]
impl CloudProvider for SequencedProvider {
    async fn complete(&self, _s: &str, _u: &str, _m: u32) -> Result<String> {
        self.responses.lock().unwrap().remove(0)
    }
}

#[tokio::test]
async fn test_facade_handles_three_actions_in_sequence() {
    let store = Store::open_in_memory().unwrap();

    // Seed one contact + one thread + one message for the first two actions.
    let contact = Uuid::new_v4();
    let thread = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread,
            channel: Channel::Email,
            subject: Some("T".into()),
            participant_ids: vec![contact],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: None,
        })
        .unwrap();
    let msg = Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: thread,
        sender_id: contact,
        content: MessageContent {
            text: Some("hey".into()),
            html: None,
            subject: Some("T".into()),
            attachments: vec![],
            reply_headers: None,
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
        external_id: None,
        received_on_channel_id: None,
    };
    store.insert_message(&msg).unwrap();
    let message_id = msg.id;

    let provider = Arc::new(SequencedProvider::new(vec![
        Ok(r#"{"summary": "short thread", "language": "en"}"#.into()),
        Ok(r#"{"draft": "ok", "language": "en"}"#.into()),
        Ok(r#"{"answer": "no matches", "sources": []}"#.into()),
    ]));
    let actions = CloudActions::new(
        provider as Arc<dyn CloudProvider>,
        Redactor::from_names(vec![]),
        None,
        UserProfile { content: String::new() },
        "claude-sonnet-4-6".into(),
    );

    let sum = actions
        .summarize_thread(&store, thread, CloudConfig::default())
        .await
        .unwrap();
    assert!(sum.output.contains("short thread"));

    let dr = actions
        .draft_reply(&store, message_id, CloudConfig::default())
        .await
        .unwrap();
    assert_eq!(dr.output, "ok");

    let ss = actions
        .smart_search(&store, "anything new?", CloudConfig::default())
        .await
        .unwrap();
    assert!(ss.output.contains("no matches"));
}
