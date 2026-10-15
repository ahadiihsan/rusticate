use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use api_kit::{CheckFuture, ReadinessCheck};
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Liveness and readiness probes (OPS-002).
///
/// * `/health/live`: the process is up. Never checks dependencies (a database
///   outage must not make Kubernetes restart every pod).
/// * `/health/ready`: accepting traffic? False once shutdown begins, or when a
///   dependency check fails.
#[derive(Clone)]
pub struct Readiness {
    accepting: Arc<AtomicBool>,
    checks: Arc<[Arc<dyn ReadinessCheck>]>,
}

impl std::fmt::Debug for Readiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Readiness")
            .field("accepting", &self.is_accepting())
            .finish_non_exhaustive()
    }
}

impl Readiness {
    pub fn new(checks: Vec<Arc<dyn ReadinessCheck>>) -> Self {
        Self {
            accepting: Arc::new(AtomicBool::new(true)),
            checks: checks.into(),
        }
    }

    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::SeqCst)
    }

    /// Called when draining starts.
    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::SeqCst);
    }

    /// `/health/live` and `/health/ready`.
    pub fn router(&self) -> Router {
        let readiness = self.clone();
        Router::new()
            .route("/health/live", get(|| async { (StatusCode::OK, "live") }))
            .route(
                "/health/ready",
                get(move || {
                    let readiness = readiness.clone();
                    async move { readiness.probe().await }
                }),
            )
    }

    async fn probe(&self) -> Response {
        if !self.is_accepting() {
            return (StatusCode::SERVICE_UNAVAILABLE, "draining").into_response();
        }
        for check in self.checks.iter() {
            let outcome = tokio::time::timeout(CHECK_TIMEOUT, check.check()).await;
            let failure = match outcome {
                Ok(Ok(())) => continue,
                Ok(Err(err)) => err,
                Err(_) => "timed out".to_owned(),
            };
            tracing::warn!(check = check.name(), error = %failure, "readiness check failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("{} unavailable", check.name()),
            )
                .into_response();
        }
        (StatusCode::OK, "ready").into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use tower::ServiceExt as _;

    use super::*;

    struct Fails;

    impl ReadinessCheck for Fails {
        fn name(&self) -> &'static str {
            "broker"
        }

        fn check(&self) -> CheckFuture<'_> {
            Box::pin(async { Err("down".to_owned()) })
        }
    }

    async fn status(readiness: &Readiness, uri: &str) -> StatusCode {
        let request = http::Request::get(uri).body(Body::empty()).unwrap();
        readiness.router().oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn readiness_follows_draining_and_checks_but_liveness_does_not() {
        let healthy = Readiness::new(Vec::new());
        let broken = Readiness::new(vec![Arc::new(Fails)]);

        assert_eq!(status(&healthy, "/health/ready").await, StatusCode::OK);
        assert_eq!(
            status(&broken, "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(&broken, "/health/live").await, StatusCode::OK);

        healthy.stop_accepting();
        assert_eq!(
            status(&healthy, "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(&healthy, "/health/live").await, StatusCode::OK);
    }
}
