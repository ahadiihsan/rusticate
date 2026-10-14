//! Billing bounded context — **the reference implementation** of a feature
//! slice (SOP ARCH-003..ARCH-006). Copy its shape, not its business rules.
//!
//! One module per use case, named `<verb>_<noun>` in the ubiquitous language:
//!
//! ```text
//! <feature>.rs        HOP 2  workflow: permission → tx → [idempotency] → db/engine → outbox → commit
//! <feature>/http.rs   HOP 1  axum handler + request/response DTOs + OpenAPI (or handler.rs for events)
//! <feature>/db.rs     HOP 3  private SQL (`query!` macros), no business branching
//! billing-engine      HOP 3  pure rules (totals, rounding, lifecycle)
//! ```
//!
//! Public surface (ARCH-005): [`router`], [`MIGRATOR`], [`permissions`],
//! [`events`], workflow entry points, [`RecordInvoiceStats`] and the domain
//! types. Everything else is private to its feature.

mod dto;
mod error;
pub mod events;
pub mod permissions;
mod types;

// xtask:features
pub mod configure_billing;
pub mod get_billing_settings;
pub mod get_billing_stats;
pub mod get_invoice;
pub mod issue_invoice;
pub mod list_invoices;
pub mod record_invoice_stats;
pub mod void_invoice;

use axum::extract::FromRef;
use datastore::DataStore;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use error::BillingError;
pub use record_invoice_stats::RecordInvoiceStats;
pub use types::{
    BillingSettings, CurrencyStats, CustomerEmail, InvoiceId, InvoiceLineView, InvoiceSummary,
    InvoiceView, LineDescription, VoidReason,
};

/// Migrations for the `billing` schema. Run after `datastore::MIGRATOR`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// OpenAPI tag of every billing operation.
pub const TAG: &str = "billing";

/// HTTP routes of this context. Generic over the host's state so any product
/// binary can mount it (ARCH-007): it only needs a `DataStore` from the state.
pub fn router<S>() -> OpenApiRouter<S>
where
    S: Clone + Send + Sync + 'static,
    DataStore: FromRef<S>,
{
    OpenApiRouter::new()
        // xtask:routes
        .routes(routes!(get_billing_settings::http::handle, configure_billing::http::handle))
        .routes(routes!(get_billing_stats::http::handle))
        .routes(routes!(list_invoices::http::handle, issue_invoice::http::handle))
        .routes(routes!(get_invoice::http::handle))
        .routes(routes!(void_invoice::http::handle))
}
