use async_trait::async_trait;
use messagehub_core::ai::{Category, Classifier, LlmBackend, RagContext};
use messagehub_core::error::{CoreError, Result};
use messagehub_core::types::Channel;
use std::sync::Arc;
use std::sync::Mutex;

struct ScriptedLlm {
    responses: Mutex<Vec<Result<String>>>,
    last_user_prompt: Mutex<Option<String>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<Result<String>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            last_user_prompt: Mutex::new(None),
        }
    }
}

#[async_trait]
impl LlmBackend for ScriptedLlm {
    async fn complete(&self, _system: &str, user: &str, _max_tokens: u32) -> Result<String> {
        *self.last_user_prompt.lock().unwrap() = Some(user.to_string());
        self.responses.lock().unwrap().remove(0)
    }
}

fn empty_ctx() -> RagContext {
    RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    }
}

#[tokio::test]
async fn test_classify_happy_path() {
    let llm = Arc::new(ScriptedLlm::new(vec![Ok(
        r#"{"priority": 4, "category": "family", "reasoning": "Daughter's school."}"#.to_string(),
    )]));
    let classifier = Classifier::new(llm.clone());
    let result = classifier
        .classify(
            Channel::Email,
            "Alice",
            "alice@example.com",
            "School",
            "Hi",
            &empty_ctx(),
        )
        .await
        .unwrap();
    assert_eq!(result.priority.value(), 4);
    assert_eq!(result.category, Category::Family);
    assert!(result.reasoning.contains("school") || result.reasoning.contains("School"));

    let prompt = llm.last_user_prompt.lock().unwrap().clone().unwrap();
    assert!(prompt.contains("alice@example.com"));
    assert!(prompt.contains("School"));
}

#[tokio::test]
async fn test_classify_surfaces_llm_error() {
    let llm = Arc::new(ScriptedLlm::new(vec![Err(CoreError::Ai(
        "backend down".to_string(),
    ))]));
    let classifier = Classifier::new(llm);
    let err = classifier
        .classify(
            Channel::Email,
            "Alice",
            "alice@example.com",
            "s",
            "b",
            &empty_ctx(),
        )
        .await
        .unwrap_err();
    assert!(format!("{}", err).contains("backend down"));
}

#[tokio::test]
async fn test_classify_surfaces_parse_error() {
    let llm = Arc::new(ScriptedLlm::new(vec![Ok(
        "I think this is work priority 3".to_string(), // not JSON
    )]));
    let classifier = Classifier::new(llm);
    let err = classifier
        .classify(Channel::Email, "A", "a@x.com", "s", "b", &empty_ctx())
        .await
        .unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("JSON") || msg.contains("ai"));
}
