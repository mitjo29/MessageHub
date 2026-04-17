use std::sync::Arc;

use tracing::{debug, warn};

use crate::ai::llm::LlmBackend;
use crate::ai::prompts::{
    CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt, parse_classification_response,
};
use crate::ai::{Category, RagContext};
use crate::error::Result;
use crate::types::{Channel, PriorityScore};

/// The three fields the classifier produces.
///
/// `priority` is the 1-5 integer score; `category` is one of the
/// enumerated `Category` variants; `reasoning` is a short
/// human-readable sentence the UI can surface as "Why is this
/// prioritized?"
#[derive(Debug, Clone)]
pub struct Classification {
    pub priority: PriorityScore,
    pub category: Category,
    pub reasoning: String,
}

/// Runs the one-shot classification call against an `LlmBackend`.
///
/// The classifier holds a dynamically-dispatched backend so tests can
/// inject a scripted implementation that never touches the network.
pub struct Classifier {
    llm: Arc<dyn LlmBackend>,
    /// Cap on tokens the model is allowed to generate for a single
    /// classification response. A well-behaved response is ~50 tokens.
    /// 256 leaves headroom for chain-of-thought preambles we discard.
    max_tokens: u32,
}

impl Classifier {
    pub fn new(llm: Arc<dyn LlmBackend>) -> Self {
        Self {
            llm,
            max_tokens: 256,
        }
    }

    /// Run classification for a single message.
    ///
    /// Returns `Err(CoreError::Ai(...))` if:
    /// - the backend call fails (Ollama down, timeout, 5xx)
    /// - the response is not valid JSON matching the schema
    /// - the priority is outside 1..=5 or the category is unknown
    ///
    /// The pipeline in Task 9 catches these errors and stores the
    /// message with `priority = None` — this method should not swallow.
    pub async fn classify(
        &self,
        channel: Channel,
        sender_name: &str,
        sender_address: &str,
        subject: &str,
        body: &str,
        rag: &RagContext,
    ) -> Result<Classification> {
        let user_prompt = build_classification_user_prompt(
            channel,
            sender_name,
            sender_address,
            subject,
            body,
            rag,
        );
        debug!(
            channel = %channel,
            sender = %sender_address,
            prompt_chars = user_prompt.len(),
            "running classifier"
        );

        let raw = self
            .llm
            .complete(CLASSIFICATION_SYSTEM_PROMPT, &user_prompt, self.max_tokens)
            .await?;

        match parse_classification_response(&raw) {
            Ok(classification) => {
                debug!(
                    priority = classification.priority.value(),
                    category = classification.category.as_str(),
                    "classification succeeded"
                );
                Ok(classification)
            }
            Err(e) => {
                warn!(
                    raw_preview = %raw.chars().take(200).collect::<String>(),
                    error = %e,
                    "classifier parse failure"
                );
                Err(e)
            }
        }
    }
}
