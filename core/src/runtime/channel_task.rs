//! Per-channel polling task — drives one `ChannelAdapter` with exponential backoff.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::SeedableRng;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::adapters::ChannelAdapter;
use crate::runtime::events::{EventBus, RuntimeEvent};
use crate::runtime::ingestor::IngestJob;
use crate::runtime::status::{BackoffState, ChannelStatus};
use crate::store::Store;
use crate::types::ChannelConfig;

/// Handle for a running channel task. Drop or call `stop()` to cancel.
pub struct ChannelTaskHandle {
    pub config_id: Uuid,
    pub token: CancellationToken,
    pub join: JoinHandle<()>,
}

impl ChannelTaskHandle {
    pub fn stop(&self) {
        self.token.cancel();
    }
}

/// Spawns the polling loop for one channel.
pub fn spawn_channel_task(
    config: ChannelConfig,
    adapter: Box<dyn ChannelAdapter>,
    store: Arc<Mutex<Store>>,
    ingest_tx: mpsc::Sender<IngestJob>,
    bus: EventBus,
    parent_token: &CancellationToken,
) -> ChannelTaskHandle {
    let token = parent_token.child_token();
    let task_token = token.clone();
    let config_id = config.id;

    let join = tokio::spawn(run_channel_task(
        config,
        adapter,
        store,
        ingest_tx,
        bus,
        task_token,
    ));

    ChannelTaskHandle {
        config_id,
        token,
        join,
    }
}

async fn run_channel_task(
    config: ChannelConfig,
    mut adapter: Box<dyn ChannelAdapter>,
    store: Arc<Mutex<Store>>,
    ingest_tx: mpsc::Sender<IngestJob>,
    bus: EventBus,
    token: CancellationToken,
) {
    let channel_id = config.id;
    let mut backoff = BackoffState {
        consecutive_failures: config.consecutive_failures,
    };
    let mut last_status = backoff.status(config.last_error.as_deref());
    let mut last_sync_at = config.last_sync_at;
    let mut rng = rand::rngs::StdRng::from_entropy();

    info!(%channel_id, label = %config.label, "channel task: starting");

    // Restore the adapter's last cursor (Telegram last_update_id, etc.).
    // Fresh channels have last_sync_cursor = None; the adapter default
    // (e.g. TelegramAdapter::last_update_id = None) then kicks in and
    // getUpdates starts from the bot's oldest queued update.
    if let Err(e) = adapter
        .set_cursor_state(config.last_sync_cursor.clone())
        .await
    {
        warn!(%channel_id, error = %e, "channel task: cursor hydration failed; continuing with adapter default");
    }

    loop {
        let delay_secs = backoff.next_delay_secs(config.poll_interval_secs, &mut rng);

        tokio::select! {
            biased;
            _ = token.cancelled() => { break; }
            _ = tokio::time::sleep(Duration::from_secs(delay_secs)) => {}
        }

        // fetch_messages is an async call — must NOT hold store lock across it.
        match adapter.fetch_messages(last_sync_at).await {
            Ok(batch) if batch.is_empty() => {
                backoff.record_success();
                publish_status_if_changed(
                    &store,
                    &bus,
                    channel_id,
                    &backoff,
                    None,
                    &mut last_status,
                );
            }
            Ok(batch) => {
                let count = batch.len();
                let latest_ts = batch.iter().map(|m| m.timestamp).max();

                // ingest_tx.send is an .await — cannot hold store lock across it.
                let job = IngestJob { channel_id, batch };
                if let Err(e) = ingest_tx.send(job).await {
                    error!(%channel_id, error = %e, "channel task: ingestor channel closed");
                    break;
                }

                // Capture the adapter's cursor BEFORE we grab the store
                // lock — cursor_state is async and mutex must not be held
                // across .await.
                let cursor_after = adapter.cursor_state().await;

                // Persist cursor + timestamp under a brief lock — no await.
                if let Some(ts) = latest_ts {
                    {
                        let guard = store.lock().expect("channel_task: store mutex poisoned");
                        if let Err(e) = guard.update_sync_state(&channel_id, cursor_after.as_deref(), ts) {
                            warn!(%channel_id, error = %e, "channel task: failed to persist cursor");
                        }
                    } // lock released here
                    last_sync_at = Some(ts);
                }

                backoff.record_success();
                bus.publish(RuntimeEvent::SyncSucceeded { channel_id, count });
                publish_status_if_changed(
                    &store,
                    &bus,
                    channel_id,
                    &backoff,
                    None,
                    &mut last_status,
                );
            }
            Err(e) => {
                backoff.record_failure();
                let err_str = e.to_string();
                let attempt = backoff.consecutive_failures;
                bus.publish(RuntimeEvent::SyncFailed {
                    channel_id,
                    error: err_str.clone(),
                    attempt,
                });
                publish_status_if_changed(
                    &store,
                    &bus,
                    channel_id,
                    &backoff,
                    Some(&err_str),
                    &mut last_status,
                );
            }
        }
    }

    info!(%channel_id, "channel task: disconnecting");
    if let Err(e) = adapter.disconnect().await {
        warn!(%channel_id, error = %e, "channel task: disconnect error");
    }
}

