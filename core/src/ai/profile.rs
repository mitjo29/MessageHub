use std::path::Path;

use tracing::{debug, warn};

use crate::error::Result;

/// The user's self-authored profile, loaded from a vault markdown file
/// (typically `02-Areas/User/user-profile.md`).
///
/// The content is injected into every classification prompt so the LLM
/// has standing context about the user's languages, role, and life areas.
/// The file is truncated to `MAX_PROFILE_CHARS` to keep the prompt under
/// a reasonable token budget.
pub struct UserProfile {
    pub content: String,
}

/// Character budget for the profile content injected into prompts.
/// At ~4 chars/token this is ~1000 tokens — comfortable for a 3B model's
/// context window while leaving room for the message and retrieved chunks.
const MAX_PROFILE_CHARS: usize = 4_000;

impl UserProfile {
    /// Load the profile from a markdown file on disk.
    ///
    /// If the file doesn't exist, returns an empty profile rather than
    /// an error — the pipeline degrades gracefully when there's no
    /// user-authored profile yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let content = if raw.chars().count() > MAX_PROFILE_CHARS {
                    let truncated: String = raw.chars().take(MAX_PROFILE_CHARS - 12).collect();
                    warn!(
                        path = %path.display(),
                        original_chars = raw.chars().count(),
                        "user profile truncated to fit prompt budget"
                    );
                    format!("{}[truncated]", truncated)
                } else {
                    raw
                };
                debug!(path = %path.display(), chars = content.len(), "user profile loaded");
                Ok(Self { content })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "user profile file not found; using empty profile");
                Ok(Self {
                    content: String::new(),
                })
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to read user profile");
                Ok(Self {
                    content: String::new(),
                })
            }
        }
    }

    /// True when the profile has any non-whitespace content.
    pub fn has_content(&self) -> bool {
        !self.content.trim().is_empty()
    }
}
