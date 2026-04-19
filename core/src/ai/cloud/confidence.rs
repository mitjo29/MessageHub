use crate::ai::RagContext;

/// Derive a heuristic 0.0-1.0 confidence score from the grounding
/// signals available at the call site.
///
/// - `top_sim`: the best cosine similarity from the retriever. For
///   `summarize_thread`, where there is no retrieval, callers pass
///   `&[0.85]` as a baseline — the grounding is the thread itself.
/// - `sender_signal`: 1.0 if the sender is a vault-known contact, 0.7
///   otherwise. Strangers can still ground on message content, so we
///   don't zero it out.
/// - `profile_signal`: 1.0 if `user-profile.md` has content, 0.8 if not.
///
/// The product is clamped to 0.0..=1.0 as a belt-and-braces guard; with
/// well-behaved inputs the components never exceed 1.0 each.
///
/// Property: unknown sender + empty profile + zero retrieval = 0.0.
pub fn derive_confidence(rag: &RagContext, retrieval_sims: &[f32]) -> f32 {
    let top_sim = retrieval_sims
        .iter()
        .cloned()
        .fold(0.0_f32, f32::max);
    let sender_signal = if rag.sender_name.is_some() { 1.0 } else { 0.7 };
    let profile_signal = if !rag.user_profile_content.trim().is_empty() {
        1.0
    } else {
        0.8
    };
    (top_sim * sender_signal * profile_signal).clamp(0.0, 1.0)
}