/// Derive the new status from `backoff`; if it differs from `last_status`,
/// persist it (under a brief lock, no await), publish `ChannelStatusChanged`,
/// and update `*last_status`.
fn publish_status_if_changed(
    store: &Mutex<Store>,
    bus: &EventBus,
    channel_id: Uuid,
    backoff: &BackoffState,
    last_error: Option<&str>,
    last_status: &mut ChannelStatus,
) {
    let new = backoff.status(last_error);
    if new != *last_status {
        // Brief synchronous lock — no await in this scope.
        {
            let guard = store.lock().expect("channel_task: store mutex poisoned");
            if let Err(e) =
                guard.update_channel_status(&channel_id, &new, backoff.consecutive_failures)
            {
                warn!(%channel_id, error = %e, "channel task: failed to persist status");
            }
        } // lock released here
        bus.publish(RuntimeEvent::ChannelStatusChanged {
            channel_id,
            status: new.clone(),
        });
        *last_status = new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock::MockAdapter;
    use crate::adapters::RawMessage;
    use crate::runtime::events::EventBus;
    use crate::runtime::ingestor::spawn_ingestor;
    use crate::store::Store;
    use crate::types::Channel;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn seed_config() -> ChannelConfig {
        ChannelConfig {
            id: Uuid::new_v4(),
            channel: Channel::Telegram,
            label: "test".to_string(),
            keychain_ref: "none".to_string(),
            enabled: true,
            poll_interval_secs: 1,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn successful_fetch_publishes_sync_succeeded_and_forwards_to_ingestor() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let config = seed_config();

        // Insert the channel config so update_sync_state can find the row.
        {
            let guard = store.lock().unwrap();
            guard.insert_channel_config(&config).unwrap();
        }

        let bus = EventBus::with_capacity(32);
        let mut events = bus.subscribe();

        let (ingest_tx, ingest_handle) = spawn_ingestor(
            Arc::clone(&store),
            bus.clone(),
            None,
            8,
            CancellationToken::new(),
        );

        let adapter = MockAdapter::new();
        adapter.add_message(RawMessage {
            external_id: "msg-1".to_string(),
            channel: Channel::Telegram,
            external_thread_id: Some("chat-1".to_string()),
            sender_name: "Bot".to_string(),
            sender_address: "bot".to_string(),
            text: Some("Hi".to_string()),
            html: None,
            subject: None,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });

        let root = CancellationToken::new();
        let handle = spawn_channel_task(
            config.clone(),
            Box::new(adapter),
            Arc::clone(&store),
            ingest_tx.clone(),
            bus,
            &root,
        );

        // Advance past one poll interval (1s base + up to 20% jitter → max 1.2s).
        tokio::time::advance(Duration::from_millis(1500)).await;

        // Give the spawned tasks a chance to run.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Collect up to 8 events with a small timeout each.
        let mut saw_sync_succeeded = false;
        let mut saw_ingested = false;
        for _ in 0..8 {
            let evt =
                tokio::time::timeout(Duration::from_millis(200), events.recv()).await;
            if let Ok(Ok(ev)) = evt {
                match ev {
                    RuntimeEvent::SyncSucceeded { .. } => saw_sync_succeeded = true,
                    RuntimeEvent::MessageIngested { .. } => saw_ingested = true,
                    _ => {}
                }
            }
        }

        assert!(saw_sync_succeeded, "expected SyncSucceeded event");
        assert!(saw_ingested, "expected MessageIngested event");

        root.cancel();
        handle.join.await.unwrap();
        drop(ingest_tx);
        ingest_handle.await.unwrap();
    }
}
