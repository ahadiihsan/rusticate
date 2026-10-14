//! Shared fixtures for the billing integration tests.

use billing::configure_billing::{ConfigureBillingCmd, Expectation, configure_billing};
use billing::issue_invoice::{IssueInvoiceCmd, NewLine, issue_invoice};
use billing::permissions::{
    INVOICE_ISSUE, INVOICE_READ, INVOICE_VOID, SETTINGS_MANAGE, SETTINGS_READ,
};
use billing::{CustomerEmail, InvoiceView, LineDescription};
use billing_engine::invoice::Quantity;
use billing_engine::money::{Bps, Currency};
use idempotency::IdempotencyKey;
use op_context::{OpCtx, TenantId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx};

pub(crate) async fn db(pool: PgPoolOptions, conn: PgConnectOptions) -> TestDb {
    TestDb::new(pool, conn, &[&billing::MIGRATOR]).await
}

/// A user holding every billing permission.
pub(crate) fn admin(tenant: TenantId) -> OpCtx {
    ctx::user(
        tenant,
        &[
            SETTINGS_READ,
            SETTINGS_MANAGE,
            INVOICE_ISSUE,
            INVOICE_READ,
            INVOICE_VOID,
        ],
    )
}

/// Configures IDR with 11 % tax.
pub(crate) async fn configure(db: &TestDb, user: &OpCtx) {
    configure_billing(
        user,
        &db.app,
        ConfigureBillingCmd {
            currency: Currency::Idr,
            tax_rate: Bps::new(1_100).unwrap(),
            expectation: Expectation::Create,
        },
    )
    .await
    .unwrap();
}

pub(crate) fn invoice_cmd(unit_price_minor: i64, quantity: u32) -> IssueInvoiceCmd {
    IssueInvoiceCmd {
        customer_email: CustomerEmail::parse("ap@customer.example").unwrap(),
        lines: vec![NewLine {
            description: LineDescription::parse("Consulting").unwrap(),
            quantity: Quantity::new(quantity).unwrap(),
            unit_price_minor,
        }],
    }
}

pub(crate) fn key(raw: &str) -> IdempotencyKey {
    IdempotencyKey::parse(raw).unwrap()
}

pub(crate) async fn issue(
    db: &TestDb,
    user: &OpCtx,
    idempotency_key: &str,
    unit_price_minor: i64,
) -> InvoiceView {
    issue_invoice(
        user,
        &db.app,
        &key(idempotency_key),
        invoice_cmd(unit_price_minor, 1),
    )
    .await
    .unwrap()
}

pub(crate) async fn count(db: &TestDb, sql: &'static str) -> i64 {
    let mut tx = db.system.begin().await.unwrap();
    sqlx::query_scalar(sql).fetch_one(tx.conn()).await.unwrap()
}
