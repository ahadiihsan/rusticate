//! Restricted capability: constructing an [`OpCtx`] (SEC-002).
//!
//! Only authentication edges may mint a context: the IAM middleware (after
//! verifying a token), worker envelopes (after reading a trusted event), and
//! tests. `cargo xtask arch` fails the build if any other crate references
//! `op_context::mint`.

use crate::{ActorId, ActorKind, CorrelationId, OpCtx, Permission, TenantId};

/// Context for a verified external principal.
pub fn authenticated(
    tenant_id: TenantId,
    actor_id: ActorId,
    actor_kind: ActorKind,
    correlation_id: CorrelationId,
    permissions: impl IntoIterator<Item = Box<str>>,
) -> OpCtx {
    OpCtx::new(tenant_id, actor_id, actor_kind, correlation_id, permissions)
}

/// Context for a system-initiated operation (event consumer, scheduled job),
/// scoped to one tenant and to exactly the permissions the job needs.
pub fn system(
    tenant_id: TenantId,
    correlation_id: CorrelationId,
    permissions: &[Permission],
) -> OpCtx {
    OpCtx::new(
        tenant_id,
        ActorId::SYSTEM,
        ActorKind::System,
        correlation_id,
        permissions.iter().map(|p| Box::from(p.as_str())),
    )
}
