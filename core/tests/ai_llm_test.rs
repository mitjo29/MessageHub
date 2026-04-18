use messagehub_core::ai::{LlmBackend, OllamaLlm};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_ollama_complete_posts_to_api_chat_and_parses_response() {
    let server = MockServer::start().await;

    // Canned Ollama /api/chat response for stream=false.
    let body = serde_json::json!({
        "model": "phi3:mini",
        "created_at": "2026-04-17T12:00:00Z",
        "message": { "role": "assistant", "content": "hello back" },
        "done": true
    });

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    let out = llm
        .complete("you are a test", "say hello", 32)
        .await
        .unwrap();
    assert_eq!(out, "hello back");
}

#[tokio::test]
async fn test_ollama_complete_returns_error_on_5xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    let err = llm.complete("s", "u", 32).await.unwrap_err();
    let msg = format!("{}", err);
    assert!(
        msg.contains("ollama") || msg.contains("500") || msg.contains("ai"),
        "error does not mention ollama/500/ai: {}",
        msg
    );
}

#[tokio::test]
async fn test_ollama_complete_returns_error_on_malformed_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    assert!(llm.complete("s", "u", 32).await.is_err());
}

#[tokio::test]
async fn test_ollama_health_check_hits_api_tags() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": []})))
        .expect(1)
        .mount(&server)
        .await;

    let llm = OllamaLlm::new(server.uri(), "phi3:mini".to_string());
    assert!(llm.health_check().await.unwrap());
}

#[tokio::test]
async fn test_ollama_health_check_returns_false_when_server_down() {
    // Point at a localhost port that nothing is listening on.
    let llm = OllamaLlm::new("http://127.0.0.1:1".to_string(), "phi3:mini".to_string());
    assert_eq!(llm.health_check().await.unwrap(), false);
}
