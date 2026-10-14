use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use datastore::{DataStore, SystemStore};
use messaging::nats::{Topology, ensure_stream};
use server_kit::Readiness;
use worker::config::Config;
use worker::health::NatsReadiness;
use worker::tasks::{self, Deps};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SERVICE_NAME: &str = "worker";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let _telemetry = telemetry::init(SERVICE_NAME, &config.telemetry)?;

    let app = DataStore::connect(&config.database, SERVICE_NAME)
        .await
        .context("connecting app_rw")?;
    let system = SystemStore::connect(&config.system_database, SERVICE_NAME)
        .await
        .context("connecting app_system")?;
    let nats = async_nats::connect(&config.nats.url)
        .await
        .context("connecting to NATS")?;
    let js = async_nats::jetstream::new(nats.clone());
    let topology = Topology::default();
    let stream = ensure_stream(&js, &topology, config.nats.stream_replicas).await?;

    let shutdown = server_kit::shutdown_signal();
    let readiness = Readiness::new(vec![
        Arc::new(app.clone()),
        Arc::new(system.clone()),
        Arc::new(NatsReadiness(nats.clone())),
    ]);
    let mut running = tasks::spawn_all(
        Deps {
            app: app.clone(),
            system: system.clone(),
            js,
            stream,
            topology,
            concurrency: config.consumers.concurrency,
            maintenance_interval: Duration::from_secs(config.consumers.maintenance_interval_secs),
        },
        &shutdown,
    );

    let probes = server_kit::with_middleware(readiness.router(), &config.http);
    let health = tokio::spawn({
        let http = config.http.clone();
        let shutdown = shutdown.clone();
        async move { server_kit::serve(probes, &http, readiness, shutdown).await }
    });

    // A task that exits on its own (not via shutdown) is fatal: stop everything
    // so the orchestrator restarts the pod instead of running half a worker.
    while let Some(finished) = running.join_next().await {
        if !shutdown.is_cancelled() {
            tracing::error!(result = ?finished, "worker task exited unexpectedly; shutting down");
            shutdown.cancel();
        }
    }
    health.await??;

    nats.drain().await.ok();
    app.close().await;
    system.close().await;
    tracing::info!("stopped");
    Ok(())
}
