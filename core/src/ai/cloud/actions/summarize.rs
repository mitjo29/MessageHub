use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::{un_redact, Redactor};
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::{build_rag_context, RagContext};
use crate::error::{CoreError, Result};
use crate::store::{NewDraft, Store};
use crate::types::Message;

const SYSTEM_PROMPT: &str = r#"You are a conversation summarizer running with user consent over a single thread.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "summary": <1-3 sentence plain-text summary of the thread>,
  "language": <one of: "en", "fr", "de">
}

Summarize what the thread is actually about, what was decided, and any open question. Do not add commentary or apologies. Do not wrap the JSON in code fences unless absolutely necessary."#;

/// Public-facing record returned by every cloud action.
#[derive(Debug, Clone)]
pub struct DraftOutcome {
    pub id: Uuid,
    pub action: CloudAction,
    pub output: String,
    pub confidence: f32,
}

#[derive(Debug, Deserialize)]
struct ParsedSummary {
    summary: String,
    #[allow(dead_code)]
    language: String,
}

/// Summarize every message in `thread_id` via the cloud provider.
///
/// On success: persists to `ai_drafts` (anchored to the newest message in
/// the thread) and writes a `summarize_thread` row to `action_log`. Returns
/// the un-redacted summary ready for display.
///
/// On cloud or parse failure: writes a `summarize_thread_failed` row to
/// `action_log` and returns `CoreError::Cloud(...)` to the caller.
pub async fn summarize_thread(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    profile: &UserProfile,
    thread_id: Uuid,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let messages = store.list_messages_in_thread(&thread_id, 200)?;
    if messages.is_empty() {
        return Err(CoreError::Cloud(format!(
            "cannot summarize empty thread {}",
            thread_id
        )));
    }
    let anchor_message = messages.last().cloned().unwrap();

    let thread_text = render_thread_as_text(&messages);
    let (redacted_thread, reverse_map) = if cfg.redact {
        redactor.redact(&thread_text)
    } else {
        (thread_text.clone(), Default::default())
    };

    let last_sender_addr = resolve_sender_address(store, &anchor_message);
    let rag = build_rag_context(
        store,
        None, // no retriever needed — the thread IS the grounding
        profile,
        anchor_message.channel,
        last_sender_addr.as_deref().unwrap_or(""),
        anchor_message.content.subject.as_deref().unwrap_or(""),
        &redacted_thread,
    )?;

    let user_prompt = build_user_prompt(&redacted_thread, &rag);
    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 512)
        .await
        .map_err(|e| log_and_wrap(store, thread_id, &e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, raw_preview = %raw.chars().take(200).collect::<String>(),
                  "summarize_thread parse failed");
            let _ = store.log_ai_decision(
                "summarize_thread_failed",
                "thread",
                &thread_id.to_string(),
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let final_output = un_redact(&parsed.summary, &reverse_map);
    let confidence = derive_confidence(&rag, &[0.85]);

    let draft_id = Uuid::new_v4();
    let preview: String = redacted_thread.chars().take(2_000).collect();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: Some(anchor_message.id),
        action_type: CloudAction::SummarizeThread.as_str(),
        input_redacted: &preview,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    store.log_ai_decision(
        CloudAction::SummarizeThread.as_str(),
        "thread",
        &thread_id.to_string(),
        &final_output,
        confidence as f64,
    )?;

    info!(
        thread_id = %thread_id,
        confidence,
        timestamp = %Utc::now(),
        "summarize_thread succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::SummarizeThread,
        output: final_output,
        confidence,
    })
}

fn render_thread_as_text(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        out.push_str(&format!("[{}] ", m.timestamp.to_rfc3339()));
        if let Some(s) = &m.content.subject {
            out.push_str(&format!("(subject: {}) ", s));
        }
        if let Some(t) = &m.content.text {
            out.push_str(t.trim());
        }
        out.push('\n');
    }
    out
}

fn build_user_prompt(thread_text: &str, rag: &RagContext) -> String {
    let mut out = String::new();
    out.push_str("# Conversation to summarize\n\n");
    out.push_str(thread_text.trim());
    out.push_str("\n\n");
    out.push_str(&rag.to_prompt_section());
    out.push_str("\nSummarize this conversation.\n");
    out
}

fn parse_response(raw: &str) -> Result<ParsedSummary> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedSummary = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.summary.trim().is_empty() {
        return Err(CoreError::Cloud("empty summary field".into()));
    }
    Ok(parsed)
}

/// Sender address lookup: messages only store `sender_id`, so walk to
/// `contacts` for the first identity matching the message's channel.
fn resolve_sender_address(store: &Store, msg: &Message) -> Option<String> {
    let contact = store.get_contact(&msg.sender_id).ok()?;
    contact
        .identities
        .into_iter()
        .find(|id| id.channel == msg.channel)
        .map(|id| id.address)
}

fn log_and_wrap(store: &Store, thread_id: Uuid, err: &CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "summarize_thread_failed",
        "thread",
        &thread_id.to_string(),
        &format!("{}", err),
        0.0,
    );
    debug!(error = %err, "summarize_thread cloud call failed");
    CoreError::Cloud(format!("{}", err))
}

/// Strip triple-backtick fences (with optional `json` language tag).
fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

/// Return the first balanced `{...}` block.
fn first_balanced_object(s: &str) -> Option<&str> {
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
