use std::sync::Arc;

use crate::ai::profile::UserProfile;
use crate::error::Result;
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::Store;
use crate::types::Channel;

/// Everything the classifier prompt needs about the world surrounding
/// an incoming message.
///
/// Contains three buckets: (1) who the sender is per the vault,
/// (2) vault chunks semantically near the message body, (3) the user's
/// standing profile. All three are optional — any combination can be
/// empty and the prompt will still be well-formed.
#[derive(Debug, Clone)]
pub struct RagContext {
    pub sender_name: Option<String>,
    pub sender_vault_path: Option<String>,
    pub topic_chunks: Vec<ContextChunk>,
    pub user_profile_content: String,
}

/// One vault chunk surfaced by semantic retrieval.
#[derive(Debug, Clone)]
pub struct ContextChunk {
    pub file_path: String,
    pub heading: Option<String>,
    pub content: String,
}

impl RagContext {
    /// Render this context as a markdown section suitable to paste into
    /// a user prompt. Uses headings and bullet points for clarity.
    pub fn to_prompt_section(&self) -> String {
        let mut out = String::new();

        // Sender section
        out.push_str("# Sender context (from vault)\n");
        match (&self.sender_name, &self.sender_vault_path) {
            (Some(name), Some(path)) => {
                out.push_str(&format!("- Known contact: {} (profile: {})\n", name, path));
            }
            _ => {
                out.push_str("- Unknown sender — no vault profile match.\n");
            }
        }
        out.push('\n');

        // Topic chunks
        out.push_str("# Relevant vault notes\n");
        if self.topic_chunks.is_empty() {
            out.push_str("- (no vault content matched this message)\n");
        } else {
            for chunk in &self.topic_chunks {
                let heading = chunk.heading.as_deref().unwrap_or("(no heading)");
                out.push_str(&format!(
                    "- [{} — {}] {}\n",
                    chunk.file_path,
                    heading,
                    chunk.content.trim()
                ));
            }
        }
        out.push('\n');

        // User profile
        out.push_str("# User profile\n");
        if self.user_profile_content.trim().is_empty() {
            out.push_str("- (no profile configured)\n");
        } else {
            out.push_str(&self.user_profile_content);
            if !self.user_profile_content.ends_with('\n') {
                out.push('\n');
            }
        }

        out
    }
}

/// Assemble a `RagContext` for an incoming message.
///
/// Parameters:
/// - `store` — live database handle
/// - `retriever` — optional vault retriever. If `None`, topic chunks are
///   skipped (useful for tests that don't want to load the embedder, and
///   for the degraded mode where the knowledge engine is disabled).
/// - `profile` — pre-loaded user profile (empty if not configured)
/// - `channel`, `sender_address` — used for sender lookup via `Store::find_vault_person_by_address`
/// - `subject`, `body` — combined into the retrieval query string
///
/// The retrieval filter is left `Default` so the top-k pulls from any
/// PARA folder. Callers that want folder-scoped retrieval (e.g. "only
/// business notes for work messages") can extend this signature later.
pub fn build_rag_context(
    store: &Store,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    channel: Channel,
    sender_address: &str,
    subject: &str,
    body: &str,
) -> Result<RagContext> {
    let (sender_name, sender_vault_path) =
        match store.find_vault_person_by_address(channel.to_db_str(), sender_address)? {
            Some((name, path)) => (Some(name), Some(path)),
            None => (None, None),
        };

    let topic_chunks = match retriever {
        Some(r) => {
            let query = build_retrieval_query(subject, body);
            let filters = RetrievalFilters {
                para_folders: None,
                top_k: Some(5),
            };
            r.search(store, &query, &filters)?
                .into_iter()
                .map(|rc| ContextChunk {
                    file_path: rc.file_path,
                    heading: rc.section_heading,
                    content: rc.content,
                })
                .collect()
        }
        None => Vec::new(),
    };

    Ok(RagContext {
        sender_name,
        sender_vault_path,
        topic_chunks,
        user_profile_content: profile.content.clone(),
    })
}

/// Combine subject and body into a single retrieval query.
/// Subject gets repeated twice so keyword-like phrasing has a stronger
/// signal than a single mention inside a long body.
fn build_retrieval_query(subject: &str, body: &str) -> String {
    let subject = subject.trim();
    let body_excerpt: String = body.trim().chars().take(500).collect();
    if subject.is_empty() {
        body_excerpt
    } else {
        format!("{}\n{}\n{}", subject, subject, body_excerpt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_retrieval_query_handles_empty_subject() {
        let q = build_retrieval_query("", "Hello world");
        assert_eq!(q, "Hello world");
    }

    #[test]
    fn test_build_retrieval_query_emphasizes_subject() {
        let q = build_retrieval_query("Urgent", "detail");
        let count = q.matches("Urgent").count();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_build_retrieval_query_truncates_long_bodies() {
        let long = "x".repeat(1_000);
        let q = build_retrieval_query("S", &long);
        // "S\nS\n" prefix (4 chars) + at most 500 body chars = 504
        assert!(q.chars().count() <= 504);
    }
}
