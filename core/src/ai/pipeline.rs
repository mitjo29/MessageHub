use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::ai::classifier::Classifier;
use crate::ai::llm::LlmBackend;
use crate::ai::profile::UserProfile;
use crate::ai::rag::build_rag_context;
use crate::error::Result;
use crate::knowledge::Retriever;
use crate::store::Store;
use crate::types::Message;

/// Outcome of processing a single message.
#[derive(Debug, Clone, Copy)]
pub struct EnrichOutcome {
    /// True when the LLM succeeded and a priority + category were attached.
    /// False when classification failed and the message was stored in
    /// degraded (priority = None, category = None) form.
    pub classified: bool,
}

/// Top-level AI pipeline.
///
/// Holds the pieces the classifier needs (`LlmBackend`, optional
/// `Retriever`, `UserProfile`) and exposes a single `enrich_and_store`
/// entry point that the channel adapter manager calls for every
/// incoming normalized `Message`.
///
/// The pipeline is `Clone` via the inner `Arc`s so it can be passed
/// into the `AdapterManager::on_messages` callback closure without
/// ownership gymnastics.
#[derive(Clone)]
pub struct AiPipeline {
    classifier: Arc<Classifier>,
    retriever: Option<Arc<Retriever>>,
    profile: Arc<UserProfile>,
}

impl AiPipeline {
    pub fn new(
        llm: Arc<dyn LlmBackend>,
        retriever: Option<Arc<Retriever>>,
        profile: UserProfile,
    ) -> Self {
        Self {
            classifier: Arc::new(Classifier::new(llm)),
            retriever,
            profile: Arc::new(profile),
        }
    }

    /// Classify a message, attach `priority` + `category`, persist via
    /// `Store::insert_message`, and log the decision to `action_log`.
    ///
    /// `sender_address` and `sender_name` are passed through rather than
    /// re-resolved from the store because the adapter manager has already
    /// done that lookup to produce the `Message::sender_id`.
    ///
    /// Graceful degradation: if classification fails for any reason
    /// (LLM down, parse error, bad output), the message is stored
    /// without a priority/category and a `classify_failed` row is
    /// written to the log. The outer `Result` only returns `Err` for
    /// storage failures (which are unrecoverable).
    pub async fn enrich_and_store(
        &self,
        store: &Store,
        mut msg: Message,
        sender_address: &str,
        sender_name: &str,
    ) -> Result<EnrichOutcome> {
        let subject = msg.content.subject.clone().unwrap_or_default();
        let body = msg.content.text.clone().unwrap_or_default();

        let rag = build_rag_context(
            store,
            self.retriever.as_ref(),
            &self.profile,
            msg.channel,
            sender_address,
            &subject,
            &body,
        )?;

        let classification_result = self
            .classifier
            .classify(
                msg.channel,
                sender_name,
                sender_address,
                &subject,
                &body,
                &rag,
            )
            .await;

        let message_id_str = msg.id.to_string();

        match classification_result {
            Ok(classification) => {
                msg.priority = Some(classification.priority);
                msg.category = Some(classification.category.as_str().to_string());
                store.insert_message(&msg)?;
                // Confidence score: we don't yet expose model log-probs; use
                // 1.0 for parsed successes. Plan 5 can refine this when cloud
                // tier exposes confidence.
                store.log_ai_decision(
                    "classify",
                    "message",
                    &message_id_str,
                    &classification.reasoning,
                    1.0,
                )?;
                info!(
                    message_id = %message_id_str,
                    priority = classification.priority.value(),
                    category = classification.category.as_str(),
                    "message classified and stored"
                );
                Ok(EnrichOutcome { classified: true })
            }
            Err(e) => {
                // Degraded mode: store the message without priority and log
                // the failure so the UI can offer a retry.
                store.insert_message(&msg)?;
                let reason = format!("classification failed: {}", e);
                if let Err(log_err) = store.log_ai_decision(
                    "classify_failed",
                    "message",
                    &message_id_str,
                    &reason,
                    0.0,
                ) {
                    warn!(error = %log_err, "failed to log classification failure");
                }
                debug!(
                    message_id = %message_id_str,
                    error = %e,
                    "classification failed; stored in degraded mode"
                );
                Ok(EnrichOutcome { classified: false })
            }
        }
    }
}
