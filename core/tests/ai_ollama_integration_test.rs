//! Smoke test against a real local Ollama instance.
//!
//! Runs only when explicitly requested:
//!     cargo test -p messagehub-core --test ai_ollama_integration_test -- --ignored --nocapture
//!
//! Requires:
//! - Ollama running at http://127.0.0.1:11434
//! - A model named in MESSAGEHUB_TEST_MODEL (default: "phi3:mini") pulled

use messagehub_core::ai::{LlmBackend, OllamaLlm};

fn model_name() -> String {
    std::env::var("MESSAGEHUB_TEST_MODEL").unwrap_or_else(|_| "phi3:mini".to_string())
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_health_check() {
    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    assert!(
        llm.health_check().await.unwrap(),
        "Ollama is not responding at http://127.0.0.1:11434/api/tags"
    );
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_complete_returns_non_empty() {
    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    let out = llm
        .complete(
            "You are a helpful assistant.",
            "Say the word hello and nothing else.",
            32,
        )
        .await
        .unwrap();
    assert!(!out.trim().is_empty(), "ollama returned empty response");
}

#[tokio::test]
#[ignore = "requires a running Ollama with a model pulled"]
async fn test_real_ollama_can_classify_simple_message() {
    use messagehub_core::ai::prompts::{
        CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt,
        parse_classification_response,
    };
    use messagehub_core::ai::RagContext;
    use messagehub_core::types::Channel;

    let llm = OllamaLlm::new("http://127.0.0.1:11434".to_string(), model_name());
    let ctx = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Email,
        "Newsletter Bot",
        "news@promo.example",
        "50% off this weekend only!",
        "Click here to save big.",
        &ctx,
    );
    let raw = llm
        .complete(CLASSIFICATION_SYSTEM_PROMPT, &prompt, 256)
        .await
        .unwrap();

    // We don't assert an exact classification — small models vary.
    // We DO assert the response parses: if the prompt pipeline is
    // well-formed, the model stays on-schema.
    let parsed = parse_classification_response(&raw);
    assert!(
        parsed.is_ok(),
        "real Ollama response did not parse: {:?} — raw: {}",
        parsed,
        raw
    );
}
