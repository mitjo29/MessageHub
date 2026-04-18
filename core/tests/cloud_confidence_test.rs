use messagehub_core::ai::cloud::confidence::derive_confidence;
use messagehub_core::ai::RagContext;

fn ctx(sender_name: Option<&str>, profile: &str) -> RagContext {
    RagContext {
        sender_name: sender_name.map(|s| s.to_string()),
        sender_vault_path: sender_name.map(|_| "05-People/x.md".to_string()),
        topic_chunks: vec![],
        user_profile_content: profile.to_string(),
    }
}

#[test]
fn test_confidence_full_signal() {
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[0.9, 0.7]);
    assert!((score - 0.9).abs() < 1e-6);
}

#[test]
fn test_confidence_unknown_sender_drops_signal() {
    let score = derive_confidence(&ctx(None, "Role: x"), &[1.0]);
    assert!((score - 0.7).abs() < 1e-6);
}

#[test]
fn test_confidence_empty_profile_drops_signal() {
    let score = derive_confidence(&ctx(Some("Alice"), ""), &[1.0]);
    assert!((score - 0.8).abs() < 1e-6);
}

#[test]
fn test_confidence_whitespace_only_profile_treated_as_empty() {
    let score = derive_confidence(&ctx(Some("Alice"), "   \n   "), &[1.0]);
    assert!((score - 0.8).abs() < 1e-6);
}

#[test]
fn test_confidence_zero_when_nothing_matches() {
    let score = derive_confidence(&ctx(None, ""), &[]);
    assert_eq!(score, 0.0);
}

#[test]
fn test_confidence_is_clamped_to_0_1() {
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[1.5]);
    assert_eq!(score, 1.0);
}

#[test]
fn test_confidence_takes_max_of_retrieval_scores() {
    let score = derive_confidence(&ctx(Some("Alice"), "Role: x"), &[0.1, 0.8, 0.3]);
    assert!((score - 0.8).abs() < 1e-6);
}
