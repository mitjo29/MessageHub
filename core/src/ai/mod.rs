//! Local AI classification pipeline (Tier 1).
//!
//! This module provides always-on, fully-offline priority scoring and
//! category tagging for incoming messages using a local LLM via Ollama.
//!
//! Architecture:
//! - `llm` — `LlmBackend` trait + `OllamaLlm` HTTP client
//! - `profile` — user profile loader (`user-profile.md`)
//! - `rag` — per-message RAG context builder
//! - `prompts` — classification prompt template + JSON response parser
//! - `classifier` — ties the above into a single `classify()` call
//! - `pipeline` — orchestrator: classify → store → log

pub mod classifier;
pub mod llm;
pub mod pipeline;
pub mod profile;
pub mod prompts;
pub mod rag;

pub use classifier::{Classification, Classifier};
pub use llm::{LlmBackend, OllamaLlm};
pub use pipeline::AiPipeline;
pub use profile::UserProfile;
pub use prompts::{build_classification_user_prompt, parse_classification_response, CLASSIFICATION_SYSTEM_PROMPT};
pub use rag::RagContext;

use serde::{Deserialize, Serialize};

/// High-level category derived from the user's PARA vault structure.
///
/// These are the only values the classifier is allowed to emit.
/// The parser in `prompts::parse_classification_response` rejects
/// anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Work,
    Personal,
    Finance,
    Family,
    Notifications,
    Newsletters,
    Spam,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Work => "work",
            Category::Personal => "personal",
            Category::Finance => "finance",
            Category::Family => "family",
            Category::Notifications => "notifications",
            Category::Newsletters => "newsletters",
            Category::Spam => "spam",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "work" => Some(Category::Work),
            "personal" => Some(Category::Personal),
            "finance" => Some(Category::Finance),
            "family" => Some(Category::Family),
            "notifications" => Some(Category::Notifications),
            "newsletters" => Some(Category::Newsletters),
            "spam" => Some(Category::Spam),
            _ => None,
        }
    }

    /// All valid category strings, used for prompt template injection.
    pub fn all_strs() -> &'static [&'static str] {
        &[
            "work",
            "personal",
            "finance",
            "family",
            "notifications",
            "newsletters",
            "spam",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::Category;

    #[test]
    fn test_category_roundtrip() {
        for s in Category::all_strs() {
            let cat = Category::from_str(s).unwrap();
            assert_eq!(cat.as_str(), *s);
        }
    }

    #[test]
    fn test_category_case_insensitive() {
        assert_eq!(Category::from_str("WORK").unwrap(), Category::Work);
        assert_eq!(Category::from_str("  Spam  ").unwrap(), Category::Spam);
    }

    #[test]
    fn test_category_rejects_unknown() {
        assert!(Category::from_str("important").is_none());
        assert!(Category::from_str("").is_none());
    }
}
