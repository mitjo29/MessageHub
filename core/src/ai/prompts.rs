use serde::Deserialize;

use crate::ai::classifier::Classification;
use crate::ai::{Category, RagContext};
use crate::error::{CoreError, Result};
use crate::types::{Channel, PriorityScore};

/// System prompt for the one-shot classification call.
///
/// The model MUST emit a single JSON object. The system prompt enumerates
/// every legal category so the model has no freedom to invent values.
/// We also ask for reasoning so the UI can show "Why is this prioritized?"
pub const CLASSIFICATION_SYSTEM_PROMPT: &str = r#"You are an inbox classification assistant running locally on the user's device. Your job is to prioritize and categorize incoming messages.

You must respond with a single JSON object and nothing else. The schema is strict:

{
  "priority": <integer 1 to 5>,
  "category": <one of: "work", "personal", "finance", "family", "notifications", "newsletters", "spam">,
  "reasoning": <one short sentence explaining your choice>
}

Priority scale:
- 5 = urgent, needs action today (personal emergencies, hard deadlines, direct asks from family/key clients)
- 4 = important, action this week (replies needed from known contacts, meeting confirmations)
- 3 = normal (regular conversation, FYI)
- 2 = low (newsletters you subscribed to, non-urgent notifications)
- 1 = spam or irrelevant (promotional, unknown bulk senders, noise)

Use the "Sender context" section to identify who the sender is relative to the user's vault. Known contacts from the user's vault (especially family and work relationships) should generally score higher.

Use the "Relevant vault notes" section to understand project and topic context.

Use the "User profile" section to understand the user's languages, role, and life areas.

Do not include any text outside the JSON object. Do not wrap the JSON in code fences unless you absolutely cannot avoid it.
"#;

/// Build the user-turn prompt.
///
/// Layout:
/// - Incoming message metadata (channel, sender, subject, body)
/// - RAG context rendered via `RagContext::to_prompt_section`
/// - Final "Classify this message." instruction
pub fn build_classification_user_prompt(
    channel: Channel,
    sender_name: &str,
    sender_address: &str,
    subject: &str,
    body: &str,
    rag: &RagContext,
) -> String {
    let mut out = String::new();
    out.push_str("# Incoming message\n");
    out.push_str(&format!("Channel: {}\n", channel));
    out.push_str(&format!("From: {} <{}>\n", sender_name, sender_address));
    if !subject.trim().is_empty() {
        out.push_str(&format!("Subject: {}\n", subject));
    }
    out.push_str("\nBody:\n");
    // Truncate very long bodies to keep the prompt in budget.
    let body_excerpt: String = body.chars().take(2_000).collect();
    out.push_str(body_excerpt.trim());
    out.push_str("\n\n");

    out.push_str(&rag.to_prompt_section());
    out.push_str("\nClassify this message.\n");
    out
}

/// Parse the LLM's raw response into a `Classification`.
///
/// Accepts:
/// - Pure JSON
/// - JSON wrapped in triple-backtick code fences
/// - JSON preceded by a leading explanation (we extract the first balanced {...} block)
///
/// Rejects:
/// - Priority outside 1..=5
/// - Category not in `Category::all_strs`
/// - Missing `priority`, `category`, or `reasoning` fields
/// - No JSON object at all
pub fn parse_classification_response(raw: &str) -> Result<Classification> {
    let stripped = strip_fences(raw);
    let json_slice = extract_first_json_object(&stripped)
        .ok_or_else(|| CoreError::Ai(format!("no JSON object found in response: {:?}", raw)))?;

    let parsed: RawResponse = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Ai(format!("response JSON does not match schema: {}", e)))?;

    let priority = PriorityScore::new(parsed.priority).ok_or_else(|| {
        CoreError::Ai(format!(
            "priority {} out of range 1..=5",
            parsed.priority
        ))
    })?;
    let category = Category::from_str(&parsed.category).ok_or_else(|| {
        CoreError::Ai(format!(
            "unknown category '{}'; must be one of {:?}",
            parsed.category,
            Category::all_strs()
        ))
    })?;
    if parsed.reasoning.trim().is_empty() {
        return Err(CoreError::Ai("empty reasoning field".to_string()));
    }

    Ok(Classification {
        priority,
        category,
        reasoning: parsed.reasoning,
    })
}

#[derive(Deserialize)]
struct RawResponse {
    priority: u8,
    category: String,
    reasoning: String,
}

/// Strip triple-backtick fences (with or without a `json` language tag).
fn strip_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    let fence = "```";
    if let Some(rest) = trimmed.strip_prefix(fence) {
        // Drop optional language tag on first line.
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix(fence).unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

/// Find the first `{...}` block with balanced braces.
///
/// Naive but reliable for LLM outputs that don't contain strings with
/// unescaped braces. If the JSON contains a `}` inside a string, this
/// could mismatch — but `serde_json::from_str` would then fail with a
/// parse error, which surfaces as `CoreError::Ai` upstream.
fn extract_first_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_fences_noop_when_absent() {
        assert_eq!(strip_fences("{\"a\": 1}"), "{\"a\": 1}");
    }

    #[test]
    fn test_strip_fences_handles_json_label() {
        let input = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_fences(input), "{\"a\": 1}");
    }

    #[test]
    fn test_extract_first_json_object_finds_balanced_block() {
        let input = "prefix {\"a\": {\"b\": 1}} suffix";
        assert_eq!(
            extract_first_json_object(input).unwrap(),
            "{\"a\": {\"b\": 1}}"
        );
    }

    #[test]
    fn test_extract_first_json_object_returns_none_when_no_object() {
        assert_eq!(extract_first_json_object("no braces here"), None);
    }
}
