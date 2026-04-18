use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::error::{CoreError, Result};

/// Abstraction over a cloud LLM that the action orchestrators talk to.
///
/// Mirrors the shape of `ai::llm::LlmBackend` deliberately: one method
/// taking a system prompt, a user prompt, and a max-tokens cap, returning
/// the assistant's plain-text output. Tests inject scripted implementations
/// that skip the network entirely (see `ScriptedCloudProvider` in the
/// per-action integration tests).
#[async_trait]
pub trait CloudProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

/// HTTP client for Anthropic's `/v1/messages` endpoint.
///
/// Holds the API key, the model name (e.g. `"claude-sonnet-4-6"`), and
/// the base URL (configurable for wiremock tests). Sends `stream: false`
/// requests and joins every `type: "text"` block in the response `content`
/// array into one string.
///
/// The constructor intentionally does NOT read from the environment —
/// credential policy (env, keychain, config file) is the caller's
/// responsibility.
pub struct AnthropicCloud {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicCloud {
    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client builder never fails with default config");
        Self {
            client,
            api_key,
            model,
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Override the base URL. Primary use: point tests at a wiremock
    /// server. Production callers should leave this at the default.
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Check whether the Anthropic endpoint is reachable with the
    /// configured API key.
    ///
    /// Returns `Ok(false)` on any network-level failure (connection
    /// refused, timeout, DNS). Returns `Ok(true)` on any 2xx. Only
    /// propagates `Err(...)` for logic bugs — consistent with Plan 4's
    /// `OllamaLlm::health_check`.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v1/models", self.base_url);
        match self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
        {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!(error = %e, url = %url, "anthropic health check failed");
                Ok(false)
            }
        }
    }
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    system: &'a str,
    messages: Vec<MessagesInput<'a>>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Serialize)]
struct MessagesInput<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    #[serde(other)]
    Other,
}

#[async_trait]
impl CloudProvider for AnthropicCloud {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = MessagesRequest {
            model: &self.model,
            system,
            messages: vec![MessagesInput {
                role: "user",
                content: user,
            }],
            max_tokens,
            temperature: 0.3,
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Cloud(format!("anthropic request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let preview: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            warn!(status = %status, body_preview = %preview, "anthropic returned non-2xx");
            return Err(CoreError::Cloud(format!(
                "anthropic returned {} — {}",
                status, preview
            )));
        }

        let parsed: MessagesResponse = resp.json().await.map_err(|e| {
            CoreError::Cloud(format!("anthropic response body malformed: {}", e))
        })?;

        let mut out = String::new();
        for block in parsed.content {
            if let ContentBlock::Text { text } = block {
                out.push_str(&text);
            }
        }
        if out.is_empty() {
            return Err(CoreError::Cloud(
                "anthropic returned no text blocks".to_string(),
            ));
        }
        Ok(out)
    }
}
