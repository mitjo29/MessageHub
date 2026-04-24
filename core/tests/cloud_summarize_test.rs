use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::actions::summarize::summarize_thread;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::{CoreError, Result};
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedCloudProvider {
    next: Mutex<Option<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedCloudProvider {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl CloudProvider for ScriptedCloudProvider {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.next
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Err(CoreError::Cloud("no canned response".into())))
    }
}

fn seed_thread(store: &Store) -> (Uuid, Uuid) {
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
            subject: Some("Project X".into()),
            participant_ids: vec![contact],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: None,
        })
        .unwrap();
    for (i, body) in ["hey", "sure, Friday?", "sounds good"].iter().enumerate() {
        let m = Message {
            id: Uuid::new_v4(),
            channel: Channel::Email,
            thread_id: thread,
            sender_id: contact,
            content: MessageContent {
                text: Some((*body).to_string()),
                html: None,
                subject: Some("Project X".into()),
                attachments: vec![],
                reply_headers: None,
            },
            timestamp: Utc::now() + chrono::Duration::seconds(i as i64),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
            external_id: None,
            received_on_channel_id: None,
        };
        store.insert_message(&m).unwrap();
    }
    (thread, contact)
}

#[tokio::test]
async fn test_summarize_thread_happy_path_persists_draft_and_log() {
    let store = Store::open_in_memory().unwrap();
    let (thread_id, _contact) = seed_thread(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"summary": "Alice confirmed Friday for Project X.", "language": "en"}"#,
    ));
    let profile = UserProfile { content: "Role: consultant".into() };
    let redactor = Redactor::from_names(vec![]);

    let draft = summarize_thread(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        thread_id,
        CloudConfig::default(),
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();

    assert_eq!(draft.action.as_str(), "summarize_thread");
    assert!(draft.output.contains("Alice"));
    assert!(draft.confidence > 0.0);

    let log = store
        .list_ai_decisions_for_entity("thread", &thread_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "summarize_thread");

    let msgs = store.list_messages_in_thread(&thread_id, 10).unwrap();
    let last = msgs.last().unwrap();
    let drafts = store.list_drafts_for_message(&last.id).unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].provider, "anthropic");
}

#[tokio::test]
async fn test_summarize_thread_rejects_empty_thread() {
    let store = Store::open_in_memory().unwrap();
    let unknown_thread = Uuid::new_v4();
    let provider = Arc::new(ScriptedCloudProvider::ok("{}"));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = summarize_thread(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        unknown_thread,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("empty") || msg.contains("thread"));
}

#[tokio::test]
async fn test_summarize_thread_surfaces_parse_error_and_logs_failure() {
    let store = Store::open_in_memory().unwrap();
    let (thread_id, _) = seed_thread(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok("this is not JSON"));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = summarize_thread(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        &profile,
        thread_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.to_lowercase().contains("json") || msg.to_lowercase().contains("cloud"));

    let log = store
        .list_ai_decisions_for_entity("thread", &thread_id.to_string())
        .unwrap();
    assert_eq!(log[0].action_type, "summarize_thread_failed");
}
