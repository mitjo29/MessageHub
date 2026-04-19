use tokio::sync::broadcast;
use tracing::trace;
use uuid::Uuid;

use crate::runtime::status::ChannelStatus;
use crate::types::PriorityScore;

/// Events published by the runtime. Consumers subscribe via `Runtime::subscribe`.
#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    MessageIngested      { id: Uuid, channel_id: Uuid },
    MessageClassified    { id: Uuid, category: Option<String>, priority: Option<PriorityScore> },
    SyncSucceeded        { channel_id: Uuid, count: usize },
    SyncFailed           { channel_id: Uuid, error: String, attempt: u32 },
    ChannelStatusChanged { channel_id: Uuid, status: ChannelStatus },
}

/// Thin wrapper around `broadcast::Sender` that silently drops send errors.
///
/// `broadcast::Sender::send` returns `Err` iff there are no receivers. The
/// runtime must not care — nobody subscribed yet is a valid state.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RuntimeEvent>,
}

impl EventBus {
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }

    /// Publish an event. Never blocks, never errors.
    pub fn publish(&self, ev: RuntimeEvent) {
        if let Err(err) = self.tx.send(ev) {
            trace!(error = %err, "no runtime event subscribers");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_and_receive_roundtrip() {
        let bus = EventBus::with_capacity(4);
        let mut rx = bus.subscribe();
        let id = Uuid::new_v4();
        let ch = Uuid::new_v4();
        bus.publish(RuntimeEvent::MessageIngested { id, channel_id: ch });
        bus.publish(RuntimeEvent::MessageClassified {
            id,
            category: Some("Work".to_string()),
            priority: Some(PriorityScore::new(4).unwrap()),
        });
        match rx.recv().await.unwrap() {
            RuntimeEvent::MessageIngested { id: got, .. } => assert_eq!(got, id),
            other => panic!("unexpected event: {:?}", other),
        }
        match rx.recv().await.unwrap() {
            RuntimeEvent::MessageClassified { id: got, priority, .. } => {
                assert_eq!(got, id);
                assert_eq!(priority, Some(PriorityScore::new(4).unwrap()));
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn publish_without_subscribers_is_noop() {
        let bus = EventBus::with_capacity(4);
        bus.publish(RuntimeEvent::SyncSucceeded {
            channel_id: Uuid::new_v4(),
            count: 3,
        });
        // No panic, no error — success condition.
    }
}
