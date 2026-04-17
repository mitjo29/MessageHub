use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::{AiPipeline, LlmBackend, UserProfile};
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedLlm {
    next: Mutex<Option<Result<String>>>,
}

impl ScriptedLlm {
    fn ok(body: &str) -> Self {
        Self {
            next: Mutex::new(Some(Ok(body.to_string()))),
        }
    }
    fn err(e: messagehub_core::error::CoreError) -> Self {
        Self {
            next: Mutex::new(Some(Err(e))),
        }
    }
}

#[async_trait]
impl LlmBackend for ScriptedLlm {
    async fn complete(&self, _system: &str, _user: &str, _max_tokens: u32) -> Result<String> {
        self.next.lock().unwrap().take().unwrap()
    }
}

fn seed_sender(store: &Store) -> (Uuid, Uuid) {
    // Build a minimum contact + thread so insert_message FK constraints pass.
    let contact_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact_id,
            display_name: "Alice".to_string(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".to_string(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread_id,
            channel: Channel::Email,
            subject: Some("Hi".to_string()),
            participant_ids: vec![],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
        })
        .unwrap();
    (contact_id, thread_id)
}

fn make_msg(sender_id: Uuid, thread_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id,
        sender_id,
        content: MessageContent {
            text: Some("Are we still on for tomorrow?".to_string()),
            html: None,
            subject: Some("Project X update".to_string()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
    }
}

#[tokio::test]
async fn test_pipeline_happy_path_stores_enriched_message_and_logs_decision() {
    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::ok(
        r#"{"priority": 4, "category": "work", "reasoning": "Active project."}"#,
    ));
    let pipeline = AiPipeline::new(
        llm,
        None, // no retriever for this test
        UserProfile {
            content: "Languages: EN".to_string(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    assert!(outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert_eq!(stored.priority.unwrap().value(), 4);
    assert_eq!(stored.category.as_deref(), Some("work"));

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "classify");
    assert!(log[0].reasoning.as_deref().unwrap().contains("project"));
}

#[tokio::test]
async fn test_pipeline_graceful_degradation_on_llm_failure() {
    use messagehub_core::error::CoreError;

    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::err(CoreError::Ai("ollama down".to_string())));
    let pipeline = AiPipeline::new(
        llm,
        None,
        UserProfile {
            content: String::new(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    // Degraded mode — classification failed, message still stored.
    assert!(!outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert!(stored.priority.is_none());
    assert!(stored.category.is_none());

    // A failure row should be in the log so the UI can surface "retry classification".
    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action_type, "classify_failed");
    assert!(log[0].reasoning.as_deref().unwrap().contains("ollama down"));
}

#[tokio::test]
async fn test_pipeline_graceful_degradation_on_parse_failure() {
    let store = Store::open_in_memory().unwrap();
    let (sender_id, thread_id) = seed_sender(&store);
    let msg = make_msg(sender_id, thread_id);
    let message_id = msg.id;

    let llm = Arc::new(ScriptedLlm::ok("this is not valid json"));
    let pipeline = AiPipeline::new(
        llm,
        None,
        UserProfile {
            content: String::new(),
        },
    );

    let outcome = pipeline
        .enrich_and_store(&store, msg, "alice@example.com", "Alice")
        .await
        .unwrap();
    assert!(!outcome.classified);

    let stored = store.get_message(&message_id).unwrap();
    assert!(stored.priority.is_none());

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(log[0].action_type, "classify_failed");
}
