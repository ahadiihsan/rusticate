use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{HttpConfig, Readiness};

/// A token cancelled on SIGTERM or Ctrl-C. Share it with every background
/// task so the whole process drains together.
pub fn shutdown_signal() -> CancellationToken {
    let token = CancellationToken::new();
    let trigger = token.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received");
        trigger.cancel();
    });
    token
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "cannot install SIGTERM handler; using Ctrl-C only");
            ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    ctrl_c().await;
}

async fn ctrl_c() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "cannot listen for Ctrl-C; shutdown only via SIGTERM");
        std::future::pending::<()>().await;
    }
}

/// Serves `app` until `shutdown`, then drains (OPS-003):
///
/// 1. readiness flips to 503, but the listener keeps accepting for
///    `drain_delay` while load balancers deregister the instance;
/// 2. the listener closes and in-flight requests finish;
/// 3. after `shutdown_timeout` the server stops waiting.
pub async fn serve(
    app: Router,
    cfg: &HttpConfig,
    readiness: Readiness,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    tracing::info!(address = %cfg.bind, "listening");

    let drain_delay = cfg.drain_delay();
    let draining = {
        let shutdown = shutdown.clone();
        async move {
            shutdown.cancelled().await;
            readiness.stop_accepting();
            tracing::info!(?drain_delay, "draining: readiness is now 503");
            tokio::time::sleep(drain_delay).await;
        }
    };
    let server = axum::serve(listener, app).with_graceful_shutdown(draining);

    let deadline = {
        let shutdown = shutdown.clone();
        let total = drain_delay.saturating_add(cfg.shutdown_timeout());
        async move {
            shutdown.cancelled().await;
            tokio::time::sleep(total).await;
        }
    };

    tokio::select! {
        result = server => result,
        () = deadline => {
            tracing::warn!("shutdown timeout reached; abandoning in-flight requests");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn serve_flips_readiness_drains_and_returns_after_shutdown() {
        let cfg = HttpConfig {
            bind: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            drain_delay_ms: 20,
            shutdown_timeout_ms: 200,
            ..HttpConfig::default()
        };
        let readiness = Readiness::new(Vec::new());
        let shutdown = shutdown_signal();
        let server = tokio::spawn({
            let (readiness, shutdown) = (readiness.clone(), shutdown.clone());
            async move { serve(Router::new(), &cfg, readiness, shutdown).await }
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(readiness.is_accepting());
        shutdown.cancel();

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server stops within the configured bounds")
            .unwrap()
            .unwrap();
        assert!(!readiness.is_accepting());
    }
}
