//! The readiness contract between a service's dependencies and its
//! `/health/ready` probe (OPS-002). Defined here, below every crate that has a
//! dependency to report, so each implements it for its own types and the HTTP
//! runtime (`server-kit`) needs none of them.

use std::future::Future;
use std::pin::Pin;

/// The future a [`ReadinessCheck`] returns.
pub type CheckFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// A dependency that must be reachable for the instance to receive traffic.
pub trait ReadinessCheck: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn check(&self) -> CheckFuture<'_>;
}
