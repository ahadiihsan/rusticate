//! Billing integration tests (TEST-002). One binary per crate: it links once,
//! and every module shares the fixtures in `support`.
//!
//! Every test runs against a fresh, migrated database as the real `app_rw`
//! role, so row-level security is exercised exactly as in production.
#![cfg(test)]

mod configure_billing;
mod issue_invoice;
mod list_invoices;
mod record_invoice_stats;
mod support;
mod void_invoice;
// xtask:tests
