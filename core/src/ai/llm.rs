use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::error::{CoreError, Result};

/// Abstraction over a local LLM that the classifier talks to.
///
/// The only method required is `complete`: given a system prompt and a
/// user prompt, produce a single assistant response. No streaming —
/// classification is a short single-shot call.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// the AI pipeline's async tasks.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

/// HTTP client for Ollama's `/api/chat` endpoint.
///
/// Sends `stream: false` requests and reads the full JSON body. The
/// default base URL is `http://127.0.0.1:11434`, matching an Ollama
/// default installation. The model name (e.g. `"phi3:mini"`) is
/// configured at construction — the pipeline does not ship a hardcoded
/// default so tests and deployments can swap freely.
pub struct OllamaLlm {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaLlm {
    pub fn new(base_url: String, model: String) -> Self {
        // Classification should complete in <500ms on a 3B-param model but
        // cold starts can take several seconds. 60s is generous but not
        // unbounded — callers hang otherwise if the model fails to load.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client builder never fails with default config");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }

    /// Check whether Ollama is reachable and responsive.
    ///
    /// Returns `Ok(false)` if the HTTP request fails (connection refused,
    /// timeout, 5xx). Returns `Ok(true)` on any 2xx. Only propagates
    /// `Err(...)` for logic bugs — never for "server unreachable", which
    /// is an expected-runtime condition the pipeline handles gracefully.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!(error = %e, url = %url, "ollama health check failed");
                Ok(false)
            }
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatOptions {
    /// Deterministic-ish: low temperature for classification.
    temperature: f32,
    /// Cap response length.
    num_predict: i32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[async_trait]
impl LlmBackend for OllamaLlm {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            stream: false,
            options: ChatOptions {
                temperature: 0.1,
                num_predict: max_tokens as i32,
            },
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| CoreError::Ai(format!("ollama request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body_preview = resp.text().await.unwrap_or_default();
            let body_preview: String = body_preview.chars().take(200).collect();
            warn!(status = %status, body_preview = %body_preview, "ollama returned non-2xx");
            return Err(CoreError::Ai(format!(
                "ollama returned {} — {}",
                status, body_preview
            )));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| CoreError::Ai(format!("ollama response body is not valid chat JSON: {}", e)))?;

        Ok(parsed.message.content)
    }
}
