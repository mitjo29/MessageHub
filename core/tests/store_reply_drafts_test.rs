use chrono::Utc;
use messagehub_core::store::{NewReplyDraft, ReplyDraft, Store};
use uuid::Uuid;

fn fresh_store() -> Store {
    Store::open_in_memory().expect("open_in_memory")
}

#[test]
fn upsert_then_get_roundtrip() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    let msg = Uuid::new_v4();

    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg,
            body: "hello world",
            subject: Some("Re: ping"),
        })
        .expect("upsert ok");

    let got: ReplyDraft = store
        .get_reply_draft(&thread)
        .expect("get ok")
        .expect("row exists");
    assert_eq!(got.thread_id, thread);
    assert_eq!(got.in_reply_to_message_id, msg);
    assert_eq!(got.body, "hello world");
    assert_eq!(got.subject.as_deref(), Some("Re: ping"));
    // updated_at is set by the DB default.
    assert!(got.updated_at <= Utc::now());
}

#[test]
fn second_upsert_overwrites_body_and_reply_target() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    let msg1 = Uuid::new_v4();
    let msg2 = Uuid::new_v4();

    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg1,
            body: "v1",
            subject: None,
        })
        .unwrap();
    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg2,
            body: "v2",
            subject: Some("Re: foo"),
        })
        .unwrap();

    let got = store.get_reply_draft(&thread).unwrap().unwrap();
    assert_eq!(got.in_reply_to_message_id, msg2);
    assert_eq!(got.body, "v2");
    assert_eq!(got.subject.as_deref(), Some("Re: foo"));
}

#[test]
fn get_unknown_thread_returns_none() {
    let store = fresh_store();
    assert!(store.get_reply_draft(&Uuid::new_v4()).unwrap().is_none());
}

#[test]
fn delete_is_idempotent() {
    let store = fresh_store();
    let thread = Uuid::new_v4();
    // Delete when absent.
    store.delete_reply_draft(&thread).unwrap();
    // Insert then delete.
    store
        .upsert_reply_draft(&NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: Uuid::new_v4(),
            body: "hi",
            subject: None,
        })
        .unwrap();
    store.delete_reply_draft(&thread).unwrap();
    assert!(store.get_reply_draft(&thread).unwrap().is_none());
    // Second delete — still Ok.
    store.delete_reply_draft(&thread).unwrap();
}
