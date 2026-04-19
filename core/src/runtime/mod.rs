//! Runtime: orchestration layer that drives adapters → ingestion → classification.
//!
//! See `docs/superpowers/specs/2026-04-19-plan6-channel-runtime-design.md`.

pub mod status;
pub mod events;
pub mod factory;
pub mod ingestor;
pub mod classifier_worker;
pub mod channel_task;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::ai::pipeline::AiPipeline;
use crate::error::{CoreError, Result};
use crate::store::Store;

use self::channel_task::{spawn_channel_task, ChannelTaskHandle};
use self::classifier_worker::maybe_spawn_classifier;
use self::events::{EventBus, RuntimeEvent};
use self::factory::{AdapterFactory, FactoryRegistry};
use self::ingestor::{spawn_ingestor, IngestJob};

/// Default broadcast buffer size for runtime events.
const DEFAULT_EVENT_BUFFER: usize = 1024;
/// Default bounded mpsc capacity for the classifier worker queue.
const DEFAULT_CLASSIFIER_QUEUE: usize = 256;
/// Default wall-clock timeout for each join during graceful shutdown.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// RuntimeBuilder
// ---------------------------------------------------------------------------

pub struct RuntimeBuilder {
    store: Arc<Mutex<Store>>,
    pipeline: Option<Arc<AiPipeline>>,
    registry: FactoryRegistry,
    event_buffer: usize,
    classifier_queue: usize,
    shutdown_timeout: Duration,
}

impl RuntimeBuilder {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        Self {
            store,
            pipeline: None,
            registry: FactoryRegistry::new(),
            event_buffer: DEFAULT_EVENT_BUFFER,
            classifier_queue: DEFAULT_CLASSIFIER_QUEUE,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }

    pub fn with_ai_pipeline(mut self, p: Arc<AiPipeline>) -> Self {
        self.pipeline = Some(p);
        self
    }

    pub fn with_factory(
        mut self,
        channel_type: impl Into<String>,
        factory: Arc<dyn AdapterFactory>,
    ) -> Self {
        self.registry.register(channel_type, factory);
        self
    }

    pub fn event_buffer(mut self, n: usize) -> Self {
        self.event_buffer = n;
        self
    }

    pub fn classifier_queue(mut self, n: usize) -> Self {
        self.classifier_queue = n;
        self
    }

    pub fn shutdown_timeout(mut self, d: Duration) -> Self {
        self.shutdown_timeout = d;
        self
    }

