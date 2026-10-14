use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

uuid_id!(
    /// Tenant identifier. Knowing a `TenantId` grants nothing: authority comes
    /// only from an [`OpCtx`](crate::OpCtx).
    TenantId
);

uuid_id!(
    /// Identifier of the acting principal (user, service account or system worker).
    ActorId
);

uuid_id!(
    /// Correlation id shared by every log line, span, outbox event and
    /// downstream message caused by one inbound request.
    CorrelationId
);

impl ActorId {
    /// The actor recorded for system-initiated operations (event consumers,
    /// scheduled jobs).
    pub const SYSTEM: Self = Self(Uuid::nil());
}

impl CorrelationId {
    /// A fresh time-ordered correlation id.
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }
}
