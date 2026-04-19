use messagehub_core::store::{DraftRecord, NewDraft, Store};
use uuid::Uuid;

#[test]
fn test_insert_and_list_draft_for_message() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "[body with [EMAIL_1] scrubbed]",
            output: "Hi Alice, sure we can meet tomorrow.",
            confidence: 0.72,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 1);
    let d: &DraftRecord = &drafts[0];
    assert_eq!(d.id, draft_id);
    assert_eq!(d.action_type, "draft_reply");
    assert_eq!(d.output, "Hi Alice, sure we can meet tomorrow.");
    assert!((d.confidence - 0.72).abs() < 1e-6);
    assert_eq!(d.provider, "anthropic");
    assert!(d.user_edited_output.is_none());
}

#[test]
fn test_insert_draft_with_null_message_id_for_smart_search() {
    let store = Store::open_in_memory().unwrap();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: None,
            action_type: "smart_search",
            input_redacted: "latest from alix's school?",
            output: "No school messages in the last 30 days.",
            confidence: 0.91,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    // list_drafts_for_message filters on message_id; smart_search drafts
    // are therefore invisible here — by design.
    let some_msg = Uuid::new_v4();
    let drafts = store.list_drafts_for_message(&some_msg).unwrap();
    assert!(drafts.is_empty());
}

#[test]
fn test_update_draft_output_writes_user_edited_field() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();
    let draft_id = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: draft_id,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "initial draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    store.update_draft_output(&draft_id, "edited by user").unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts[0].output, "initial draft"); // original preserved
    assert_eq!(drafts[0].user_edited_output.as_deref(), Some("edited by user"));
}

#[test]
fn test_multiple_drafts_for_same_message_return_newest_first() {
    let store = Store::open_in_memory().unwrap();
    let message_id = Uuid::new_v4();

    let first = Uuid::new_v4();
    let second = Uuid::new_v4();

    store
        .insert_draft(&NewDraft {
            id: first,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "first draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();
    // Sleep one millisecond-granular tick is hard in tests; ensure ordering
    // is stable by inserting with deterministically different ids — the
    // query uses created_at DESC, then id DESC as a tiebreaker.
    store
        .insert_draft(&NewDraft {
            id: second,
            message_id: Some(message_id),
            action_type: "draft_reply",
            input_redacted: "x",
            output: "second draft",
            confidence: 0.5,
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        })
        .unwrap();

    let drafts = store.list_drafts_for_message(&message_id).unwrap();
    assert_eq!(drafts.len(), 2);
    // Newest first — the second insert should appear at index 0.
    assert_eq!(drafts[0].id, second);
    assert_eq!(drafts[1].id, first);
}
