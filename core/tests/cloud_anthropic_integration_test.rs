//! Smoke tests against the real Anthropic API.
//!
//! Run with:
//!     ANTHROPIC_API_KEY=sk-ant-... cargo test -p messagehub-core \
//!         --test cloud_anthropic_integration_test -- --ignored --nocapture
//!
//! Requires an `ANTHROPIC_API_KEY` environment variable. The default
//! model is `claude-sonnet-4-6`; override with `MESSAGEHUB_CLOUD_MODEL`.

use messagehub_core::ai::cloud::{AnthropicCloud, CloudProvider};

fn api_key() -> String {
    std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY env var required")
}

fn model() -> String {
    std::env::var("MESSAGEHUB_CLOUD_MODEL").unwrap_or_else(|_| "claude-sonnet-4-6".into())
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY and hits the real API"]
async fn test_real_anthropic_health_check() {
    let provider = AnthropicCloud::new(api_key(), model());
    let ok = provider.health_check().await.unwrap();
    assert!(ok, "Anthropic health check failed");
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY and hits the real API"]
async fn test_real_anthropic_complete_returns_non_empty() {
    let provider = AnthropicCloud::new(api_key(), model());
    let out = provider
        .complete(
            "You are a test assistant. Respond with the single word: hello.",
            "Say hello.",
            32,
        )
        .await
        .unwrap();
    assert!(!out.trim().is_empty(), "empty response from Anthropic");
}
