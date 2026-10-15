//! Tenant-scoped PostgreSQL access (SOP DB-001..DB-011, MSG-001, MSG-004).
//!
//! * [`DataStore`]: the application's handle. Its only way to run SQL is
//!   [`DataStore::begin`] / [`DataStore::begin_read_only`], which demand an
//!   [`OpCtx`](op_context::OpCtx) and bind PostgreSQL row-level security to the
//!   caller's tenant for exactly one transaction.
//! * [`TenantTx`]: that transaction. Domain `db.rs` modules take
//!   `&mut TenantTx`, never a pool.
//! * [`SystemStore`] / [`SystemTx`]: cross-tenant access for the outbox relay
//!   and maintenance jobs only (restricted by `cargo rusticate arch`).
//! * Both stores verify their database role at startup and refuse to run with
//!   the wrong one (a superuser, an RLS-bypassing app role, a table owner).

mod config;
mod error;
mod inbox;
mod outbox;
mod posture;
mod store;
mod tx;

pub use config::DataStoreConfig;
pub use error::{ConflictKind, DataStoreError};
pub use outbox::{DomainEvent, MAX_EVENT_BYTES};
pub use store::{DataStore, SystemStore};
pub use tx::{SystemTx, TenantTx};

/// Migrations for the `platform` schema. Run before any domain migrator.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();
