use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;

/// HTTP server settings, embedded in each binary's config.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    #[serde(default = "defaults::bind")]
    pub bind: SocketAddr,
    /// Whole-request deadline; exceeded requests get 503 `REQUEST_TIMEOUT`.
    #[serde(default = "defaults::request_timeout_ms")]
    pub request_timeout_ms: u64,
    /// Maximum request body size.
    #[serde(default = "defaults::body_limit_bytes")]
    pub body_limit_bytes: usize,
    /// In-flight request cap; beyond it requests are shed with 503 instead of
    /// queueing (protects the connection pool and tail latency).
    #[serde(default = "defaults::max_concurrent_requests")]
    pub max_concurrent_requests: usize,
    /// After SIGTERM, keep serving while reporting not-ready for this long so
    /// load balancers stop routing to this instance before it stops accepting.
    #[serde(default = "defaults::drain_delay_ms")]
    pub drain_delay_ms: u64,
    /// Upper bound for in-flight requests to finish after draining starts.
    /// Keep `drain_delay + shutdown_timeout` below the pod's
    /// `terminationGracePeriodSeconds`.
    #[serde(default = "defaults::shutdown_timeout_ms")]
    pub shutdown_timeout_ms: u64,
    /// Browser origins allowed by CORS. Empty disables the CORS layer.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
    /// Serve `/docs` (interactive API reference). `/openapi.json` is always served.
    #[serde(default)]
    pub docs_ui: bool,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind: defaults::bind(),
            request_timeout_ms: defaults::request_timeout_ms(),
            body_limit_bytes: defaults::body_limit_bytes(),
            max_concurrent_requests: defaults::max_concurrent_requests(),
            drain_delay_ms: defaults::drain_delay_ms(),
            shutdown_timeout_ms: defaults::shutdown_timeout_ms(),
            cors_allowed_origins: Vec::new(),
            docs_ui: false,
        }
    }
}

impl HttpConfig {
    pub(crate) fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms)
    }

    pub(crate) fn drain_delay(&self) -> Duration {
        Duration::from_millis(self.drain_delay_ms)
    }

    pub(crate) fn shutdown_timeout(&self) -> Duration {
        Duration::from_millis(self.shutdown_timeout_ms)
    }
}

mod defaults {
    use std::net::{Ipv4Addr, SocketAddr};

    pub(super) fn bind() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080))
    }
    pub(super) const fn request_timeout_ms() -> u64 {
        10_000
    }
    pub(super) const fn body_limit_bytes() -> usize {
        1024 * 1024
    }
    pub(super) const fn max_concurrent_requests() -> usize {
        512
    }
    pub(super) const fn drain_delay_ms() -> u64 {
        5_000
    }
    pub(super) const fn shutdown_timeout_ms() -> u64 {
        20_000
    }
}
