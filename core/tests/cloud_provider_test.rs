use messagehub_core::ai::cloud::{AnthropicCloud, CloudProvider};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn canned_response_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "content": [
            { "type": "text", "text": text }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })
}

#[tokio::test]
async fn test_anthropic_complete_posts_to_v1_messages_with_auth_headers() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_response_body("hello back")))
        .expect(1)
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("test-key".into(), "claude-sonnet-4-6".into())
        .with_base_url(server.uri());
    let out = provider
        .complete("sys prompt", "user prompt", 128)
        .await
        .unwrap();
    assert_eq!(out, "hello back");
}

#[tokio::test]
async fn test_anthropic_complete_joins_multiple_text_blocks() {
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "content": [
            { "type": "text", "text": "first " },
            { "type": "text", "text": "second" }
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1, "output_tokens": 1 }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    let out = provider.complete("s", "u", 64).await.unwrap();
    assert_eq!(out, "first second");
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    let err = provider.complete("s", "u", 64).await.unwrap_err();
    let msg = format!("{}", err);
    assert!(
        msg.contains("anthropic") || msg.contains("401") || msg.contains("cloud"),
        "error does not mention anthropic/401/cloud: {}",
        msg
    );
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_5xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    assert!(provider.complete("s", "u", 64).await.is_err());
}

#[tokio::test]
async fn test_anthropic_complete_returns_error_on_malformed_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let provider = AnthropicCloud::new("k".into(), "m".into()).with_base_url(server.uri());
    assert!(provider.complete("s", "u", 64).await.is_err());
}

#[tokio::test]
async fn test_anthropic_health_check_returns_false_when_unreachable() {
    let provider = AnthropicCloud::new("k".into(), "m".into())
        .with_base_url("http://127.0.0.1:1".into());
    assert_eq!(provider.health_check().await.unwrap(), false);
}
