use std::sync::Arc;

use serde::Deserialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::ai::cloud::actions::summarize::DraftOutcome;
use crate::ai::cloud::confidence::derive_confidence;
use crate::ai::cloud::provider::CloudProvider;
use crate::ai::cloud::redactor::{un_redact, Redactor};
use crate::ai::cloud::{CloudAction, CloudConfig};
use crate::ai::profile::UserProfile;
use crate::ai::rag::RagContext;
use crate::error::{CoreError, Result};
use crate::knowledge::{RetrievalFilters, Retriever};
use crate::store::{NewDraft, Store};

const SYSTEM_PROMPT: &str = r#"You answer natural-language questions over the user's personal knowledge vault. You will receive the user's query and up to 10 retrieved chunks from their vault.

You must respond with a single JSON object and nothing else, matching this schema:

{
  "answer":  <plain-text answer, 1-3 short paragraphs>,
  "sources": <array of vault file paths cited, e.g. ["01-Projects/Project X.md"]>
}

Only cite paths that appear in the provided chunks. If the chunks don't contain an answer, say so in the answer field and return an empty sources array.

Do not wrap the JSON in code fences unless absolutely necessary."#;

#[derive(Debug, Deserialize)]
struct ParsedAnswer {
    answer: String,
    #[allow(dead_code)]
    sources: Vec<String>,
}

pub async fn smart_search(
    store: &Store,
    provider: Arc<dyn CloudProvider>,
    redactor: &Redactor,
    retriever: Option<&Arc<Retriever>>,
    profile: &UserProfile,
    query: &str,
    cfg: CloudConfig,
    model: &str,
) -> Result<DraftOutcome> {
    let (redacted_query, reverse_map) = if cfg.redact {
        redactor.redact(query)
    } else {
        (query.to_string(), Default::default())
    };

    let (chunks, sims) = match retriever {
        Some(r) => {
            let results = r.search(
                store,
                &redacted_query,
                &RetrievalFilters {
                    para_folders: None,
                    top_k: Some(10),
                },
            )?;
            let sims: Vec<f32> = results
                .iter()
                .map(|c| (1.0 - (c.distance / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0))
                .collect();
            (results, sims)
        }
        None => (Vec::new(), Vec::new()),
    };

    // Build a minimal RagContext so derive_confidence has consistent inputs.
    let rag = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: profile.content.clone(),
    };

    let user_prompt = build_user_prompt(&redacted_query, &chunks, profile);
    let raw = provider
        .complete(SYSTEM_PROMPT, &user_prompt, 1024)
        .await
        .map_err(|e| log_and_return(store, query, e))?;

    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "smart_search parse failed");
            let _ = store.log_ai_decision(
                "smart_search_failed",
                "query",
                query,
                &format!("{}", e),
                0.0,
            );
            return Err(e);
        }
    };

    let final_output = un_redact(&parsed.answer, &reverse_map);
    let confidence = derive_confidence(&rag, &sims);

    let draft_id = Uuid::new_v4();
    store.insert_draft(&NewDraft {
        id: draft_id,
        message_id: None, // smart_search has no anchor message
        action_type: CloudAction::SmartSearch.as_str(),
        input_redacted: &redacted_query,
        output: &final_output,
        confidence,
        provider: "anthropic",
        model,
    })?;
    // Audit key: entity_type=query, entity_id=original (un-redacted) query.
    store.log_ai_decision(
        CloudAction::SmartSearch.as_str(),
        "query",
        query,
        &final_output,
        confidence as f64,
    )?;

    info!(
        query_preview = %query.chars().take(80).collect::<String>(),
        confidence,
        "smart_search succeeded"
    );

    Ok(DraftOutcome {
        id: draft_id,
        action: CloudAction::SmartSearch,
        output: final_output,
        confidence,
    })
}

fn build_user_prompt(
    redacted_query: &str,
    chunks: &[crate::knowledge::RetrievedChunk],
    profile: &UserProfile,
) -> String {
    let mut out = String::new();
    out.push_str("# User query\n");
    out.push_str(redacted_query);
    out.push_str("\n\n# Retrieved vault chunks\n");
    if chunks.is_empty() {
        out.push_str("- (no retriever configured — answer from profile + general knowledge only)\n");
    } else {
        for c in chunks {
            let heading = c.section_heading.as_deref().unwrap_or("(no heading)");
            out.push_str(&format!(
                "- [{} — {}] {}\n",
                c.file_path,
                heading,
                c.content.trim().chars().take(400).collect::<String>()
            ));
        }
    }
    out.push_str("\n# User profile\n");
    if profile.content.trim().is_empty() {
        out.push_str("- (no profile configured)\n");
    } else {
        out.push_str(&profile.content);
        if !profile.content.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("\nAnswer the query using only the retrieved chunks.\n");
    out
}

fn parse_response(raw: &str) -> Result<ParsedAnswer> {
    let stripped = strip_code_fences(raw);
    let json_slice = first_balanced_object(&stripped).ok_or_else(|| {
        CoreError::Cloud(format!("no JSON object in cloud response: {:?}", raw))
    })?;
    let parsed: ParsedAnswer = serde_json::from_str(json_slice)
        .map_err(|e| CoreError::Cloud(format!("cloud response schema mismatch: {}", e)))?;
    if parsed.answer.trim().is_empty() {
        return Err(CoreError::Cloud("empty answer field".into()));
    }
    Ok(parsed)
}

fn log_and_return(store: &Store, query: &str, err: CoreError) -> CoreError {
    let _ = store.log_ai_decision(
        "smart_search_failed",
        "query",
        query,
        &format!("{}", err),
        0.0,
    );
    debug!(error = %err, "smart_search cloud call failed");
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
