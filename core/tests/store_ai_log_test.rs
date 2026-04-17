use messagehub_core::store::{AiDecision, Store};
use uuid::Uuid;

#[test]
fn test_log_and_retrieve_ai_decision() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    store
        .log_ai_decision(
            "classify",
            "message",
            &message_id.to_string(),
            "Sender is daughter; school topic -> family/high priority",
            0.87,
        )
        .unwrap();

    let decisions = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(decisions.len(), 1);
    let d: &AiDecision = &decisions[0];
    assert_eq!(d.action_type, "classify");
    assert_eq!(d.entity_type, Some("message".to_string()));
    assert!(d.reasoning.as_deref().unwrap().contains("daughter"));
    assert!((d.confidence_score.unwrap() - 0.87).abs() < 1e-6);
}

#[test]
fn test_multiple_decisions_per_entity_returned_in_insertion_order() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    store
        .log_ai_decision("classify", "message", &message_id.to_string(), "first", 0.5)
        .unwrap();
    store
        .log_ai_decision("reprioritize", "message", &message_id.to_string(), "second", 0.9)
        .unwrap();

    let decisions = store
        .list_ai_decisions_for_entity("message", &message_id.to_string())
        .unwrap();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].action_type, "classify");
    assert_eq!(decisions[1].action_type, "reprioritize");
}

#[test]
fn test_list_returns_empty_for_unknown_entity() {
    let store = Store::open_in_memory().unwrap();
    let decisions = store
        .list_ai_decisions_for_entity("message", "nonexistent")
        .unwrap();
    assert!(decisions.is_empty());
}
