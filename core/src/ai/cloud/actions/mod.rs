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
