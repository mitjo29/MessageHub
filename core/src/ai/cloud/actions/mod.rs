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
    /// The existing `draft_reply(&Store, ...)` forces callers to hold a
    /// `std::sync::MutexGuard<Store>` across the HTTP await, which is `!Send`
    /// and breaks async Tauri commands. This wrapper moves the whole call
    /// onto a `spawn_blocking` task that builds a current-thread tokio
    /// runtime internally, locks the mutex in that scope, and block_ons the
    /// async call. The outer future stays Send.
    pub async fn draft_reply_via(
        &self,
        store: std::sync::Arc<std::sync::Mutex<Store>>,
        message_id: Uuid,
        cfg: CloudConfig,
    ) -> Result<DraftOutcome> {
        let provider = self.provider.clone();
        let redactor = self.redactor.clone();
        let retriever = self.retriever.clone();
        let profile = self.profile.clone();
        let model = self.model.clone();

        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| crate::error::CoreError::Channel(format!("runtime: {}", e)))?;
            let guard = store.lock().map_err(|e| {
                crate::error::CoreError::Channel(format!("store mutex poisoned: {}", e))
            })?;
            rt.block_on(draft::draft_reply(
                &*guard,
                provider,
                &redactor,
                retriever.as_ref(),
                &profile,
                message_id,
                cfg,
                &model,
            ))
        })
        .await
        .map_err(|e| crate::error::CoreError::Channel(format!("spawn_blocking: {}", e)))?
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
