use api_kit::ApiError;

/// Why a transaction lost a concurrency race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    SerializationFailure,
    Deadlock,
}

impl std::fmt::Display for ConflictKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SerializationFailure => "serialization failure",
            Self::Deadlock => "deadlock",
        })
    }
}

/// Database failures, classified by SQLSTATE so callers can react precisely
/// (DB-009). Domain code maps the constraint-specific variants (e.g. a unique
/// violation on `invoices_number_key`) to domain errors; everything else
/// converts into [`ApiError`] with the right status.
#[derive(Debug, thiserror::Error)]
pub enum DataStoreError {
    #[error("unique constraint `{constraint}` violated")]
    UniqueViolation { constraint: String },
    #[error("foreign key constraint `{constraint}` violated")]
    ForeignKeyViolation { constraint: String },
    #[error("check constraint `{constraint}` violated")]
    CheckViolation { constraint: String },
    #[error("transaction aborted by a concurrent transaction ({0}); safe to retry")]
    Conflict(ConflictKind),
    #[error("timed out waiting for a row lock")]
    LockTimeout,
    #[error("statement exceeded statement_timeout")]
    StatementTimeout,
    #[error("timed out waiting for a database connection")]
    PoolTimeout,
    #[error("database connection unavailable")]
    Unavailable(#[source] sqlx::Error),
    #[error("row-level security rejected the statement")]
    RlsViolation(#[source] sqlx::Error),
    #[error("database role misconfigured: {0}")]
    RoleMisconfigured(String),
    #[error("JSON encoding failed")]
    Encoding(#[from] serde_json::Error),
    #[error("event `{event_type}` is {bytes} bytes; the limit is 64 KiB")]
    EventTooLarge {
        event_type: &'static str,
        bytes: usize,
    },
    #[error("database error")]
    Database(#[source] sqlx::Error),
}

impl DataStoreError {
    /// True when retrying the whole transaction may succeed.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Conflict(_)
                | Self::LockTimeout
                | Self::StatementTimeout
                | Self::PoolTimeout
                | Self::Unavailable(_)
        )
    }
}

impl From<sqlx::Error> for DataStoreError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::PoolTimedOut => return Self::PoolTimeout,
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::PoolClosed => {
                return Self::Unavailable(err);
            }
            _ => {}
        }
        let Some(db_err) = err.as_database_error() else {
            return Self::Database(err);
        };
        let constraint = || db_err.constraint().unwrap_or("<unknown>").to_owned();
        let code = db_err.code();
        if code.as_deref().is_some_and(is_unavailable_sqlstate) {
            return Self::Unavailable(err);
        }
        match code.as_deref() {
            Some("23505") => Self::UniqueViolation {
                constraint: constraint(),
            },
            Some("23503") => Self::ForeignKeyViolation {
                constraint: constraint(),
            },
            Some("23514") => Self::CheckViolation {
                constraint: constraint(),
            },
            Some("40001") => Self::Conflict(ConflictKind::SerializationFailure),
            Some("40P01") => Self::Conflict(ConflictKind::Deadlock),
            Some("55P03") => Self::LockTimeout,
            Some("57014") => Self::StatementTimeout,
            Some("42501") if db_err.message().contains("row-level security") => {
                Self::RlsViolation(err)
            }
            _ => Self::Database(err),
        }
    }
}

/// The server, not the statement, failed: retrying on a healthy connection
/// (after failover or restart) can succeed.
fn is_unavailable_sqlstate(code: &str) -> bool {
    code.starts_with("08") // connection_exception class
        || matches!(
            code,
            "57P01" // admin_shutdown
                | "57P02" // crash_shutdown
                | "57P03" // cannot_connect_now (starting up, failing over)
                | "25P03" // idle_in_transaction_session_timeout
                | "53300" // too_many_connections
        )
}

/// Default HTTP mapping. Transient failures become 503 + `Retry-After`;
/// constraint violations that validation should have caught are bugs and
/// become logged 500s.
impl From<DataStoreError> for ApiError {
    fn from(err: DataStoreError) -> Self {
        match err {
            DataStoreError::UniqueViolation { .. } => {
                Self::conflict("ALREADY_EXISTS", "The resource already exists.")
            }
            DataStoreError::ForeignKeyViolation { .. } => Self::unprocessable(
                "REFERENCED_RESOURCE_MISSING",
                "A referenced resource does not exist.",
            ),
            DataStoreError::Conflict(_) | DataStoreError::LockTimeout => Self::unavailable(
                "CONCURRENT_MODIFICATION",
                "The resource is being modified concurrently. Retry the request.",
            ),
            DataStoreError::StatementTimeout => Self::unavailable(
                "DATABASE_TIMEOUT",
                "The operation timed out. Retry the request.",
            ),
            DataStoreError::PoolTimeout | DataStoreError::Unavailable(_) => Self::unavailable(
                "DATABASE_UNAVAILABLE",
                "The service is temporarily overloaded. Retry the request.",
            ),
            other @ (DataStoreError::CheckViolation { .. }
            | DataStoreError::RlsViolation(_)
            | DataStoreError::RoleMisconfigured(_)
            | DataStoreError::Encoding(_)
            | DataStoreError::EventTooLarge { .. }
            | DataStoreError::Database(_)) => Self::internal(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_of(err: DataStoreError) -> u16 {
        ApiError::from(err).status().as_u16()
    }

    #[test]
    fn transient_failures_map_to_retryable_503() {
        for err in [
            DataStoreError::Conflict(ConflictKind::Deadlock),
            DataStoreError::Conflict(ConflictKind::SerializationFailure),
            DataStoreError::LockTimeout,
            DataStoreError::StatementTimeout,
            DataStoreError::PoolTimeout,
        ] {
            assert!(err.is_transient(), "{err} must be transient");
            assert_eq!(status_of(err), 503);
        }
    }

    #[test]
    fn constraint_violations_map_by_kind() {
        let unique = DataStoreError::UniqueViolation {
            constraint: "k".to_owned(),
        };
        let foreign = DataStoreError::ForeignKeyViolation {
            constraint: "k".to_owned(),
        };
        let check = DataStoreError::CheckViolation {
            constraint: "k".to_owned(),
        };

        assert_eq!(status_of(unique), 409);
        assert_eq!(status_of(foreign), 422);
        assert_eq!(
            status_of(check),
            500,
            "a check violation means validation is missing: a bug"
        );
    }

    #[test]
    fn misconfiguration_is_internal_and_not_transient() {
        let err = DataStoreError::RoleMisconfigured("role `x` is a superuser".to_owned());

        assert!(!err.is_transient());
        assert_eq!(status_of(err), 500);
    }

    #[test]
    fn failover_and_restart_sqlstates_are_transient() {
        for code in [
            "08006", "08001", "57P01", "57P02", "57P03", "25P03", "53300",
        ] {
            assert!(is_unavailable_sqlstate(code), "{code}");
        }
        for code in ["23505", "42501", "22P02"] {
            assert!(!is_unavailable_sqlstate(code), "{code}");
        }
    }

    #[test]
    fn pool_and_io_errors_are_classified_without_a_database() {
        assert!(matches!(
            DataStoreError::from(sqlx::Error::PoolTimedOut),
            DataStoreError::PoolTimeout
        ));
        assert!(matches!(
            DataStoreError::from(sqlx::Error::PoolClosed),
            DataStoreError::Unavailable(_)
        ));
        assert!(matches!(
            DataStoreError::from(sqlx::Error::RowNotFound),
            DataStoreError::Database(_)
        ));
    }
}
