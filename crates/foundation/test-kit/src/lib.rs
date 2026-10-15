//! Test fixtures (SOP TEST-002..TEST-004). **Dev-dependency only**: `cargo rusticate
//! arch` fails if any crate lists it under `[dependencies]`.
//!
//! * [`TestDb`]: a fresh database per test, migrated, with handles that run as
//!   the *real* runtime roles (`app_rw`, `app_system`) so RLS behaves exactly
//!   as in production. Never test tenant isolation as a superuser.
//! * [`ctx`]: contexts for tests.
//! * [`schema`]: the catalog-level guard every product runs once
//!   (`assert_schema_invariants`).
#![expect(
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "fixtures fail the test loudly instead of returning errors"
)]

pub mod ctx;
mod db;
pub mod schema;

pub use db::TestDb;
