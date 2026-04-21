use chrono::Utc;
use messagehub_core::store::MessageFilter;
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
        external_thread_id: None,
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
fn test_list_messages_default_filter_returns_all() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..3 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let filter = MessageFilter::default();
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 3);
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

    let filter = MessageFilter {
        channel: Some(Channel::Email),
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 3);

    let filter_sms = MessageFilter {
        channel: Some(Channel::Sms),
        ..Default::default()
    };
    let empty = store.list_messages(&filter_sms, 10, 0).unwrap();
    assert_eq!(empty.len(), 0);
}

#[test]
fn test_list_messages_unread_only() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    // Two unread, one read.
    let m1 = make_message(contact.id, thread.id);
    let m2 = make_message(contact.id, thread.id);
    let m3 = make_message(contact.id, thread.id);
    store.insert_message(&m1).unwrap();
    store.insert_message(&m2).unwrap();
    store.insert_message(&m3).unwrap();
    store.mark_read(&m2.id, true).unwrap();

    let filter = MessageFilter {
        unread_only: true,
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|m| !m.is_read));
}

#[test]
fn test_list_messages_min_priority() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    // priority is 3 by default via make_message; add one at 5 and one at 1.
    let mut low = make_message(contact.id, thread.id);
    low.priority = PriorityScore::new(1);
    let mut high = make_message(contact.id, thread.id);
    high.priority = PriorityScore::new(5);
    let mid = make_message(contact.id, thread.id); // priority=3

    store.insert_message(&low).unwrap();
    store.insert_message(&mid).unwrap();
    store.insert_message(&high).unwrap();

    let filter = MessageFilter {
        min_priority: Some(4),
        ..Default::default()
    };
    let messages = store.list_messages(&filter, 10, 0).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].priority.unwrap().value(), 5);
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

#[test]
fn test_count_messages_default_matches_list_len() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    for _ in 0..5 {
        store
            .insert_message(&make_message(contact.id, thread.id))
            .unwrap();
    }

    let filter = MessageFilter::default();
    let count = store.count_messages(&filter).unwrap();
    let list_len = store.list_messages(&filter, 100, 0).unwrap().len() as u64;
    assert_eq!(count, list_len);
    assert_eq!(count, 5);
}

#[test]
fn test_count_messages_unread_only() {
    let store = test_store();
    let contact = make_contact(&store);
    let thread = make_thread(&store);

    let m1 = make_message(contact.id, thread.id);
    let m2 = make_message(contact.id, thread.id);
    store.insert_message(&m1).unwrap();
    store.insert_message(&m2).unwrap();
    store.mark_read(&m1.id, true).unwrap();

    let filter = MessageFilter {
        unread_only: true,
        ..Default::default()
    };
    assert_eq!(store.count_messages(&filter).unwrap(), 1);
}
