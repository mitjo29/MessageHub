pub mod draft;
pub mod search;
pub mod summarize;

use std::sync::Arc;

use uuid::Uuid;

use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::Redactor;
use crate::ai::cloud::CloudConfig;
use crate::ai::profile::UserProfile;
use crate::error::Result;
use crate::knowledge::Retriever;
use crate::store::Store;

pub use summarize::DraftOutcome;

/// The single entry point the app binary uses for cloud actions.
///
/// Holds the provider, redactor, optional retriever, and user profile
/// so each action call is a one-liner at the call site.
pub struct CloudActions {
    provider: Arc<dyn CloudProvider>,
    redactor: Redactor,
    retriever: Option<Arc<Retriever>>,
    profile: Arc<UserProfile>,
    model: String,
}

impl CloudActions {
    pub fn new(
        provider: Arc<dyn CloudProvider>,
        redactor: Redactor,
        retriever: Option<Arc<Retriever>>,
        profile: UserProfile,
        model: String,
    ) -> Self {
        Self {
            provider,
            redactor,
            retriever,
            profile: Arc::new(profile),
            model,
        }
    }

    pub async fn summarize_thread(
        &self,
        store: &Store,
        thread_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        summarize::summarize_thread(
            store,
            self.provider.clone(),
            &self.redactor,
            &self.profile,
            thread_id,
            cfg,
            &self.model,
        )
        .await
    }

    pub async fn draft_reply(
        &self,
        store: &Store,
        message_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        draft::draft_reply(
            store,
            self.provider.clone(),
            &self.redactor,
            self.retriever.as_ref(),
            &self.profile,
            message_id,
            cfg,
            &self.model,
        )
        .await
    }

    /// Async variant of `draft_reply` that owns its store-access discipline.
    ///
    /// Unlike the `&Store` variant, this takes `Arc<Mutex<Store>>` and
    /// acquires the lock only for brief read/write phases, releasing it
    /// before the Anthropic HTTP call. This satisfies the project-wide
    /// lock discipline: never hold the store mutex across `.await`.
    pub async fn draft_reply_via(
        &self,
        store: std::sync::Arc<std::sync::Mutex<Store>>,
        message_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        use crate::ai::cloud::confidence::derive_confidence;
        use crate::ai::cloud::redactor::un_redact;
        use crate::ai::rag::build_rag_context;
        use crate::store::NewDraft;

        // Phase 1: read-side work under a brief lock.
        let (subject, sender_addr, channel, redacted_body, reverse_map, rag, retrieval_sims) = {
            let guard = store.lock().map_err(|e| {
                crate::error::CoreError::Channel(format!("store mutex poisoned: {}", e))
            })?;

            let message = guard.get_message(&message_id)?;
            let contact = guard.get_contact(&message.sender_id)?;
            let sender_addr = contact
                .identities
                .iter()
                .find(|id| id.channel == message.channel)
                .map(|id| id.address.clone())
                .unwrap_or_default();

            let subject = message.content.subject.clone().unwrap_or_default();
            let body = message.content.text.clone().unwrap_or_default();

            let (redacted_body, reverse_map) = if cfg.redact {
                self.redactor.redact(&body)
            } else {
                (body.clone(), Default::default())
            };

            let retrieval_sims: Vec<f32> = match self.retriever.as_ref() {
                Some(r) => r
                    .search(
                        &*guard,
                        &format!("{} {}", subject, redacted_body),
                        &crate::knowledge::RetrievalFilters::default(),
                    )?
                    .into_iter()
                    .map(|c| draft::similarity_from_distance(c.distance))
                    .collect(),
                None => Vec::new(),
            };

            let rag = build_rag_context(
                &*guard,
                self.retriever.as_ref(),
                &self.profile,
                message.channel,
                &sender_addr,
                &subject,
                &redacted_body,
            )?;

            (
                subject,
                sender_addr,
                message.channel,
                redacted_body,
                reverse_map,
                rag,
                retrieval_sims,
            )
        };
        // Lock is dropped here.

        // Phase 2: HTTP call WITHOUT the lock.
        let user_prompt = draft::build_user_prompt(
            &channel.to_string(),
            &sender_addr,
            &subject,
            &redacted_body,
            &rag,
        );

        let raw = self
            .provider
            .complete(draft::SYSTEM_PROMPT, &user_prompt, 1024)
            .await?;

        let parsed = draft::parse_response(&raw)?;
        let un_redacted = un_redact(&parsed.draft, &reverse_map);
        let final_output = draft::LEFTOVER_TOKEN_RE
            .replace_all(&un_redacted, "")
            .to_string();
        let confidence = derive_confidence(&rag, &retrieval_sims);
        let draft_id = Uuid::new_v4();

        // Phase 3: write-side work under a brief lock.
        {
            let guard = store.lock().map_err(|e| {
                crate::error::CoreError::Channel(format!("store mutex poisoned: {}", e))
            })?;

            guard.insert_draft(&NewDraft {
                id: draft_id,
                message_id: Some(message_id),
                action_type: crate::ai::cloud::CloudAction::DraftReply.as_str(),
                input_redacted: &redacted_body,
                output: &final_output,
                confidence,
                provider: "anthropic",
                model: &self.model,
            })?;
            guard.log_ai_decision(
                crate::ai::cloud::CloudAction::DraftReply.as_str(),
                "message",
                &message_id.to_string(),
                &format!("language={}", parsed.language),
                confidence as f64,
            )?;
        }
        // Lock dropped.

        Ok(DraftOutcome {
            id: draft_id,
            action: crate::ai::cloud::CloudAction::DraftReply,
            output: final_output,
            confidence,
        })
    }

    pub async fn smart_search(
        &self,
        store: &Store,
        query: &str,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        search::smart_search(
            store,
            self.provider.clone(),
            &self.redactor,
            self.retriever.as_ref(),
            &self.profile,
            query,
            cfg,
            &self.model,
        )
        .await
    }
}
