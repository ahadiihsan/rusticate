//! Operation contexts for tests (feature `mint`): a fresh tenant, a user
//! holding exactly the permissions under test, a system context.

use uuid::Uuid;

use crate::mint;
use crate::{ActorId, ActorKind, CorrelationId, OpCtx, Permission, TenantId};

/// A fresh tenant.
pub fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::now_v7())
}

/// A user in `tenant` holding exactly `permissions`.
pub fn user(tenant: TenantId, permissions: &[Permission]) -> OpCtx {
    user_as(tenant, ActorId::from_uuid(Uuid::now_v7()), permissions)
}

/// A specific user in `tenant` holding exactly `permissions`.
pub fn user_as(tenant: TenantId, actor: ActorId, permissions: &[Permission]) -> OpCtx {
    mint::authenticated(
        tenant,
        actor,
        ActorKind::User,
        CorrelationId::new_v7(),
        permissions.iter().map(|p| Box::from(p.as_str())),
    )
}

/// A system context in `tenant` holding exactly `permissions`.
pub fn system(tenant: TenantId, permissions: &[Permission]) -> OpCtx {
    mint::system(tenant, CorrelationId::new_v7(), permissions)
}