    pub fn build(self) -> Runtime {
        let bus = EventBus::with_capacity(self.event_buffer);
        Runtime {
            store: self.store,
            pipeline: self.pipeline,
            registry: self.registry,
            bus,
            classifier_queue: self.classifier_queue,
            // ingestor capacity scales with registered channels; rebuilt in start().
            ingest_capacity_base: 16,
            shutdown_timeout: self.shutdown_timeout,
            running: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

pub struct Runtime {
    store: Arc<Mutex<Store>>,
    pipeline: Option<Arc<AiPipeline>>,
    registry: FactoryRegistry,
    bus: EventBus,
    classifier_queue: usize,
    ingest_capacity_base: usize,
    shutdown_timeout: Duration,
    running: Option<RunningState>,
}

// ---------------------------------------------------------------------------
// RunningState (private)
// ---------------------------------------------------------------------------

struct RunningState {
    root: CancellationToken,
    ingest_tx: mpsc::Sender<IngestJob>,
    ingest_handle: JoinHandle<()>,
    classifier_handle: Option<JoinHandle<()>>,
    channel_tasks: HashMap<Uuid, ChannelTaskHandle>,
}

// ---------------------------------------------------------------------------
// Runtime impl
// ---------------------------------------------------------------------------

impl Runtime {
    /// Convenience factory — delegates to `RuntimeBuilder::new`.
    pub fn builder(store: Arc<Mutex<Store>>) -> RuntimeBuilder {
        RuntimeBuilder::new(store)
    }

    /// Subscribe to the runtime event bus.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.bus.subscribe()
    }

    /// Spawn ingestor + optional classifier + one task per enabled channel row.
    ///
    /// Returns `Err` if the runtime is already running.
    pub async fn start(&mut self) -> Result<()> {
        if self.running.is_some() {
            return Err(CoreError::InvalidInput("runtime already running".to_string()));
        }

        let root = CancellationToken::new();

        let (classifier_tx, classifier_handle) = maybe_spawn_classifier(
            Arc::clone(&self.store),
            self.pipeline.clone(),
            self.bus.clone(),
            self.classifier_queue,
            root.clone(),
        );

        let ingest_cap = self.ingest_capacity_base.max(16);
        let (ingest_tx, ingest_handle) = spawn_ingestor(
            Arc::clone(&self.store),
            self.bus.clone(),
            classifier_tx,
            ingest_cap,
            root.clone(),
        );

        self.running = Some(RunningState {
            root,
            ingest_tx,
            ingest_handle,
            classifier_handle,
            channel_tasks: HashMap::new(),
        });

        self.reload_channels().await?;
        info!("runtime started");
        Ok(())
    }

    /// (Re)read the `channels` table and reconcile with running tasks.
    ///
    /// - Tasks whose row is missing or disabled are cancelled and joined.
    /// - Enabled rows that have no running task get a fresh adapter + task.
    /// - Missing factory → warning + skip (no error).
    ///
    /// Lock discipline: the store mutex is held only for the brief synchronous
    /// `list_channel_configs()` call; all `.await`s happen with no lock held.
    pub async fn reload_channels(&mut self) -> Result<()> {
        // Brief lock: collect rows then release immediately.
        let rows = {
            let guard = self.store.lock().expect("store mutex poisoned");
            guard.list_channel_configs()?
        };

        let running = self.running.as_mut().ok_or_else(|| {
            CoreError::InvalidInput("runtime not started".to_string())
        })?;

        let enabled_ids: std::collections::HashSet<Uuid> =
            rows.iter().filter(|r| r.enabled).map(|r| r.id).collect();

        // Stop tasks whose row is missing or disabled.
        let to_stop: Vec<Uuid> = running
            .channel_tasks
            .keys()
            .copied()
            .filter(|id| !enabled_ids.contains(id))
            .collect();

        for id in to_stop {
            if let Some(h) = running.channel_tasks.remove(&id) {
                h.stop();
                if let Err(e) = h.join.await {
                    warn!(channel_id = %id, error = %e, "reload: join error");
                }
            }
        }

        // Start tasks for enabled rows that don't have one yet.
        for row in rows.into_iter().filter(|r| r.enabled) {
            if running.channel_tasks.contains_key(&row.id) {
                continue;
            }

            let channel_type = row.channel.to_db_str().to_string();
            let Some(factory) = self.registry.get(&channel_type) else {
                warn!(
                    channel_id = %row.id,
                    channel_type = %channel_type,
                    "no factory registered for channel type; skipping"
                );
                continue;
            };

            // build() and connect() are both async — no lock held.
            let mut adapter = factory.build(&row).await?;
            adapter.connect(&row).await?;

            let handle = spawn_channel_task(
                row.clone(),
                adapter,
                Arc::clone(&self.store),
                running.ingest_tx.clone(),
                self.bus.clone(),
                &running.root,
            );

            running.channel_tasks.insert(row.id, handle);
        }

        Ok(())
    }

    /// Graceful shutdown (consumes `self`).
    ///
    /// Order: cancel root token → join channel tasks → drop ingest sender →
    /// join ingestor → join classifier. Each join is bounded by `shutdown_timeout`.
    pub async fn shutdown(mut self) {
        let Some(mut running) = self.running.take() else {
            return;
        };

        running.root.cancel();

        // Channel tasks call disconnect() internally on their way out.
        let channel_ids: Vec<Uuid> = running.channel_tasks.keys().copied().collect();
        for id in channel_ids {
            if let Some(h) = running.channel_tasks.remove(&id) {
                if let Err(e) = timeout(self.shutdown_timeout, h.join).await {
                    warn!(channel_id = %id, error = %e, "shutdown: channel task timeout");
                }
            }
        }

        // Dropping the sender closes the ingestor's receiver side.
        drop(running.ingest_tx);
        if let Err(e) = timeout(self.shutdown_timeout, running.ingest_handle).await {
            warn!(error = %e, "shutdown: ingestor timeout");
        }

        if let Some(h) = running.classifier_handle {
            if let Err(e) = timeout(self.shutdown_timeout, h).await {
                warn!(error = %e, "shutdown: classifier timeout");
            }
        }

        info!("runtime shut down");
    }
}
