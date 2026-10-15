//! The operation context (SOP SEC-001..SEC-004).
//!
//! An [`OpCtx`] states *who* is acting, *for which tenant*, *with which
//! permissions*, and *under which correlation id*. Every workflow takes
//! `&OpCtx` as its first parameter, and the datastore will not open a
//! transaction without one, so tenant scoping cannot be forgotten.
//!
//! `OpCtx` cannot be constructed outside this crate except through the `mint`
//! module (feature `mint`), a capability that `cargo rusticate arch` restricts to
//! authentication edges (IAM middleware, worker envelopes, tests).

mod context;
#[cfg(feature = "axum")]
mod extract;
mod ids;
#[cfg(feature = "mint")]
pub mod mint;
mod permission;

pub use context::{ActorKind, AuthzError, OpCtx};
pub use ids::{ActorId, CorrelationId, TenantId};
pub use permission::Permission;
