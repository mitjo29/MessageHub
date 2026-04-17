use messagehub_core::ai::prompts::{
    CLASSIFICATION_SYSTEM_PROMPT, build_classification_user_prompt, parse_classification_response,
};
use messagehub_core::ai::{Category, RagContext};
use messagehub_core::types::Channel;

#[test]
fn test_system_prompt_enumerates_categories() {
    for cat in Category::all_strs() {
        assert!(
            CLASSIFICATION_SYSTEM_PROMPT.contains(cat),
            "system prompt missing category literal '{}'",
            cat
        );
    }
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("priority"));
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("category"));
    assert!(CLASSIFICATION_SYSTEM_PROMPT.contains("reasoning"));
}

#[test]
fn test_build_user_prompt_includes_message_fields() {
    let ctx = RagContext {
        sender_name: Some("Alice".to_string()),
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: "Role: freelancer".to_string(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Email,
        "Alice Example",
        "alice@example.com",
        "Project X update",
        "Hey, are we still on for tomorrow?",
        &ctx,
    );
    assert!(prompt.contains("Email"));
    assert!(prompt.contains("Alice Example"));
    assert!(prompt.contains("alice@example.com"));
    assert!(prompt.contains("Project X update"));
    assert!(prompt.contains("still on for tomorrow"));
    assert!(prompt.contains("freelancer"));
    assert!(prompt.contains("Alice"));
}

#[test]
fn test_build_user_prompt_handles_none_subject() {
    let ctx = RagContext {
        sender_name: None,
        sender_vault_path: None,
        topic_chunks: vec![],
        user_profile_content: String::new(),
    };
    let prompt = build_classification_user_prompt(
        Channel::Telegram,
        "Bob",
        "@bob",
        "", // no subject for IM channels
        "Ping",
        &ctx,
    );
    assert!(prompt.contains("Telegram"));
    assert!(prompt.contains("Bob"));
    assert!(prompt.contains("Ping"));
}

#[test]
fn test_parse_response_valid_json() {
    let raw = r#"{"priority": 4, "category": "family", "reasoning": "Sender is daughter."}"#;
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 4);
    assert_eq!(parsed.category, Category::Family);
    assert_eq!(parsed.reasoning, "Sender is daughter.");
}

#[test]
fn test_parse_response_tolerates_markdown_code_fence() {
    let raw = "```json\n{\"priority\": 2, \"category\": \"newsletters\", \"reasoning\": \"Bulk promo.\"}\n```";
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 2);
    assert_eq!(parsed.category, Category::Newsletters);
}

#[test]
fn test_parse_response_tolerates_leading_explanation_text() {
    let raw = r#"Here is my classification:
{"priority": 5, "category": "work", "reasoning": "Deadline today"}"#;
    let parsed = parse_classification_response(raw).unwrap();
    assert_eq!(parsed.priority.value(), 5);
    assert_eq!(parsed.category, Category::Work);
}

#[test]
fn test_parse_rejects_priority_out_of_range() {
    let raw = r#"{"priority": 10, "category": "work", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());

    let raw = r#"{"priority": 0, "category": "work", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_unknown_category() {
    let raw = r#"{"priority": 3, "category": "important", "reasoning": "x"}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_missing_fields() {
    let raw = r#"{"priority": 3}"#;
    assert!(parse_classification_response(raw).is_err());
}

#[test]
fn test_parse_rejects_non_json() {
    let raw = "The message is important, priority 4, work category.";
    assert!(parse_classification_response(raw).is_err());
}
