use chrono::{TimeZone, Utc};
use messagehub_core::store::Store;
use messagehub_core::types::{
    Channel, Contact, ContactIdentity, Message, MessageContent, Thread,
};
use std::collections::HashMap;
use uuid::Uuid;

fn seed_contact_and_thread(store: &Store) -> (Uuid, Uuid) {
    let contact_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    store
        .insert_contact(&Contact {
            id: contact_id,
            display_name: "Alice".into(),
            identities: vec![ContactIdentity {
                channel: Channel::Email,
                address: "alice@example.com".into(),
            }],
            vault_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .unwrap();
    store
        .insert_thread(&Thread {
            id: thread_id,
            channel: Channel::Email,
            subject: Some("Project X".into()),
            participant_ids: vec![contact_id],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: None,
        })
        .unwrap();
    (contact_id, thread_id)
}

fn msg(sender: Uuid, thread: Uuid, text: &str, epoch_secs: i64) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel: Channel::Email,
        thread_id: thread,
        sender_id: sender,
        content: MessageContent {
            text: Some(text.into()),
            html: None,
            subject: Some("Project X".into()),
            attachments: vec![],
        },
        timestamp: Utc.timestamp_opt(epoch_secs, 0).unwrap(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
        external_id: None,
    }
}

#[test]
fn test_list_messages_in_thread_returns_oldest_first() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread) = seed_contact_and_thread(&store);

    let later = msg(sender, thread, "second", 2000);
    let earlier = msg(sender, thread, "first", 1000);
    store.insert_message(&later).unwrap();
    store.insert_message(&earlier).unwrap();

    let got = store.list_messages_in_thread(&thread, 10).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].content.text.as_deref(), Some("first"));
    assert_eq!(got[1].content.text.as_deref(), Some("second"));
}

#[test]
fn test_list_messages_in_thread_respects_limit() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread) = seed_contact_and_thread(&store);

    for i in 0..5 {
        store
            .insert_message(&msg(sender, thread, &format!("m{}", i), 1000 + i as i64))
            .unwrap();
    }

    let got = store.list_messages_in_thread(&thread, 3).unwrap();
    assert_eq!(got.len(), 3);
    // Oldest-first, so we keep m0..m2 — NOT the last 3.
    assert_eq!(got[0].content.text.as_deref(), Some("m0"));
    assert_eq!(got[2].content.text.as_deref(), Some("m2"));
}

#[test]
fn test_list_messages_in_thread_returns_empty_for_unknown_thread() {
    let store = Store::open_in_memory().unwrap();
    let unknown = Uuid::new_v4();
    let got = store.list_messages_in_thread(&unknown, 10).unwrap();
    assert!(got.is_empty());
}

#[test]
fn test_list_messages_in_thread_ignores_other_threads() {
    let store = Store::open_in_memory().unwrap();
    let (sender, thread_a) = seed_contact_and_thread(&store);
    let thread_b = Uuid::new_v4();
    store
        .insert_thread(&Thread {
            id: thread_b,
            channel: Channel::Email,
            subject: Some("Unrelated".into()),
            participant_ids: vec![sender],
            message_count: 0,
            last_message_at: Utc::now(),
            created_at: Utc::now(),
            external_thread_id: None,
        })
        .unwrap();

    store.insert_message(&msg(sender, thread_a, "in A", 1000)).unwrap();
    store.insert_message(&msg(sender, thread_b, "in B", 1001)).unwrap();

    let got = store.list_messages_in_thread(&thread_a, 10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].content.text.as_deref(), Some("in A"));
}
