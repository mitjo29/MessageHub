// Integration test: the ingestor is idempotent per (channel, external_id).
// We don't run a full Runtime here — that requires networking. Instead we
// insert the same Message twice via Store::insert_message directly and
// assert only one row exists. This is the closest thing to "poll twice"
// that doesn't require a fake channel server.

use chrono::Utc;
use messagehub_core::store::{MessageFilter, Store};
use messagehub_core::types::{Channel, Message, MessageContent, Thread};
use std::collections::HashMap;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn make_thread(store: &Store, channel: Channel) -> Thread {
    let t = Thread {
        id: Uuid::new_v4(),
        channel,
        subject: Some("subj".into()),
        participant_ids: vec![],
        message_count: 0,
        last_message_at: Utc::now(),
        created_at: Utc::now(),
        external_thread_id: None,
    };
    store.insert_thread(&t).unwrap();
    t
}

fn make_message(
    channel: Channel,
    sender_id: Uuid,
    thread_id: Uuid,
    external_id: &str,
) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel,
        thread_id,
        sender_id,
        content: MessageContent {
            text: Some("hello".into()),
            html: None,
            subject: Some("subj".into()),
            attachments: vec![],
        },
        timestamp: Utc::now(),
        metadata: HashMap::new(),
        priority: None,
        category: None,
        is_read: false,
        is_archived: false,
        external_id: Some(external_id.into()),
    }
}

#[test]
fn ingest_same_external_id_twice_is_idempotent() {
    let store = test_store();
    let contact = store
        .find_or_create_contact_by_address(Channel::Email, "user@example.com", "Test User")
        .unwrap();
    let thread = make_thread(&store, Channel::Email);

    // Simulate two independent "polls" that both deliver the same
    // external_id. Each produces a fresh UUID (as normalize does).
    let poll1 = make_message(Channel::Email, contact.id, thread.id, "ext-dup-test-1");
    let poll2 = make_message(Channel::Email, contact.id, thread.id, "ext-dup-test-1");
    assert_ne!(poll1.id, poll2.id, "UUIDs must differ — dedup is on external_id");

    store.insert_message(&poll1).unwrap();
    store.insert_message(&poll2).unwrap();

    let all = store.list_messages(&MessageFilter::default(), 10, 0).unwrap();
    assert_eq!(
        all.len(),
        1,
        "dedup failed: {} rows for one external_id",
        all.len()
    );
    assert_eq!(all[0].id, poll1.id, "first-write-wins");
}

#[test]
fn different_channels_same_external_id_coexist() {
    let store = test_store();
    let c_email = store
        .find_or_create_contact_by_address(Channel::Email, "a@example.com", "A")
        .unwrap();
    let c_tg = store
        .find_or_create_contact_by_address(Channel::Telegram, "b", "B")
        .unwrap();
    let thread_email = make_thread(&store, Channel::Email);
    let thread_tg = make_thread(&store, Channel::Telegram);

    // Same external_id across different channels must NOT collide —
    // the unique index is on (channel_type, external_id).
    let email_msg = make_message(Channel::Email, c_email.id, thread_email.id, "42");
    let tg_msg = make_message(Channel::Telegram, c_tg.id, thread_tg.id, "42");

    store.insert_message(&email_msg).unwrap();
    store.insert_message(&tg_msg).unwrap();

    let all = store.list_messages(&MessageFilter::default(), 10, 0).unwrap();
    assert_eq!(all.len(), 2);
}
