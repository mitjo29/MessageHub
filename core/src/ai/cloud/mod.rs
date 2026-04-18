//! Tier 2 cloud actions (user-triggered, opt-in per call).
//!
//! This module provides three cloud-backed actions that run against
//! Anthropic's API:
//!
//! - `summarize_thread` — condense a conversation with vault context.
//! - `draft_reply`     — compose a language-matched reply draft.
//! - `smart_search`    — natural-language answer over the knowledge vault.
//!
//! The submodules are:
//! - `provider`   — `CloudProvider` trait + `AnthropicCloud` HTTP client
//! - `redactor`   — entity scrubbing with reverse-map un-redaction
//! - `confidence` — heuristic 0..1 score derived from retrieval signals
//! - `actions`    — one file per action + a `CloudActions` facade

pub mod actions;
pub mod confidence;
pub mod provider;
pub mod redactor;

pub use actions::CloudActions;
pub use provider::{AnthropicCloud, CloudProvider};
pub use redactor::{Redactor, ReverseMap};

use serde::{Deserialize, Serialize};

/// Discriminator for which cloud action was run. Persisted into
/// `ai_drafts.action_type` and `action_log.action_type` verbatim (via
/// `as_str`), so values must stay stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudAction {
    SummarizeThread,
    DraftReply,
    SmartSearch,
}

impl CloudAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            CloudAction::SummarizeThread => "summarize_thread",
            CloudAction::DraftReply => "draft_reply",
            CloudAction::SmartSearch => "smart_search",
        }
    }
}

/// Per-call options. Kept minimal for Plan 5.
#[derive(Debug, Clone, Copy)]
pub struct CloudConfig {
    /// When true, the `Redactor` scrubs named entities, emails, and
    /// phone numbers before the body is sent to the cloud. The reverse
    /// map is always applied to the response before it is stored or
    /// returned, regardless of this flag.
    pub redact: bool,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self { redact: false }
    }
}
