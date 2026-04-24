use async_trait::async_trait;
use chrono::Utc;
use messagehub_core::ai::cloud::actions::draft::draft_reply;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct ScriptedCloudProvider {
    responses: Mutex<Vec<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedCloudProvider {
    fn ok(body: &str) -> Self {
        Self {
            responses: Mutex::new(vec![Ok(body.to_string())]),
            last_user_prompt: Mutex::new(None),
        }
    }

    /// Respond with the same body on every call (for regenerate-style tests).
    fn repeating(body: &str) -> Self {
        // We pre-load enough copies for typical test usage (2 calls).
        Self {
            responses: Mutex::new(vec![Ok(body.to_string()), Ok(body.to_string())]),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl CloudProvider for ScriptedCloudProvider {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        // Pop from the front so calls are served in order.
        let mut guard = self.responses.lock().unwrap();
        assert!(!guard.is_empty(), "ScriptedCloudProvider: no more scripted responses");
        guard.remove(0)
    }
}

fn seed(store: &Store) -> (Uuid, Uuid) {
    let c = Uuid::new_v4();
    let t = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: c,
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
            id: t,
            channel: Channel::Email,
            subject: Some("Meeting".into()),
            participant_ids: vec![c],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: None,
        })
        .unwrap();
    let m = Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: t,
        sender_id: c,
        content: MessageContent {
            text: Some("Are we still on for tomorrow, Alice Example?".into()),
            html: None,
            subject: Some("Meeting".into()),
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
    store.insert_message(&m).unwrap();
    (m.id, t)
}

#[tokio::test]
async fn test_draft_reply_un_redacts_person_tokens_in_output() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "Hi [PERSON_1], yes tomorrow works.", "language": "en"}"#,
    ));
    let profile = UserProfile { content: "Role: consultant".into() };
    let redactor = Redactor::from_names(vec!["Alice Example".into()]);

    let draft = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig { redact: true },
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();

    // No token should remain in the final output.
    assert!(!draft.output.contains("[PERSON_"));
    assert!(draft.output.contains("tomorrow"));
}

#[tokio::test]
async fn test_draft_reply_rejects_unknown_language_code() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "ola", "language": "pt"}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let err = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap_err();
    assert!(format!("{}", err).to_lowercase().contains("language"));
}

#[tokio::test]
async fn test_draft_reply_persists_audit_row() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"draft": "ok", "language": "en"}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let _ = draft_reply(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap();

    let log = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert!(log.iter().any(|d| d.action_type == "draft_reply"));

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 1);
}

#[tokio::test]
async fn regenerate_inserts_second_draft_row() {
    let store = Store::open_in_memory().unwrap();
    let (message_id, _) = seed(&store);

    // Provider scripted to respond twice with a valid draft JSON.
    let provider = Arc::new(ScriptedCloudProvider::repeating(
        r#"{"draft": "ok", "language": "en"}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    // First call — initial draft.
    let _out1 = draft_reply(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .expect("first draft");

    // Second call — regenerate (same message_id).
    let _out2 = draft_reply(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        message_id,
        CloudConfig::default(),
        "m",
    )
    .await
    .expect("second draft");

    let rows = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(rows.len(), 2, "regenerate should insert a second row, not overwrite");
    assert!(
        rows.iter().all(|r| r.action_type == "draft_reply"),
        "all rows should have action_type == draft_reply"
    );
}
