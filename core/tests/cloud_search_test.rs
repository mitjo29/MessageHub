use async_trait::async_trait;
use messagehub_core::ai::cloud::actions::search::smart_search;
use messagehub_core::ai::cloud::{CloudConfig, CloudProvider, Redactor};
use messagehub_core::ai::UserProfile;
use messagehub_core::error::Result;
use messagehub_core::store::Store;
use std::sync::{Arc, Mutex};

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
        self.next.lock().unwrap().take().unwrap()
    }
}

#[tokio::test]
async fn test_smart_search_with_no_retriever_still_calls_cloud() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "No vault results for that query.", "sources": []}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let result = smart_search(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        "what did alice say last week?",
        CloudConfig::default(),
        "claude-sonnet-4-6",
    )
    .await
    .unwrap();
    assert!(result.output.contains("No vault results"));

    // Persisted with message_id = None (smart_search has no anchor).
    let drafts = store
        .list_drafts_for_message(&uuid::Uuid::new_v4())
        .unwrap();
    assert!(drafts.is_empty());
}

#[tokio::test]
async fn test_smart_search_redacts_query_when_opted_in() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "ok", "sources": []}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec!["Alice Example".into()]);

    let _ = smart_search(
        &store,
        provider.clone() as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        "what did Alice Example say?",
        CloudConfig { redact: true },
        "m",
    )
    .await
    .unwrap();

    let prompt = provider.last_user_prompt.lock().unwrap().clone().unwrap();
    assert!(!prompt.contains("Alice Example"));
    assert!(prompt.contains("[PERSON_1]"));
}

#[tokio::test]
async fn test_smart_search_logs_audit_row() {
    let store = Store::open_in_memory().unwrap();
    let provider = Arc::new(ScriptedCloudProvider::ok(
        r#"{"answer": "answer", "sources": ["05-People/x.md"]}"#,
    ));
    let profile = UserProfile { content: String::new() };
    let redactor = Redactor::from_names(vec![]);

    let _ = smart_search(
        &store,
        provider as Arc<dyn CloudProvider>,
        &redactor,
        None,
        &profile,
        "any news?",
        CloudConfig::default(),
        "m",
    )
    .await
    .unwrap();

    // action_log keyed on entity_type = "query", entity_id = the original query.
    let rows = store
        .list_ai_decisions_for_entity("query", "any news?")
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action_type, "smart_search");
}
