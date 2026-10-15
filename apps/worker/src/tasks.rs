//! The worker's long-running tasks. Every task stops when `shutdown` is
//! cancelled; an in-flight batch or message always completes first.

use std::num::NonZeroUsize;
use std::time::Duration;

use async_nats::jetstream::Context;
use async_nats::jetstream::stream::Stream;
use datastore::{DataStore, SystemStore};
use messaging::nats::{JetStreamSink, Topology, run_consumer};
use messaging::{EventHandler, OutboxRelay, RelayConfig, maintenance};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Everything the tasks need.
#[derive(Debug, Clone)]
pub struct Deps {
    pub app: DataStore,
    pub system: SystemStore,
    pub js: Context,
    pub stream: Stream,
    pub topology: Topology,
    pub concurrency: NonZeroUsize,
    pub maintenance_interval: Duration,
}

/// Spawns the relay, every event consumer, and maintenance.
///
/// Registering a new consumer = one `spawn_consumer` line (MSG-004).
pub fn spawn_all(deps: Deps, shutdown: &CancellationToken) -> JoinSet<anyhow::Result<()>> {
    let mut tasks = JoinSet::new();

    let relay = OutboxRelay::new(
        deps.system.clone(),
        JetStreamSink::new(deps.js.clone(), deps.topology.clone()),
        RelayConfig::default(),
    );
    tasks.spawn(relay_task(relay, shutdown.clone()));

    // Event consumers (MSG-004): one line per handler.
    // xtask:consumers
    // rusticate:module(example):start
    spawn_consumer(&mut tasks, &deps, shutdown, billing::RecordInvoiceStats);
    // rusticate:module(example):end

    tasks.spawn(maintenance_task(
        deps.system,
        deps.maintenance_interval,
        shutdown.clone(),
    ));
    tasks
}

/// Runs one event handler as a durable `JetStream` consumer (MSG-004). Called once
/// per handler from [`spawn_all`].
pub fn spawn_consumer<H: EventHandler + 'static>(
    tasks: &mut JoinSet<anyhow::Result<()>>,
    deps: &Deps,
    shutdown: &CancellationToken,
    handler: H,
) {
    let deps = deps.clone();
    let shutdown = shutdown.clone();
    tasks.spawn(async move {
        run_consumer(
            &deps.stream,
            &deps.topology,
            deps.app.clone(),
            handler,
            deps.concurrency,
            shutdown,
        )
        .await?;
        Ok(())
    });
}

async fn relay_task(
    relay: OutboxRelay<JetStreamSink>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    relay.run(shutdown).await;
    Ok(())
}

async fn maintenance_task(
    store: SystemStore,
    interval: Duration,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    maintenance::run(store, interval, shutdown).await;
    Ok(())
}
