//! ClassifierWorker — drains a Uuid mpsc queue, calls `AiPipeline::classify_stored`,
//! and emits `MessageClassified` events on the `EventBus`.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::ai::pipeline::AiPipeline;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::store::Store;

/// Spawn the classifier worker task if an `AiPipeline` is present.
///
/// Returns `(Some(sender), Some(handle))` when spawned; `(None, None)` when
/// `ai_pipeline` is `None` (classification disabled).
///
/// The worker:
/// 1. Receives message `Uuid`s from the returned sender.
/// 2. Calls `pipeline.classify_stored(&store, &id)` for each, which manages
///    the `Mutex<Store>` lock discipline internally (brief lock for reads,
///    async LLM call, brief lock for writes).
/// 3. Reloads the message under a brief lock to read `category` and `priority`.
/// 4. Publishes `MessageClassified { id, category, priority }` on the bus.
/// 5. On shutdown, drains remaining queued ids before exiting.
pub fn maybe_spawn_classifier(
    store: Arc<Mutex<Store>>,
    ai_pipeline: Option<Arc<AiPipeline>>,
    bus: EventBus,
    queue_capacity: usize,
    shutdown: CancellationToken,
) -> (Option<mpsc::Sender<Uuid>>, Option<JoinHandle<()>>) {
    let pipeline = match ai_pipeline {
        Some(p) => p,
        None => return (None, None),
    };

    let (tx, mut rx) = mpsc::channel::<Uuid>(queue_capacity);

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    info!("classifier_worker: shutdown signalled, draining queue");
                    while let Ok(id) = rx.try_recv() {
                        classify_one(&store, &pipeline, &bus, id).await;
                    }
                    break;
                }
                maybe_id = rx.recv() => {
                    match maybe_id {
                        Some(id) => classify_one(&store, &pipeline, &bus, id).await,
                        None => {
                            info!("classifier_worker: channel closed, exiting");
                            break;
                        }
                    }
                }
            }
        }
    });

    (Some(tx), Some(handle))
}

async fn classify_one(
    store: &Mutex<Store>,
    pipeline: &AiPipeline,
    bus: &EventBus,
    id: Uuid,
) {
    match pipeline.classify_stored(store, &id).await {
        Ok(outcome) => {
            debug!(message_id = %id, classified = outcome.classified, "classifier_worker: done");

            // Reload the message briefly to read the persisted category + priority.
            let (category, priority) = {
                match store.lock().expect("store mutex poisoned").get_message(&id) {
                    Ok(msg) => (msg.category, msg.priority),
                    Err(e) => {
                        warn!(message_id = %id, error = %e,
                              "classifier_worker: could not reload message for event");
                        (None, None)
                    }
                }
            };

            bus.publish(RuntimeEvent::MessageClassified { id, category, priority });
        }
        Err(e) => {
            error!(message_id = %id, error = %e,
                   "classifier_worker: classify_stored returned store error");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::LlmBackend;
    use crate::ai::pipeline::AiPipeline;
    use crate::ai::profile::UserProfile;
    use crate::runtime::events::RuntimeEvent;
    use crate::store::Store;
    use crate::types::{Channel, Message, MessageContent, PriorityScore, Thread};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct StubLlm;
    #[async_trait]
    impl LlmBackend for StubLlm {
        async fn complete(
            &self,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> crate::error::Result<String> {
            Ok(r#"{"category":"Work","priority":4,"reasoning":"test"}"#.to_string())
        }
    }

    /// Seed a minimal message into the store, return its id.
    fn seed_message(store: &Store) -> Uuid {
        let contact = store
            .find_or_create_contact_by_address(Channel::Telegram, "sender@test", "Sender")
            .unwrap();
        let thread_id = Uuid::new_v4();
        store
            .insert_thread(&Thread {
                id: thread_id,
                channel: Channel::Telegram,
                subject: None,
                participant_ids: vec![],
                message_count: 0,
                last_message_at: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                external_thread_id: None,
            })
            .unwrap();
        let msg = Message {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            thread_id,
            sender_id: contact.id,
            content: MessageContent {
                text: Some("Hello worker".to_string()),
                html: None,
                subject: Some("Test subject".to_string()),
                attachments: vec![],
                reply_headers: None,
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
            external_id: None,
        };
        let id = msg.id;
        store.insert_message(&msg).unwrap();
        id
    }

    #[tokio::test]
    async fn worker_processes_queued_id_and_emits_event() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let msg_id = {
            let guard = store.lock().unwrap();
            seed_message(&*guard)
        };

        let pipeline = Arc::new(AiPipeline::new(
            Arc::new(StubLlm),
            None,
            UserProfile { content: String::new() },
        ));

        let bus = EventBus::with_capacity(16);
        let mut rx = bus.subscribe();
        let shutdown = CancellationToken::new();

        let (tx, handle) = maybe_spawn_classifier(
            Arc::clone(&store),
            Some(pipeline),
            bus,
            8,
            shutdown.clone(),
        );
        let tx = tx.expect("sender should be Some when pipeline is provided");
        let handle = handle.expect("handle should be Some when pipeline is provided");

        tx.send(msg_id).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        // Collect the MessageClassified event.
        let evt = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            rx.recv(),
        )
        .await
        .expect("timeout waiting for event")
        .expect("broadcast channel closed");

        match evt {
            RuntimeEvent::MessageClassified { id, category, priority } => {
                assert_eq!(id, msg_id);
                assert!(category.is_some(), "category should be populated");
                assert!(priority.is_some(), "priority should be populated");
                assert_eq!(priority, Some(PriorityScore::new(4).unwrap()));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn worker_not_spawned_when_pipeline_absent() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let bus = EventBus::with_capacity(4);
        let shutdown = CancellationToken::new();

        let (tx, handle) = maybe_spawn_classifier(store, None, bus, 8, shutdown);

        assert!(tx.is_none(), "tx should be None when no pipeline");
        assert!(handle.is_none(), "handle should be None when no pipeline");
    }
}
