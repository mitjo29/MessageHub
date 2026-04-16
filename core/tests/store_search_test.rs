use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn seed_messages(store: &Store) {
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Test Sender".into(),
        identities: vec![ContactIdentity { channel: Channel::Email, address: "test@example.com".into() }],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let thread = Thread {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        subject: None,
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
    };
    store.insert_thread(&thread).unwrap();

    let messages = vec![
        ("Contract review for Q2 delivery", "Please review the attached contract for the helicopter parts delivery"),
        ("Sprint planning meeting", "Let's discuss the next sprint goals and assign tasks"),
        ("Invoice #2024-089", "Attached is the invoice for consulting services rendered in March"),
        ("Dinner reservation confirmed", "Your table for 4 is confirmed at Restaurant Le Jardin for Saturday"),
    ];

    for (subject, body) in messages {
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Email,
            thread_id: thread.id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some(body.into()),
                html: None,
                subject: Some(subject.into()),
                attachments: vec![],
            },
            timestamp: Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
        };
        store.insert_message(&msg).unwrap();
    }
}

#[test]
fn test_search_by_subject() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("contract", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.subject.as_ref().unwrap().contains("Contract"));
}

#[test]
fn test_search_by_body() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("helicopter", 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_search_multiple_results() {
    let store = test_store();
    seed_messages(&store);

    // FTS5 boolean syntax (OR) is intentionally disabled to prevent injection.
    // Verify that individual terms each return results.
    let review_results = store.search_messages("review", 10).unwrap();
    assert!(!review_results.is_empty());
    let invoice_results = store.search_messages("invoice", 10).unwrap();
    assert!(!invoice_results.is_empty());
}

#[test]
fn test_search_no_results() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("blockchain", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_respects_limit() {
    let store = test_store();
    seed_messages(&store);

    let results = store.search_messages("the", 2).unwrap();
    assert!(results.len() <= 2);
}
