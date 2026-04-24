use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::adapters::{normalize, RawMessage};
use crate::error::Result;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::store::Store;
use crate::types::Thread;

/// A batch of raw messages fetched from a single channel, awaiting ingestion.
#[derive(Debug)]
pub struct IngestJob {
    pub channel_id: Uuid,
    pub batch: Vec<RawMessage>,
}

/// Spawns the ingestor task. Returns the sender used by channel tasks
/// to enqueue jobs, and the JoinHandle.
///
/// `store` is wrapped in `Arc<Mutex<Store>>` because `rusqlite::Connection`
/// implements `Send` but not `Sync`; the `Mutex` makes the arc `Send + Sync`
/// so the future can cross thread boundaries inside `tokio::spawn`.
pub fn spawn_ingestor(
    store: Arc<Mutex<Store>>,
    bus: EventBus,
    classifier_tx: Option<mpsc::Sender<Uuid>>,
    queue_capacity: usize,
    shutdown: CancellationToken,
) -> (mpsc::Sender<IngestJob>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<IngestJob>(queue_capacity);

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    info!("ingestor: shutdown signalled, draining queue");
                    while let Ok(job) = rx.try_recv() {
                        process_job(&store, &bus, classifier_tx.as_ref(), job).await;
                    }
                    break;
                }
                maybe_job = rx.recv() => {
                    match maybe_job {
                        Some(job) => process_job(&store, &bus, classifier_tx.as_ref(), job).await,
                        None => { info!("ingestor: channel closed, exiting"); break; }
                    }
                }
            }
        }
    });

    (tx, handle)
}

async fn process_job(
    store: &Mutex<Store>,
    bus: &EventBus,
    classifier_tx: Option<&mpsc::Sender<Uuid>>,
    job: IngestJob,
) {
    let IngestJob { channel_id, batch } = job;
    for raw in batch {
        match ingest_one(store, &raw, channel_id) {
            Ok(message_id) => {
                bus.publish(RuntimeEvent::MessageIngested {
                    id: message_id,
                    channel_id,
                });
                if let Some(tx) = classifier_tx {
                    if let Err(e) = tx.send(message_id).await {
                        warn!(error = %e, "classifier queue closed; skipping classify");
                    }
                }
            }
            Err(e) => {
                error!(
                    external_id = %raw.external_id,
                    channel = %raw.channel,
                    error = %e,
                    "ingestor: dropped message after store error"
                );
            }
        }
    }
}

/// Resolve contact + thread, normalize, insert. Returns the new message id.
fn ingest_one(store: &Mutex<Store>, raw: &RawMessage, channel_id: Uuid) -> Result<Uuid> {
    let store = store.lock().expect("store mutex poisoned");

    let contact = store.find_or_create_contact_by_address(
        raw.channel,
        &raw.sender_address,
        &raw.sender_name,
    )?;

    let thread = resolve_thread(&store, raw)?;

    // Clone because `normalize` takes the RawMessage by value and the caller
    // expected to keep it.
    let mut message = normalize(raw.clone(), contact.id, thread.id);
    // B-004: route replies through the same channel the message arrived on.
    message.received_on_channel_id = Some(channel_id);
    store.insert_message(&message)?;
    debug!(id = %message.id, "ingestor: stored message");
    Ok(message.id)
}

fn resolve_thread(store: &Store, raw: &RawMessage) -> Result<Thread> {
    if let Some(ext) = raw.external_thread_id.as_deref() {
        if let Some(t) = store.find_thread_by_external_id(raw.channel, ext)? {
            return Ok(t);
        }
    }

    // Synthesize a new thread. External id may still be None (some channels
    // don't provide one); that is fine — future messages with the same
    // external id will match, and messages with no external id each get
    // their own thread. Thread grouping by subject/participants is a later
    // concern (not in Plan 6 scope).
    let thread = Thread {
        id: Uuid::new_v4(),
        channel: raw.channel,
        subject: raw.subject.clone(),
        participant_ids: vec![],
        message_count: 0,
        last_message_at: raw.timestamp,
        created_at: raw.timestamp,
        external_thread_id: raw.external_thread_id.clone(),
    };
    store.insert_thread(&thread)?;
    Ok(thread)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::RawMessage;
    use crate::store::Store;
    use crate::runtime::events::EventBus;
    use crate::runtime::status::ChannelStatus;
    use crate::types::{Channel, ChannelConfig};
    use std::collections::HashMap;

    /// Insert a Telegram channel row so the FK on
    /// `messages.received_on_channel_id` is satisfied.
    fn insert_channel(store: &Mutex<Store>, id: Uuid) {
        store.lock().unwrap().insert_channel_config(&ChannelConfig {
            id,
            channel: Channel::Telegram,
            label: "test".into(),
            keychain_ref: "u:p".into(),
            enabled: true,
            poll_interval_secs: 60,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        }).unwrap();
    }

    fn raw(ext_thread: Option<&str>, sender: &str) -> RawMessage {
        RawMessage {
            external_id: uuid::Uuid::new_v4().to_string(),
            channel: Channel::Telegram,
            external_thread_id: ext_thread.map(String::from),
            sender_name: sender.to_string(),
            sender_address: sender.to_string(),
            text: Some("hi".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn ingest_inserts_contact_thread_message() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let channel_id = Uuid::new_v4();
        insert_channel(&store, channel_id);
        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let (tx, handle) = spawn_ingestor(
            Arc::clone(&store),
            bus.clone(),
            None, // no classifier
            8,
            CancellationToken::new(),
        );

        tx.send(IngestJob {
            channel_id,
            batch: vec![raw(Some("chat-1"), "alice")],
        }).await.unwrap();

        // First event must be MessageIngested.
        let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await.unwrap().unwrap();
        let ingested_id = match evt {
            RuntimeEvent::MessageIngested { id, channel_id: got_ch } => {
                assert_eq!(got_ch, channel_id);
                id
            }
            other => panic!("unexpected event: {:?}", other),
        };

        drop(tx);
        handle.await.unwrap();

        // B-004: stored message must remember which channel it arrived on.
        let store = store.lock().unwrap();
        let stored = store.get_message(&ingested_id).unwrap();
        assert_eq!(stored.received_on_channel_id, Some(channel_id));
    }

    #[tokio::test]
    async fn same_external_thread_reuses_existing_thread() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let channel_id = Uuid::new_v4();
        insert_channel(&store, channel_id);
        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let (tx, handle) = spawn_ingestor(
            Arc::clone(&store), bus, None, 8, CancellationToken::new(),
        );

        tx.send(IngestJob {
            channel_id,
            batch: vec![
                raw(Some("chat-same"), "alice"),
                raw(Some("chat-same"), "bob"),
            ],
        }).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        // Collect the two MessageIngested events, look up each message, and
        // assert they share a thread_id.
        let mut ids: Vec<Uuid> = Vec::new();
        for _ in 0..2 {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await.unwrap().unwrap();
            if let RuntimeEvent::MessageIngested { id, .. } = evt { ids.push(id); }
        }
        assert_eq!(ids.len(), 2);
        let store = store.lock().unwrap();
        let m1 = store.get_message(&ids[0]).unwrap();
        let m2 = store.get_message(&ids[1]).unwrap();
        assert_eq!(m1.thread_id, m2.thread_id,
                   "both messages should land in the same thread");
    }
}
