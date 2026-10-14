use std::fmt;
use std::sync::Arc;

use crate::{ActorId, CorrelationId, Permission, TenantId};

/// Kind of principal behind an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    User,
    ServiceAccount,
    System,
}

impl ActorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ServiceAccount => "service_account",
            Self::System => "system",
        }
    }
}

/// Authorization failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthzError {
    #[error("missing permission `{required}`")]
    MissingPermission { required: Permission },
}

/// Unforgeable operation context. Cheap to clone (one `Arc`).
#[derive(Clone)]
pub struct OpCtx(Arc<Inner>);

struct Inner {
    tenant_id: TenantId,
    actor_id: ActorId,
    actor_kind: ActorKind,
    correlation_id: CorrelationId,
    /// Sorted and deduplicated, so membership is a binary search.
    permissions: Box<[Box<str>]>,
}

impl OpCtx {
    #[cfg_attr(
        not(any(feature = "mint", test)),
        expect(dead_code, reason = "only the `mint` capability constructs contexts")
    )]
    pub(crate) fn new(
        tenant_id: TenantId,
        actor_id: ActorId,
        actor_kind: ActorKind,
        correlation_id: CorrelationId,
        permissions: impl IntoIterator<Item = Box<str>>,
    ) -> Self {
        let mut permissions: Vec<Box<str>> = permissions.into_iter().collect();
        permissions.sort_unstable();
        permissions.dedup();
        Self(Arc::new(Inner {
            tenant_id,
            actor_id,
            actor_kind,
            correlation_id,
            permissions: permissions.into_boxed_slice(),
        }))
    }

    pub fn tenant_id(&self) -> TenantId {
        self.0.tenant_id
    }

    pub fn actor_id(&self) -> ActorId {
        self.0.actor_id
    }

    pub fn actor_kind(&self) -> ActorKind {
        self.0.actor_kind
    }

    pub fn correlation_id(&self) -> CorrelationId {
        self.0.correlation_id
    }

    pub fn has(&self, permission: Permission) -> bool {
        self.0
            .permissions
            .binary_search_by(|granted| granted.as_ref().cmp(permission.as_str()))
            .is_ok()
    }

    /// Capability guard: the first statement of every workflow (SEC-003).
    pub fn require(&self, permission: Permission) -> Result<(), AuthzError> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(AuthzError::MissingPermission {
                required: permission,
            })
        }
    }
}

/// Omits the permission list: it is noisy in logs and leaks the grant model.
impl fmt::Debug for OpCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpCtx")
            .field("tenant_id", &self.0.tenant_id)
            .field("actor_id", &self.0.actor_id)
            .field("actor_kind", &self.0.actor_kind)
            .field("correlation_id", &self.0.correlation_id)
            .field("permission_count", &self.0.permissions.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    const READ: Permission = Permission::new("demo.thing.read");
    const WRITE: Permission = Permission::new("demo.thing.write");

    fn ctx(perms: &[&str]) -> OpCtx {
        OpCtx::new(
            TenantId::from_uuid(Uuid::from_u128(1)),
            ActorId::from_uuid(Uuid::from_u128(2)),
            ActorKind::User,
            CorrelationId::from_uuid(Uuid::from_u128(3)),
            perms.iter().map(|p| Box::from(*p)),
        )
    }

    #[test]
    fn require_grants_only_listed_permissions() {
        let ctx = ctx(&["demo.thing.read", "demo.thing.read"]);

        assert_eq!(ctx.require(READ), Ok(()));
        assert_eq!(
            ctx.require(WRITE),
            Err(AuthzError::MissingPermission { required: WRITE })
        );
    }

    #[test]
    fn there_is_no_implicit_admin_permission() {
        let ctx = ctx(&["tenant.admin", "*"]);

        assert!(!ctx.has(WRITE));
    }

    #[test]
    fn debug_hides_permission_names() {
        let rendered = format!("{:?}", ctx(&["demo.thing.read"]));

        assert!(!rendered.contains("demo.thing.read"));
        assert!(rendered.contains("permission_count: 1"));
    }
}
