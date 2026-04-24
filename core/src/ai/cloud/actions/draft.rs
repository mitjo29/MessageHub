use std::sync::Arc;

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Strips any leftover redaction tokens (e.g. `[PERSON_1]`, `[EMAIL_2]`)
/// that the LLM produced but don't match any entry in the reverse map.
/// This is defense-in-depth: if the model hallucinates a token (or reuses
/// a token number that wasn't in the request), it should not reach the
/// user. NOTE: stripping without replacement leaves grammatically broken
/// output (e.g. "Hi , yes tomorrow works.") — in practice hallucinations
/// are rare enough that this is preferable to letting the token through.
pub(super) static LEFTOVER_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(PERSON|EMAIL|PHONE)_\d+\]").expect("leftover token regex must compile")
});

use crate::ai::cloud::actions::summarize::DraftOutcome;
use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::{un_redact, Redactor};
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::{build_rag_context, RagContext};
use crate::error::{CoreError, Result};
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::{NewDraft, Store};

pub(super) const SYSTEM_PROMPT: &str = r#"You are drafting a reply to an incoming message on behalf of the user. The user will review your draft before sending — do not invent commitments, prices, or decisions.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "draft":    <plain-text reply, no greeting signature unless the thread already uses one>,
  "language": <one of: "en", "fr", "de">
}

Match the language of the incoming message unless the user profile indicates a strong language preference, in which case follow the profile. Keep the draft concise: one or two paragraphs maximum.

Do not wrap the JSON in code fences unless absolutely necessary."#;

const ALLOWED_LANGUAGES: &[&str] = &["en", "fr", "de"];

#[derive(Debug, Deserialize)]
pub(super) struct ParsedDraft {
    pub(super) draft: String,
    pub(super) language: String,
}

/// Draft a reply to `message_id` via the cloud provider.
pub async fn draft_reply(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    message_id: Uuid,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let message = store.get_message(&message_id)?;
    let sender_addr = {
        let contact = store.get_contact(&message.sender_id)?;
        contact
            .identities
            .iter()
            .find(|id| id.channel == message.channel)
            .map(|id| id.address.clone())
            .unwrap_or_default()
    };

    let subject = message.content.subject.clone().unwrap_or_default();
    let body = message.content.text.clone().unwrap_or_default();

    let (redacted_body, reverse_map) = if cfg.redact {
        redactor.redact(&body)
    } else {
        (body.clone(), Default::default())
    };

    let retrieval_sims: Vec<f32> = match retriever {
        Some(r) => r
            .search(
                store,
                &format!("{} {}", subject, redacted_body),
                &RetrievalFilters::default(),
            )?
            .into_iter()
            .map(|c| similarity_from_distance(c.distance))
            .collect(),
        None => Vec::new(),
    };

    let rag = build_rag_context(
        store,
        retriever,
        profile,
        message.channel,
        &sender_addr,
        &subject,
        &redacted_body,
    )?;

    let user_prompt = build_user_prompt(
        &message.channel.to_string(),
        &sender_addr,
        &subject,
        &redacted_body,
        &rag,
    );

    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 1024)
        .await
        .map_err(|e| log_and_return(store, message_id, e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "draft_reply parse failed");
            let _ = store.log_ai_decision(
                "draft_reply_failed",
                "message",
                &message_id.to_string(),
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let un_redacted = un_redact(&parsed.draft, &reverse_map);
    let final_output = LEFTOVER_TOKEN_RE.replace_all(&un_redacted, "").to_string();
    let confidence = derive_confidence(&rag, &retrieval_sims);

    let draft_id = Uuid::new_v4();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: Some(message_id),
        action_type: CloudAction::DraftReply.as_str(),
        input_redacted: &redacted_body,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    store.log_ai_decision(
        CloudAction::DraftReply.as_str(),
        "message",
        &message_id.to_string(),
        &format!("language={}", parsed.language),
        confidence as f64,
    )?;

    info!(
        message_id = %message_id,
        confidence,
        language = %parsed.language,
        "draft_reply succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::DraftReply,
        output: final_output,
        confidence,
    })
}

pub(super) fn build_user_prompt(
    channel: &str,
    sender: &str,
    subject: &str,
    body: &str,
    rag: &RagContext,
) -> String {
    let mut out = String::new();
    out.push_str("# Incoming message\n");
    out.push_str(&format!("Channel: {}\n", channel));
    out.push_str(&format!("From: {}\n", sender));
    if !subject.trim().is_empty() {
        out.push_str(&format!("Subject: {}\n", subject));
    }
    out.push_str("\nBody:\n");
    out.push_str(body.trim());
    out.push_str("\n\n");
    out.push_str(&rag.to_prompt_section());
    out.push_str("\nDraft a reply to this message.\n");
    out
}

pub(super) fn parse_response(raw: &str) -> Result<ParsedDraft> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedDraft = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.draft.trim().is_empty() {
        return Err(CoreError::Cloud("empty draft field".into()));
    }
    if !ALLOWED_LANGUAGES.contains(&parsed.language.as_str()) {
        return Err(CoreError::Cloud(format!(
            "unknown language '{}'; must be one of {:?}",
            parsed.language, ALLOWED_LANGUAGES
        )));
    }
    Ok(parsed)
}

/// sqlite-vec L2 distance → 0..1 similarity. For 384-dim unit vectors
/// distances are roughly 0..2; this maps monotonically.
pub(super) fn similarity_from_distance(distance: f32) -> f32 {
    (1.0 - (distance / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// Log the failure to action_log and return the error unchanged.
/// Unlike naive wrapping, this does NOT re-wrap in CoreError::Cloud —
/// the error already has the right shape.
fn log_and_return(store: &Store, message_id: Uuid, err: CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "draft_reply_failed",
        "message",
        &message_id.to_string(),
        &format!("{}", err),
        0.0,
    );
    debug!(error = %err, "draft_reply cloud call failed");
    err
}

fn strip_code_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    trimmed.to_string()
}

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
