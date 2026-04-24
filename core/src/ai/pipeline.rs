use std::sync::{Arc, Mutex};

use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::ai::classifier::Classifier;
use crate::ai::llm::LlmBackend;
use crate::ai::profile::UserProfile;
use crate::ai::rag::build_rag_context;
use crate::error::Result;
use crate::knowledge::Retriever;
use crate::store::Store;
use crate::types::{Message, PriorityScore};

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
/// The pipeline is `Clone` via the inner `Arc`s so it can be shared
/// across async tasks without ownership gymnastics.
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
    /// re-resolved from the store because the Runtime ingestor has already
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

    /// Classify an already-stored message and persist `(category, priority)`
    /// to its row. On classifier failure, writes `category="Unknown",
    /// priority=PriorityScore(1)` so the message surfaces in the UI and logs
    /// a `classify_failed` action.
    ///
    /// This is the method the runtime's `ClassifierWorker` calls. Unlike
    /// `enrich_and_store`, it does not insert — it assumes the ingestor
    /// already persisted the message.
    ///
    /// Takes `&Mutex<Store>` so the worker can share the same `Arc<Mutex<Store>>`
    /// without holding the lock across the async LLM call. The lock is
    /// acquired twice: once to read message + contact data, and once to
    /// write the classification result.
    pub async fn classify_stored(
        &self,
        store: &Mutex<Store>,
        id: &Uuid,
    ) -> Result<EnrichOutcome> {
        // Phase 1: read data under lock, then release immediately.
        let (msg_channel, sender_display_name, sender_address, subject, body, rag) = {
            let guard = store.lock().expect("store mutex poisoned");
            let msg = guard.get_message(id)?;
            let sender = guard.get_contact(&msg.sender_id)?;
            let sender_address = sender
                .identities
                .iter()
                .find(|i| i.channel == msg.channel)
                .map(|i| i.address.clone())
                .unwrap_or_default();

            let subject = msg.content.subject.clone().unwrap_or_default();
            let body = msg.content.text.clone().unwrap_or_default();

            let rag = build_rag_context(
                &*guard,
                self.retriever.as_ref(),
                &self.profile,
                msg.channel,
                &sender_address,
                &subject,
                &body,
            )?;

            (msg.channel, sender.display_name.clone(), sender_address, subject, body, rag)
            // guard drops here, lock released
        };

        // Phase 2: async LLM call — no store lock held.
        let result = self
            .classifier
            .classify(
                msg_channel,
                &sender_display_name,
                &sender_address,
                &subject,
                &body,
                &rag,
            )
            .await;

        // Phase 3: write result under a brief lock.
        let message_id_str = id.to_string();
        let guard = store.lock().expect("store mutex poisoned");
        match result {
            Ok(classification) => {
                guard.set_message_classification(
                    id,
                    Some(classification.category.as_str()),
                    Some(classification.priority),
                )?;
                guard.log_ai_decision(
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
                    "classify_stored: message classified",
                );
                Ok(EnrichOutcome { classified: true })
            }
            Err(e) => {
                guard.set_message_classification(
                    id,
                    Some("Unknown"),
                    Some(PriorityScore::new(1).unwrap()),
                )?;
                let reason = format!("classification failed: {}", e);
                if let Err(log_err) = guard.log_ai_decision(
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
                    "classify_stored: degraded mode",
                );
                Ok(EnrichOutcome { classified: false })
            }
        }
    }
}

#[cfg(test)]
mod classify_stored_tests {
    use super::*;
    use crate::ai::llm::LlmBackend;
    use crate::ai::profile::UserProfile;
    use crate::store::Store;
    use crate::types::{Channel, Message, MessageContent};
    use async_trait::async_trait;

    struct StubLlm;
    #[async_trait]
    impl LlmBackend for StubLlm {
        async fn complete(
            &self,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> crate::error::Result<String> {
            Ok(r#"{"category":"Work","priority":4,"reasoning":"test"}"#.to_string())
        }
    }

    struct ErroringLlm;
    #[async_trait]
    impl LlmBackend for ErroringLlm {
        async fn complete(
            &self,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> crate::error::Result<String> {
            Err(crate::error::CoreError::InvalidInput(
                "simulated llm failure".to_string(),
            ))
        }
    }

    async fn setup_stored_message() -> (std::sync::Mutex<Store>, Uuid) {
        let store = Store::open_in_memory().unwrap();
        let contact = store
            .find_or_create_contact_by_address(Channel::Telegram, "u1", "User")
            .unwrap();
        let thread_id = Uuid::new_v4();
        store
            .insert_thread(&crate::types::Thread {
                id: thread_id,
                channel: Channel::Telegram,
                subject: None,
                participant_ids: vec![],
                message_count: 0,
                last_message_at: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                external_thread_id: None,
            })
            .unwrap();
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            thread_id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some("Hello".to_string()),
                html: None,
                subject: None,
                attachments: vec![],
                reply_headers: None,
            },
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
            external_id: None,
            received_on_channel_id: None,
        };
        let id = msg.id;
        store.insert_message(&msg).unwrap();
        (std::sync::Mutex::new(store), id)
    }

    #[tokio::test]
    async fn classify_stored_happy_path_updates_row() {
        let (store, id) = setup_stored_message().await;
        let pipeline = AiPipeline::new(
            Arc::new(StubLlm),
            None,
            UserProfile { content: String::new() },
        );

        let outcome = pipeline.classify_stored(&store, &id).await.unwrap();
        assert!(outcome.classified);

        let reloaded = store.lock().unwrap().get_message(&id).unwrap();
        assert!(reloaded.category.is_some());
        assert!(reloaded.priority.is_some());
    }

    #[tokio::test]
    async fn classify_stored_llm_failure_writes_degraded_row() {
        let (store, id) = setup_stored_message().await;
        let pipeline = AiPipeline::new(
            Arc::new(ErroringLlm),
            None,
            UserProfile { content: String::new() },
        );

        let outcome = pipeline.classify_stored(&store, &id).await.unwrap();
        assert!(!outcome.classified);

        let reloaded = store.lock().unwrap().get_message(&id).unwrap();
        assert!(reloaded.category.is_some());
        assert_eq!(reloaded.priority, Some(PriorityScore::new(1).unwrap()));
    }
}
