//! Startup verification of the connected database role (DB-002).
//!
//! RLS is silently disabled for superusers, for roles with BYPASSRLS, and for
//! table owners (unless FORCE is set). A misconfigured connection string would
//! therefore turn tenant isolation off without any error. Each store checks its
//! role once at startup and refuses to run with the wrong posture.

use sqlx::PgPool;

use crate::DataStoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Posture {
    /// Subject to RLS: the application role.
    RlsEnforced,
    /// Bypasses RLS, limited by GRANTs: the system role.
    System,
}

pub(crate) async fn verify(pool: &PgPool, expected: Posture) -> Result<(), DataStoreError> {
    let role = sqlx::query!(
        r#"
        SELECT
            current_user::text AS "name!",
            r.rolsuper AS "is_superuser!",
            r.rolbypassrls AS "bypasses_rls!",
            EXISTS (
                SELECT 1
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE c.relowner = r.oid
                  AND c.relkind IN ('r', 'p')
                  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
                  AND n.nspname NOT LIKE 'pg_toast%'
            ) AS "owns_tables!"
        FROM pg_roles r
        WHERE r.rolname = current_user
        "#
    )
    .fetch_one(pool)
    .await?;

    let problem = if role.is_superuser {
        Some("is a superuser (bypasses row-level security)")
    } else if role.owns_tables {
        Some("owns tables (runtime roles must not own schema objects)")
    } else {
        match (expected, role.bypasses_rls) {
            (Posture::RlsEnforced, true) => Some("has BYPASSRLS but this store must be RLS-bound"),
            (Posture::System, false) => Some("lacks BYPASSRLS required by the system store"),
            _ => None,
        }
    };

    match problem {
        Some(reason) => Err(DataStoreError::RoleMisconfigured(format!(
            "role `{}` {reason}",
            role.name
        ))),
        None => Ok(()),
    }
}
