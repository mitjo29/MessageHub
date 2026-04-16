use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn make_contact(store: &Store) -> Contact {
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Test User".into(),
        identities: vec![ContactIdentity {
            channel: Channel::Email,
            address: "test@example.com".into(),
        }],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();
    contact
}

fn make_thread(store: &Store) -> Thread {
    let thread = Thread {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        subject: Some("Test thread".into()),
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
    };
    store.insert_thread(&thread).unwrap();
    thread
}

fn make_message(sender_id: Uuid, thread_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id,
        sender_id,
        content: MessageContent {
            text: Some("Hello, this is a test message about contracts".into()),
            html: None,
            subject: Some("Contract Review".into()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: PriorityScore::new(3),
        category: Some("work".into()),
        is_read: false,
        is_archived: false,
    }
}

#[test]
fn test_insert_and_get_message() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);

    store.insert_message(&msg).unwrap();

    let retrieved = store.get_message(&msg.id).unwrap();
    assert_eq!(retrieved.id, msg.id);
    assert_eq!(retrieved.content.subject.as_deref(), Some("Contract Review"));
    assert_eq!(retrieved.is_read, false);
}

#[test]
fn test_list_messages_by_channel() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..3 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let messages = store
        .list_messages(Some(Channel::Email), false, 10, 0)
        .unwrap();
    assert_eq!(messages.len(), 3);
}

#[test]
fn test_mark_message_read() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);
    store.insert_message(&msg).unwrap();

    store.mark_read(&msg.id, true).unwrap();

    let retrieved = store.get_message(&msg.id).unwrap();
    assert!(retrieved.is_read);
}

#[test]
fn test_search_messages_fts() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);
    let msg = make_message(contact.id, thread.id);
    store.insert_message(&msg).unwrap();

    let results = store.search_messages("contracts", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, msg.id);

    let no_results = store.search_messages("nonexistent", 10).unwrap();
    assert!(no_results.is_empty());
}
